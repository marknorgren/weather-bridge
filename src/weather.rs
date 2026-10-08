//! The weather service shared by REST, MCP and the CLI: resolves a query to a location,
//! then assembles reports, hourly forecasts and alerts from NWS documents.
mod convert;
mod normalize;
mod nws;
mod query;

pub use crate::model::{Location, Precision};
pub use nws::{CACHE_SECONDS, with_cache_lifetime};
pub use query::{GridPoint, Units, WeatherQuery};

use crate::{
    Error, ErrorCode,
    cities::Cities,
    diagnostics::{Metrics, Operation},
    model::{
        ActiveAlerts, AlertSources, CheckedAlertsStatus, Completeness, HourlyAlertsStatus,
        HourlyForecast, HourlySources, Observation, QuerySource, ReportSources, Warning,
        WeatherReport,
    },
};
use chrono::Utc;
use normalize::{alert_list, issued_source, nearest_stations, observation, periods};
use nws::{
    UpstreamPath,
    documents::{AlertFeature, Alerts, Forecast, PointsProperties, Stations},
    required,
};
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

/// Weather lookups allowed at once; more are refused as `BUSY`. Cached lookups need no
/// upstream requests, so this bounds concurrency, not upstream load. Upstream pacing
/// serves about one uncached report every six seconds, so uncached lookups beyond that are
/// refused as `BUSY` by the pacing wait window ([`nws::UPSTREAM_WAIT_WINDOW`]) instead.
const MAX_LOOKUPS_IN_FLIGHT: usize = 8;
/// Deadline for one whole weather lookup, across all its upstream requests.
pub(crate) const LOOKUP_DEADLINE: Duration = Duration::from_secs(45);

/// Time allowed for the whole observation search, across all stations tried. Observations
/// are optional, so when it runs out the report keeps the best observation found so far, or
/// none with [`Warning::NoObservation`], rather than missing [`LOOKUP_DEADLINE`].
const OBSERVATION_BUDGET: Duration = nws::UPSTREAM_REQUEST_TIMEOUT;
// A report runs the grid lookup (alongside alerts), then the forecasts and station list
// together, then the observation search. Waits for pacing all end within the wait window
// of the lookup's start, so the worst case is that window, two upstream requests and the
// observation budget. It must fit inside the deadline, so that slow sources fail or
// degrade with a typed result rather than time out the whole lookup.
const _: () = assert!(
    nws::UPSTREAM_WAIT_WINDOW.as_millis()
        + 2 * nws::UPSTREAM_REQUEST_TIMEOUT.as_millis()
        + OBSERVATION_BUDGET.as_millis()
        < LOOKUP_DEADLINE.as_millis()
);

/// The public NWS API origin.
pub const NWS_BASE_URL: &str = "https://api.weather.gov";

/// Runtime configuration parsed once by the entry point; nothing here reads the environment.
#[derive(Clone, Debug)]
pub struct Config {
    /// Contact User-Agent sent to NWS. `None` uses a generic demo identifier.
    pub user_agent: Option<String>,
    /// NWS API origin (`http` or `https`, no path or query). Every request goes here, links
    /// discovered in NWS documents must be on this origin, and response sources name it.
    pub nws_base_url: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            user_agent: None,
            nws_base_url: NWS_BASE_URL.into(),
        }
    }
}

