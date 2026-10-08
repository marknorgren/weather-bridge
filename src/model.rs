//! Typed response bodies shared by REST, MCP and the CLI.
//!
//! These types are the runtime side of the contract in `openapi.json`;
//! `tests/contract.rs` validates their serialized form against it. Field names follow the
//! wire format (camelCase). Responses are converted to `serde_json::Value` before they are
//! written, so object keys keep the sorted order clients have always received.
use crate::weather::{GridPoint, Units};
use rmcp::schemars;
use serde::{Serialize, Serializer};
use serde_json::Number;

/// `{ "data": ..., "meta": ... }` wrapper for every successful response.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields, rename = "{T}Envelope")]
pub struct Envelope<T> {
    pub data: T,
    pub meta: Meta,
}
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct Meta {
    /// Data sources that must be credited.
    pub attribution: Vec<Attribution>,
}
impl Default for Meta {
    fn default() -> Self {
        Self {
            attribution: vec![
                Attribution {
                    name: "National Weather Service".into(),
                    url: "https://www.weather.gov/".into(),
                    license: None,
                },
                Attribution {
                    name: "GeoNames".into(),
                    url: "https://www.geonames.org/".into(),
                    license: Some("CC BY 4.0".into()),
                },
            ],
        }
    }
}
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct Attribution {
    pub name: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("description" = "Present when the source requires a license notice."))]
    pub license: Option<String>,
}

/// Up to ten city search matches, ordered by population.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct Cities(#[schemars(length(max = 10))] pub Vec<crate::cities::City>);

/// How the location was determined.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[schemars(
    description = "city-center when resolved from the city index; coordinates when supplied by the caller."
)]
pub enum Precision {
    /// Resolved from the city index; coordinates are the city center.
    CityCenter,
    /// Coordinates supplied by the caller.
    Coordinates,
}
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
pub struct Location {
    pub name: String,
    #[schemars(extend("description" = "City center, or the latitude supplied."))]
    pub latitude: f64,
    #[schemars(extend("description" = "City center, or the longitude supplied."))]
    pub longitude: f64,
    pub time_zone: Option<String>,
    #[schemars(extend("description" = "GeoNames ID when the location came from the city index."))]
    pub city_id: Option<u64>,
    pub precision: Precision,
    /// The rounded point sent to the NWS /points grid lookup; forecasts follow that grid.
    pub grid_lookup_point: GridPoint,
}

/// A measurement in the selected units, rounded to one decimal.
#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
#[schemars(description = "A measurement in the selected units.")]
pub struct Quantity {
    #[schemars(extend("description" = "Rounded to one decimal."))]
    pub value: f64,
    pub unit: Unit,
}
/// The unit of a [`Quantity`]. Each variant serializes to the exact string shown to callers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[schemars(description = "°F and mph for us units; °C and km/h for metric units.")]
pub enum Unit {
    // Variants carry no doc comments, so the schema stays a plain string enum.
    #[serde(rename = "°F")]
    Fahrenheit,
    #[serde(rename = "°C")]
    Celsius,
    #[serde(rename = "mph")]
    MilesPerHour,
    #[serde(rename = "km/h")]
    KilometresPerHour,
}
impl Unit {
    /// Every variant, in schema order.
    pub const ALL: [Self; 4] = [
        Self::Fahrenheit,
        Self::Celsius,
        Self::MilesPerHour,
        Self::KilometresPerHour,
    ];
    /// The wire string, identical to the serialized JSON value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fahrenheit => "°F",
            Self::Celsius => "°C",
            Self::MilesPerHour => "mph",
            Self::KilometresPerHour => "km/h",
        }
    }
}
impl std::fmt::Display for Unit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
/// A station observation. Never a forecast.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
#[schemars(description = "Station observation, never a forecast. Missing quantities are null.")]
pub struct Observation {
    #[schemars(extend("format" = "date-time"))]
    pub observed_at: String,
    #[schemars(extend("description" = "Seconds since observedAt; null when the timestamp is invalid."))]
    pub age_seconds: Option<i64>,
    /// True when the observation is over two hours old or its timestamp is invalid.
    #[schemars(extend("description" = "True when over 2 hours old or the timestamp is invalid."))]
    pub stale: bool,
    #[schemars(extend("description" = "NWS station identifier."))]
    pub station: String,
    pub station_distance_km: f64,
    pub condition: Option<String>,
    pub temperature: Option<Quantity>,
    pub humidity_percent: Option<Number>,
    pub wind_speed: Option<Quantity>,
    pub wind_direction_degrees: Option<Number>,
    #[schemars(extend("description" = "NWS URL of this observation."))]
    pub source_url: String,
}
/// One NWS forecast period (daily or hourly).
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
#[schemars(description = "One NWS forecast period (daily or hourly).")]
pub struct Period {
    pub name: Option<String>,
    #[schemars(extend("format" = "date-time"))]
    pub starts_at: Option<String>,
    #[schemars(extend("format" = "date-time"))]
    pub ends_at: Option<String>,
    pub is_daytime: Option<bool>,
    pub temperature: Option<Quantity>,
    pub precipitation_probability_percent: Option<Number>,
    #[schemars(extend("description" = "Speed and direction in the selected units, e.g. 8–16 km/h NW."))]
    pub wind: String,
    pub condition: Option<String>,
    /// Official NWS forecast text, verbatim.
    #[schemars(extend("description" = "Official NWS forecast text, verbatim; measurements keep the original NWS units."))]
    pub detail: Option<String>,
}
/// One active NWS alert. Text fields are the original NWS wording.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
#[schemars(description = "One active NWS alert. Text fields are the original NWS wording.")]
pub struct Alert {
    pub id: Option<String>,
    pub event: Option<String>,
    pub severity: Option<String>,
    pub headline: Option<String>,
    pub description: Option<String>,
    #[schemars(extend("description" = "Original NWS instruction text."))]
    pub instruction: Option<String>,
    #[schemars(extend("format" = "date-time"))]
    pub effective_at: Option<String>,
    #[schemars(extend("format" = "date-time"))]
    pub expires_at: Option<String>,
    pub area: Option<String>,
}
/// Whether active alerts were checked for this response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[schemars(
    description = "checked: alerts were checked (an empty list means none are active). unavailable: the alert check failed; this does not mean there are no alerts. not-checked: this response does not check alerts by design."
)]
pub enum AlertsStatus {
    /// Alerts were checked; an empty list means none are active.
    Checked,
    /// The alert check failed. This does not mean there are no alerts.
    Unavailable,
    /// This response does not check alerts by design (hourly forecast).
    NotChecked,
}