#[derive(Clone)]
pub struct Weather {
    cities: Cities,
    nws: nws::Client,
    admission: Arc<Semaphore>,
    metrics: Arc<Metrics>,
}
impl Weather {
    pub fn new() -> anyhow::Result<Self> {
        Self::configured(Config::default())
    }
    /// Fails when the configured NWS base URL is not a plain `http(s)` origin.
    pub fn configured(config: Config) -> anyhow::Result<Self> {
        let metrics = Arc::new(Metrics::default());
        Ok(Self {
            cities: Cities::load()?,
            nws: nws::Client::new(&config.nws_base_url, config.user_agent, metrics.clone())?,
            admission: Arc::new(Semaphore::new(MAX_LOOKUPS_IN_FLIGHT)),
            metrics,
        })
    }
    /// Up to ten cities whose name matches `query` exactly or by prefix, exact matches
    /// first. Uses only the embedded index, with no network requests. Queries outside
    /// the advertised trimmed-length bounds are `INVALID_LOCATION`.
    pub fn search_cities(&self, query: &str) -> Result<Vec<crate::cities::City>, Error> {
        self.cities.search(query)
    }
    /// Full report: observations, forecast, hourly periods and alerts.
    /// May fetch NWS documents; cached sources are reused within their freshness limits.
    /// The lookup has a 45-second deadline and may fail with `BUSY` before it starts.
    pub async fn report(&self, q: WeatherQuery) -> Result<WeatherReport, Error> {
        let location = query::resolve(&self.cities, &q)?;
        self.guarded(Operation::Weather, self.report_inner(location, q.units))
            .await
    }
    /// Hourly periods only: needs the grid lookup and the hourly forecast, nothing else.
    /// May fetch NWS documents or reuse fresh cached sources. The lookup has a
    /// 45-second deadline and may fail with `BUSY` before it starts.
    pub async fn hourly(&self, q: WeatherQuery) -> Result<HourlyForecast, Error> {
        let location = query::resolve(&self.cities, &q)?;
        self.guarded(Operation::Hourly, self.hourly_inner(location, q.units))
            .await
    }
    /// Active alerts only. Independent of /points and forecasts, so a forecast outage does
    /// not hide alerts. A failed check is `alertsStatus: "unavailable"`, never an all-clear.
    /// May fetch NWS documents or reuse fresh cached sources. The lookup has a
    /// 45-second deadline and may fail with `BUSY` before it starts.
    pub async fn alerts(&self, q: WeatherQuery) -> Result<ActiveAlerts, Error> {
        let location = query::resolve(&self.cities, &q)?;
        self.guarded(Operation::Alerts, self.alerts_inner(location, q.units))
            .await
    }
    /// Admission control, the overall deadline and partial-report counting shared by every
    /// weather operation.
    async fn guarded<T: Completeness>(
        &self,
        operation: Operation,
        work: impl Future<Output = Result<T, Error>>,
    ) -> Result<T, Error> {
        let _permit = self.admission.try_acquire().map_err(|_| {
            self.metrics.busy();
            busy()
        })?;
        let result = tokio::time::timeout(LOOKUP_DEADLINE, nws::lookup(work))
            .await
            .map_err(|_| {
                Error::new(
                    ErrorCode::UpstreamTimeout,
                    "Weather sources took too long to respond. Retry shortly.",
                )
            })?;
        match &result {
            Ok(report) if !report.is_complete() => self.metrics.partial(operation),
            // Upstream pacing refused a request this lookup could not do without.
            Err(error) if error.code == ErrorCode::Busy => self.metrics.busy(),
            _ => {}
        }
        result
    }
    pub(crate) fn metrics(&self) -> Arc<Metrics> {
        self.metrics.clone()
    }
    async fn report_inner(
        &self,
        mut location: Location,
        units: Units,
    ) -> Result<WeatherReport, Error> {
        let alerts_path = alerts_path(&location);
        // Alerts do not depend on the grid lookup, so they start alongside it.
        let (point, alerts) = tokio::join!(
            self.nws.point(location.grid_lookup_point),
            self.active_alerts(&alerts_path)
        );
        let grid = point?;
        name_from_point(&mut location, &grid.properties);
        let forecast_link = grid.forecast()?;
        let hourly_link = grid.hourly();
        let stations_link = grid.stations();
        let (forecast, hourly, stations) = tokio::join!(
            self.nws.fetch::<Forecast>(forecast_link.path()),
            async {
                match &hourly_link {
                    Ok(link) => self.nws.fetch::<Forecast>(link.path()).await,
                    Err(error) => Err(error.clone()),
                }
            },
            async {
                match &stations_link {
                    Ok(link) => self.nws.fetch::<Stations>(link.path()).await,
                    Err(error) => Err(error.clone()),
                }
            }
        );
        let forecast = forecast?;
        let mut warnings = Vec::new();
        let hourly = partial(hourly, Warning::HourlyUnavailable, &mut warnings);
        let alerts = partial(alerts, Warning::AlertsUnavailable, &mut warnings);
        let stations = partial(stations, Warning::StationsUnavailable, &mut warnings);
        let observation = match &stations {
            Some(stations) => self.observation(stations, &location, units).await,
            None => None,
        };
        if observation.is_none() {
            warnings.push(Warning::NoObservation);
        }
        let daily = required(
            periods(&forecast, 14, units),
            forecast_link.path().as_str(),
            "periods",
        )?;
        let hours = hourly
            .as_ref()
            .and_then(|hourly| periods(hourly, 24, units))
            .unwrap_or_default();
        let summary = daily
            .first()
            .and_then(|p| p.detail.as_deref())
            .unwrap_or("Forecast details unavailable.")
            .to_owned();
        Ok(WeatherReport {
            location,
            units,
            summary,
            current: observation,
            forecast: daily,
            hourly: hours,
            alerts: alert_list(alerts.as_deref().unwrap_or_default()),
            alerts_status: alerts_status(&alerts),
            warnings,
            sources: ReportSources {
                forecast: issued_source(forecast_link.url().to_owned(), Some(&forecast)),
                hourly: hourly_link
                    .ok()
                    .map(|link| issued_source(link.url().to_owned(), hourly.as_ref())),
                alerts: self.query_source(&alerts_path),
            },
            assembled_at: Utc::now().to_rfc3339(),
            cache_max_age_seconds: CACHE_SECONDS,
        })
    }
    async fn hourly_inner(
        &self,
        mut location: Location,
        units: Units,
    ) -> Result<HourlyForecast, Error> {
        let grid = self.nws.point(location.grid_lookup_point).await?;
        name_from_point(&mut location, &grid.properties);
        let hourly_link = grid.hourly()?;
        let hourly = self.nws.fetch::<Forecast>(hourly_link.path()).await?;
        let hours = required(
            periods(&hourly, 24, units),
            hourly_link.path().as_str(),
            "periods",
        )?;
        Ok(HourlyForecast {
            location,
            units,
            hourly: hours,
            alerts_status: HourlyAlertsStatus::NotChecked,
            sources: HourlySources {
                hourly: issued_source(hourly_link.url().to_owned(), Some(&hourly)),
            },
            warnings: vec![Warning::AlertsNotChecked],
            assembled_at: Utc::now().to_rfc3339(),
            cache_max_age_seconds: CACHE_SECONDS,
        })
    }
    async fn alerts_inner(
        &self,
        mut location: Location,
        units: Units,
    ) -> Result<ActiveAlerts, Error> {
        // Name from a cached grid lookup when one exists; never spend an upstream call on it.
        if let Some(point) = self.nws.cached_point(location.grid_lookup_point).await {
            name_from_point(&mut location, &point.properties);
        }
        let alerts_path = alerts_path(&location);
        let mut warnings = Vec::new();
        let alerts = match self.active_alerts(&alerts_path).await {
            // Alerts are the only source here, so pacing refusal is BUSY, not a failed check.
            Err(e) if matches!(e.code, ErrorCode::OutsideCoverage | ErrorCode::Busy) => {
                return Err(e);
            }
            result => partial(result, Warning::AlertsUnavailable, &mut warnings),
        };
        Ok(ActiveAlerts {
            location,
            units,
            alerts: alert_list(alerts.as_deref().unwrap_or_default()),
            alerts_status: alerts_status(&alerts),
            sources: AlertSources {
                alerts: self.query_source(&alerts_path),
            },
            warnings,
            assembled_at: Utc::now().to_rfc3339(),
            cache_max_age_seconds: CACHE_SECONDS,
        })
    }
    /// The latest observation from the nearest of up to three stations. A station whose
    /// observation has no temperature is kept only until a later one has a temperature.
    /// The search stops at [`OBSERVATION_BUDGET`] with whatever it has found.
    async fn observation(
        &self,
        stations: &Stations,
        location: &Location,
        units: Units,
    ) -> Option<Observation> {
        let deadline = tokio::time::Instant::now() + OBSERVATION_BUDGET;
        let mut found = None;
        for (km, id) in nearest_stations(stations, location.latitude, location.longitude) {
            let path = UpstreamPath::observation(id);
            let Ok(fetched) = tokio::time::timeout_at(deadline, self.nws.fetch(&path)).await else {
                break;
            };
            let Ok(latest) = fetched else {
                continue;
            };
            let Some(current) = observation(&latest, id, km, units, self.nws.public_url(&path))
            else {
                continue;
            };
            let has_temperature = current.temperature.is_some();
            found = Some(current);
            if has_temperature {
                break;
            }
        }
        found
    }
    /// The active alerts. A response without a `features` list cannot show that no alerts
    /// are active, so it counts as a failed check rather than an all-clear.
    async fn active_alerts(&self, path: &UpstreamPath) -> Result<Vec<AlertFeature>, Error> {
        let alerts: Alerts = self.nws.fetch(path).await?;
        required(alerts.features, path.as_str(), "features")
    }
    fn query_source(&self, path: &UpstreamPath) -> QuerySource {
        QuerySource {
            url: self.nws.public_url(path),
        }
    }
}
/// The error for a lookup refused by admission control or upstream pacing.
fn busy() -> Error {
    Error::new(
        ErrorCode::Busy,
        "Weather Bridge is busy. Please retry shortly.",
    )
}
/// Alerts use four-decimal coordinate precision rather than the two-decimal grid point.
fn alerts_path(location: &Location) -> UpstreamPath {
    UpstreamPath::alerts(location.latitude, location.longitude)
}
/// Coordinates get the place name and time zone of the NWS grid lookup.
fn name_from_point(location: &mut Location, grid: &PointsProperties) {
    if location.precision != Precision::Coordinates {
        return;
    }
    let place = &grid.relative_location.properties;
    if let (Some(city), Some(state)) = (&place.city, &place.state) {
        location.name = format!("Near {city}, {state}");
    }
    if let Some(zone) = &grid.time_zone {
        location.time_zone = Some(zone.clone());
    }
}
fn alerts_status(alerts: &Option<Vec<AlertFeature>>) -> CheckedAlertsStatus {
    if alerts.is_none() {
        CheckedAlertsStatus::Unavailable
    } else {
        CheckedAlertsStatus::Checked
    }
}
/// An optional source: on failure, note the warning and carry on without it.
fn partial<T>(
    result: Result<T, Error>,
    warning: Warning,
    warnings: &mut Vec<Warning>,
) -> Option<T> {
    result.inspect_err(|_| warnings.push(warning)).ok()
}

#[cfg(test)]
mod diagnostics_tests {
    use super::*;

    #[tokio::test]
    async fn admission_rejections_increment_the_busy_metric_without_contacting_nws() {
        let weather = Weather::new().unwrap();
        let permits = weather
            .admission
            .acquire_many(MAX_LOOKUPS_IN_FLIGHT as u32)
            .await
            .unwrap();
        let error = weather
            .report(WeatherQuery {
                city: Some("Seattle, WA".into()),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Busy);
        assert!(
            weather
                .metrics()
                .render()
                .contains("weather_bridge_busy_total 1")
        );
        drop(permits);
    }
}