/// Alert check outcomes for endpoints that check alerts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CheckedAlertsStatus {
    /// Alerts were checked; an empty list means none are active.
    Checked,
    /// The alert check failed. This does not mean there are no alerts.
    Unavailable,
}
impl From<CheckedAlertsStatus> for AlertsStatus {
    fn from(status: CheckedAlertsStatus) -> Self {
        match status {
            CheckedAlertsStatus::Checked => Self::Checked,
            CheckedAlertsStatus::Unavailable => Self::Unavailable,
        }
    }
}
/// The hourly endpoint deliberately does not check alerts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HourlyAlertsStatus {
    NotChecked,
}
impl From<HourlyAlertsStatus> for AlertsStatus {
    fn from(_: HourlyAlertsStatus) -> Self {
        Self::NotChecked
    }
}

/// A note attached to a response. Serialized as its message. Source failures make a
/// response partial; informational notes do not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Warning {
    HourlyUnavailable,
    AlertsUnavailable,
    StationsUnavailable,
    NoObservation,
    /// Fixed note on hourly results, which skip alerts by design.
    AlertsNotChecked,
}
impl Warning {
    pub fn message(self) -> &'static str {
        match self {
            Self::HourlyUnavailable => "Hourly forecast is temporarily unavailable.",
            Self::AlertsUnavailable => {
                "Alerts could not be checked. This does not mean there are no alerts."
            }
            Self::StationsUnavailable => "Observation stations are temporarily unavailable.",
            Self::NoObservation => {
                "No recent station observation is available. Forecast values are shown separately."
            }
            Self::AlertsNotChecked => {
                "This hourly forecast does not check alerts. Use /v1/alerts or get_active_alerts."
            }
        }
    }
    /// True when a source failed or returned nothing usable, so the response is partial.
    pub fn is_partial(self) -> bool {
        !matches!(self, Self::AlertsNotChecked)
    }
}
impl Serialize for Warning {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.message())
    }
}
impl schemars::JsonSchema for Warning {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Warning".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        String::json_schema(generator)
    }
}
/// A response is complete when alerts were not reported unavailable and no source failed.
/// Only complete responses may be cached by shared caches.
pub fn is_complete(alerts_status: AlertsStatus, warnings: &[Warning]) -> bool {
    alerts_status != AlertsStatus::Unavailable && !warnings.iter().any(|w| w.is_partial())
}
/// A weather response that reports its alert status and warnings, and therefore whether it
/// is complete (see [`is_complete`]).
pub trait Completeness {
    fn alerts_status(&self) -> AlertsStatus;
    fn warnings(&self) -> &[Warning];
    /// True when the response may be cached by shared caches.
    fn is_complete(&self) -> bool {
        is_complete(self.alerts_status(), self.warnings())
    }
}
/// Implements [`Completeness`] from the `alerts_status` and `warnings` fields.
macro_rules! completeness_from_fields {
    ($($ty:ty),+) => {$(
        impl Completeness for $ty {
            fn alerts_status(&self) -> AlertsStatus {
                self.alerts_status.into()
            }
            fn warnings(&self) -> &[Warning] {
                &self.warnings
            }
        }
    )+};
}
completeness_from_fields!(WeatherReport, HourlyForecast, ActiveAlerts);

/// An upstream document and when NWS issued it.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
#[schemars(description = "An NWS document and when it was issued.")]
pub struct IssuedSource {
    pub url: String,
    #[schemars(extend("format" = "date-time", "description" = "NWS update time; null when unavailable."))]
    pub issued_at: Option<String>,
}
/// An upstream query URL.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
#[schemars(description = "An NWS query URL.")]
pub struct QuerySource {
    pub url: String,
}
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ReportSources {
    pub forecast: IssuedSource,
    /// Absent when the grid lookup did not provide a safe hourly source URL.
    pub hourly: Option<IssuedSource>,
    pub alerts: QuerySource,
}
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct HourlySources {
    pub hourly: IssuedSource,
}
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct AlertSources {
    pub alerts: QuerySource,
}

/// Full weather report: observation, forecast, hourly periods and alerts.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
#[schemars(description = "Full weather report: observation, forecast, hourly periods and alerts.")]
pub struct WeatherReport {
    pub location: Location,
    pub units: Units,
    /// Official forecast text for the first period.
    #[schemars(extend("description" = "Official forecast text for the first period."))]
    pub summary: String,
    #[schemars(extend("description" = "Station observation or null; never a forecast."))]
    pub current: Option<Observation>,
    #[schemars(extend("maxItems" = 14))]
    pub forecast: Vec<Period>,
    #[schemars(extend("maxItems" = 24, "description" = "Empty when the hourly forecast failed (see warnings)."))]
    pub hourly: Vec<Period>,
    pub alerts: Vec<Alert>,
    #[schemars(extend("description" = "See AlertsStatus. unavailable means alerts could not be checked."))]
    pub alerts_status: CheckedAlertsStatus,
    #[schemars(extend("description" = "Human-readable notes. Source failures make the response partial (Cache-Control: no-store)."))]
    pub warnings: Vec<Warning>,
    pub sources: ReportSources,
    #[schemars(extend("format" = "date-time", "description" = "When this response was assembled, not when upstream data were observed."))]
    pub assembled_at: String,
    #[schemars(extend("description" = "Upstream responses are cached up to this many seconds."))]
    pub cache_max_age_seconds: u64,
}
/// Next 24 hourly forecast periods. Alerts are not checked.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
#[schemars(description = "Next 24 hourly forecast periods. Alerts are not checked.")]
pub struct HourlyForecast {
    pub location: Location,
    pub units: Units,
    #[schemars(extend("maxItems" = 24))]
    pub hourly: Vec<Period>,
    #[schemars(extend("description" = "This endpoint does not check alerts; use /v1/alerts."))]
    pub alerts_status: HourlyAlertsStatus,
    pub sources: HourlySources,
    #[schemars(extend("description" = "Always includes a fixed note that alerts were not checked; that note alone does not make the response partial."))]
    pub warnings: Vec<Warning>,
    #[schemars(extend("format" = "date-time", "description" = "When this response was assembled, not when upstream data were observed."))]
    pub assembled_at: String,
    #[schemars(extend("description" = "Upstream responses are cached up to this many seconds."))]
    pub cache_max_age_seconds: u64,
}
/// Active alerts and whether they could be checked.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
#[schemars(description = "Active alerts and whether they could be checked.")]
pub struct ActiveAlerts {
    pub location: Location,
    pub units: Units,
    pub alerts: Vec<Alert>,
    #[schemars(extend("description" = "unavailable means alerts could not be checked, not that there are none."))]
    pub alerts_status: CheckedAlertsStatus,
    pub sources: AlertSources,
    #[schemars(extend("description" = "Human-readable notes. Source failures make the response partial (Cache-Control: no-store)."))]
    pub warnings: Vec<Warning>,
    #[schemars(extend("format" = "date-time", "description" = "When this response was assembled, not when upstream data were observed."))]
    pub assembled_at: String,
    #[schemars(extend("description" = "Upstream responses are cached up to this many seconds."))]
    pub cache_max_age_seconds: u64,
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn every_unit_serializes_to_its_published_string() {
        let wire: Vec<_> = Unit::ALL
            .iter()
            .map(|u| serde_json::to_value(u).unwrap())
            .collect();
        assert_eq!(
            wire,
            [json!("°F"), json!("°C"), json!("mph"), json!("km/h")]
        );
        for unit in Unit::ALL {
            assert_eq!(json!(unit.as_str()), json!(unit));
            assert_eq!(unit.to_string(), unit.as_str());
        }
        let q = Quantity {
            value: 68.0,
            unit: Unit::Fahrenheit,
        };
        assert_eq!(
            serde_json::to_string(&q).unwrap(),
            r#"{"value":68.0,"unit":"°F"}"#
        );
    }
    #[test]
    fn unit_schema_is_a_string_enum_of_every_variant() {
        let schema = serde_json::to_value(schemars::schema_for!(Unit)).unwrap();
        assert_eq!(schema["type"], "string");
        assert_eq!(schema["enum"], json!(["°F", "°C", "mph", "km/h"]));
    }
}
