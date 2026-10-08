//! Loopback NWS fixture shared by the integration tests. It never contacts NWS.
#![allow(dead_code, reason = "each test crate uses a different subset")]
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderValue, StatusCode, Uri, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::SecondsFormat;
use serde_json::{Value, json};
use std::str::FromStr;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use weather_bridge::{
    api::{HttpConfig, app},
    weather::{Config, Weather, WeatherQuery},
};

/// Header carried by every development-fixture response.
pub const DEV_FIXTURE_HEADER: &str = "x-weather-bridge-fixture";

/// Predictable, offline states exposed by the development fixture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevScenario {
    Healthy,
    Stale,
    AlertsUnavailable,
}

impl DevScenario {
    pub const ALL: [Self; 3] = [Self::Healthy, Self::Stale, Self::AlertsUnavailable];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Stale => "stale",
            Self::AlertsUnavailable => "alerts-unavailable",
        }
    }

    fn faults(self) -> Faults {
        match self {
            Self::Healthy => Faults {
                fresh_observation: true,
                ..Default::default()
            },
            Self::Stale => Faults::default(),
            Self::AlertsUnavailable => Faults {
                alerts_fail: true,
                fresh_observation: true,
                ..Default::default()
            },
        }
    }
}

impl FromStr for DevScenario {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|scenario| scenario.as_str() == value)
            .ok_or_else(|| {
                format!(
                    "unknown scenario {value:?}; expected healthy, stale, or alerts-unavailable"
                )
            })
    }
}
/// Fixture failure modes. Every upstream outage is opt-in per test.
#[derive(Clone, Copy, Default)]
pub struct Faults {
    pub alerts_fail: bool,
    pub alerts_body: Option<&'static str>,
    pub points_fail: bool,
    pub forecast_fail: bool,
    pub forecast_missing: bool,
    pub stations_fail: bool,
    /// /alerts/active answers 200 with a body that has no `features` array.
    pub alerts_malformed: bool,
    /// /hourly answers 200 with a body that is not JSON.
    pub hourly_not_json: bool,
    /// /stations answers 200 with a body over the 2 MB limit.
    pub stations_oversized: bool,
    /// The first /points response is HTTP 200 with no usable properties.
    pub points_malformed_once: bool,
    /// The first /forecast response is HTTP 200 without a periods list.
    pub forecast_missing_periods_once: bool,
    /// /hourly answers HTTP 200 without a periods list.
    pub hourly_missing_periods: bool,
    /// /stations answers HTTP 200 without a features list.
    pub stations_missing_features: bool,
    /// The grid lookup omits the optional hourly forecast link.
    pub hourly_link_missing: bool,
    /// The grid lookup omits the daily forecast link.
    pub forecast_link_missing: bool,
    /// The grid lookup publishes an off-origin optional station link.
    pub stations_link_off_origin: bool,
    /// The hourly and station links deliberately name the same URL.
    pub shared_hourly_stations_link: bool,
    /// The forecast, hourly and station links live under `/points/` and answer 404.
    pub links_under_points: bool,
    /// The first nearest-station observation has properties but no timestamp.
    pub observation_missing_timestamp_once: bool,
    /// Use the current time so the observation is fresh. The default fixed timestamp keeps
    /// existing tests deterministic and deliberately stale.
    pub fresh_observation: bool,
    /// /points and /forecast answer only after this delay: slow, but within the timeout.
    pub required_delay: Duration,
    /// Every station observation answers only after this delay.
    pub observation_delay: Duration,
    /// /stations lists a third station, so a report can try three observations.
    pub three_stations: bool,
    /// Every /points grid lookup succeeds, and its links carry the point as a query, so
    /// each grid point needs its own uncached forecast, hourly and station documents.
    pub every_point_covered: bool,
}
#[derive(Clone)]
struct Fixture {
    /// The fixture's own origin. It is the configured NWS base URL, so the links it
    /// publishes pass the service's origin check.
    root: String,
    hits: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
    faults: Faults,
}
fn unavailable() -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"internal":"do not expose"})),
    )
}
async fn response(State(f): State<Fixture>, uri: Uri) -> Response {
    f.hits.fetch_add(1, Ordering::SeqCst);
    let request_number = {
        let mut requests = f.requests.lock().unwrap();
        let request_number = requests
            .iter()
            .filter(|request| *request == &uri.to_string())
            .count()
            + 1;
        requests.push(uri.to_string());
        request_number
    };
    let delay = match uri.path() {
        path if path.starts_with("/points/") || path == "/forecast" => f.faults.required_delay,
        path if path.ends_with("/observations/latest") => f.faults.observation_delay,
        _ => Duration::ZERO,
    };
    tokio::time::sleep(delay).await;
    match uri.path() {
        "/hourly" if f.faults.hourly_not_json => {
            (StatusCode::OK, "<html>internal: do not expose</html>").into_response()
        }
        "/stations" if f.faults.stations_oversized => {
            (StatusCode::OK, format!("\"{}\"", "x".repeat(2_100_000))).into_response()
        }
        _ => document(f.faults, &f.root, &uri, request_number).into_response(),
    }
}
fn document(
    faults: Faults,
    root: &str,
    uri: &Uri,
    request_number: usize,
) -> (StatusCode, Json<Value>) {
    let value = match uri.path() {
        "/points/47.61,-122.33" if faults.points_fail => return unavailable(),
        "/points/47.61,-122.33" if faults.points_malformed_once && request_number == 1 => {
            json!({})
        }
        path if path == "/points/47.61,-122.33"
            || (faults.every_point_covered && path.starts_with("/points/")) =>
        {
            // These unknown paths fall through to the fixture's 404 answer.
            let (forecast, hourly, stations) = if faults.links_under_points {
                (
                    "/points/47.61,-122.33/forecast",
                    "/points/47.61,-122.33/forecast/hourly",
                    "/points/47.61,-122.33/stations",
                )
            } else {
                ("/forecast", "/hourly", "/stations")
            };
            let grid = if faults.every_point_covered {
                format!("?grid={}", uri.path().trim_start_matches("/points/"))
            } else {
                String::new()
            };
            let stations_path = if faults.shared_hourly_stations_link {
                format!("{root}{hourly}{grid}")
            } else {
                format!("{root}{stations}{grid}")
            };
            let mut properties = json!({
                "observationStations": if faults.stations_link_off_origin {
                    "https://example.invalid/stations".to_owned()
                } else {
                    stations_path
                }
            });
            if !faults.forecast_link_missing {
                properties["forecast"] = json!(format!("{root}{forecast}{grid}"));
            }
            if !faults.hourly_link_missing {
                properties["forecastHourly"] = json!(format!("{root}{hourly}{grid}"));
            }
            json!({"properties": properties})
        }
        "/forecast" if faults.forecast_fail => return unavailable(),
        "/forecast" if faults.forecast_missing => {
            return (StatusCode::NOT_FOUND, Json(json!({"error":"not found"})));
        }
        "/forecast" if faults.forecast_missing_periods_once && request_number == 1 => {
            json!({"properties":{"updateTime":"2026-09-30T12:00:00Z"}})
        }
        "/hourly" if faults.hourly_missing_periods => {
            json!({"properties":{"updateTime":"2026-09-30T12:00:00Z"}})
        }
        "/forecast" | "/hourly" => {
            json!({"properties":{"updateTime":"2026-09-30T12:00:00Z","periods":[{"name":"Today","startTime":"2026-09-30T13:00:00Z","endTime":"2026-09-30T14:00:00Z","isDaytime":true,"temperature":68,"temperatureUnit":"F","windSpeed":"5 to 10 mph","windDirection":"NW","shortForecast":"Sunny","detailedForecast":"Sunny today.","probabilityOfPrecipitation":{"value":null}}]}})
        }
        "/alerts/active" if uri.query() == Some("point=0.0000,0.0000") => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"detail":"Parameter \"point\" is invalid: out of bounds"})),
            );
        }
        "/alerts/active" if faults.alerts_fail => return unavailable(),
        "/alerts/active" if faults.alerts_malformed => json!({"type":"FeatureCollection"}),
        "/alerts/active" if faults.alerts_body.is_some() => {
            serde_json::from_str(faults.alerts_body.unwrap()).unwrap()
        }
        "/alerts/active" => {
            json!({"features":[{"id":"urn:alert:1","properties":{"event":"Wind Advisory","severity":"Moderate","headline":"Wind Advisory until 6 PM","description":"Gusts to 45 mph.","instruction":"Secure outdoor objects.","effective":"2026-09-30T12:00:00Z","expires":"2026-09-30T23:00:00Z","areaDesc":"Seattle"}}]})
        }
        "/stations" if faults.stations_fail => return unavailable(),
        "/stations" if faults.stations_missing_features => {
            json!({"type":"FeatureCollection"})
        }
        "/stations" if faults.three_stations => {
            json!({"features":[{"geometry":{"coordinates":[-121.0,48.0]},"properties":{"stationIdentifier":"FAR"}},{"geometry":{"coordinates":[-122.33,47.60]},"properties":{"stationIdentifier":"NEAR"}},{"geometry":{"coordinates":[-122.0,47.8]},"properties":{"stationIdentifier":"MID"}}]})
        }
        "/stations" => {
            json!({"features":[{"geometry":{"coordinates":[-121.0,48.0]},"properties":{"stationIdentifier":"FAR"}},{"geometry":{"coordinates":[-122.33,47.60]},"properties":{"stationIdentifier":"NEAR"}}]})
        }
        "/stations/NEAR/observations/latest" => {
            if faults.observation_missing_timestamp_once && request_number == 1 {
                json!({"properties":{"temperature":{"value":20,"unitCode":"wmoUnit:degC"}}})
            } else {
                let timestamp = if faults.fresh_observation {
                    chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
                } else {
                    "2020-01-01T12:00:00Z".to_owned()
                };
                json!({"properties":{"timestamp":timestamp,"temperature":{"value":20,"unitCode":"wmoUnit:degC"},"windSpeed":{"value":36,"unitCode":"wmoUnit:km_h-1"},"textDescription":"Clear","relativeHumidity":{"value":55}}})
            }
        }
        _ => return (StatusCode::NOT_FOUND, Json(json!({"error":"not found"}))),
    };
    (StatusCode::OK, Json(value))
}
pub struct Harness {
    pub weather: Arc<Weather>,
    /// The fixture origin, configured as the NWS base URL.
    pub base: String,
    pub hits: Arc<AtomicUsize>,
    pub requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub async fn fixture(faults: Faults) -> Harness {
    let hits = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().fallback(get(response)).with_state(Fixture {
        root: root.clone(),
        hits: hits.clone(),
        requests: requests.clone(),
        faults,
    });
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        weather: Arc::new(
            Weather::configured(Config {
                nws_base_url: root.clone(),
                ..Config::default()
            })
            .unwrap(),
        ),
        base: root,
        hits,
        requests,
        task,
    }
}

/// Start the loopback NWS fixture configured for a named development scenario.
pub async fn dev_fixture(scenario: DevScenario) -> Harness {
    fixture(scenario.faults()).await
}

/// Build the real HTTP/MCP app and make its synthetic nature visible on every response.
pub fn dev_app(weather: Arc<Weather>, scenario: DevScenario) -> Router {
    app(weather, HttpConfig::default()).layer(axum::middleware::from_fn_with_state(
        scenario,
        mark_fixture_response,
    ))
}

async fn mark_fixture_response(
    State(scenario): State<DevScenario>,
    mut request: Request,
    next: Next,
) -> Response {
    // The outer marker layer needs plain HTML in order to insert the visible banner.
    // Compression remains available for assets and API responses.
    if matches!(request.uri().path(), "/" | "/developer") {
        request.headers_mut().remove(header::ACCEPT_ENCODING);
    }
    let response = next.run(request).await;
    let is_html = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"));
    let mut response = if is_html {
        let (mut parts, body) = response.into_parts();
        match to_bytes(body, 1_000_000).await {
            Ok(bytes) => {
                let html = String::from_utf8_lossy(&bytes);
                let marker = format!(
                    "<div class=\"status error\" role=\"status\"><strong>SYNTHETIC OFFLINE FIXTURE</strong> · scenario: {} · no live NWS requests</div>",
                    scenario.as_str()
                );
                let html = html
                    .replacen("<body>", &format!("<body>{marker}"), 1)
                    .replace("value=\"San Francisco, CA\"", "value=\"Seattle, WA\"");
                parts.headers.remove(header::CONTENT_LENGTH);
                Response::from_parts(parts, Body::from(html))
            }
            Err(_) => Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("development fixture could not label HTML"))
                .expect("static fixture error response"),
        }
    } else {
        response
    };
    response.headers_mut().insert(
        DEV_FIXTURE_HEADER,
        HeaderValue::from_static("synthetic-offline-data"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
/// A service result as the JSON clients receive, so assertions check the wire format.
pub fn wire<T: serde::Serialize>(result: Result<T, weather_bridge::Error>) -> Value {
    serde_json::to_value(result.unwrap()).unwrap()
}
/// Under `start_paused` time, an idle runtime jumps the clock straight to the next timer,
/// which can fire an upstream timeout while a loopback response is still in transit. This
/// task keeps a timer at most one millisecond away, so the clock advances in small steps
/// and loopback I/O completes long before any timeout. Aborted when dropped.
pub fn small_clock_steps() -> ClockSteps {
    ClockSteps(tokio::spawn(async {
        loop {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }))
}
pub struct ClockSteps(tokio::task::JoinHandle<()>);
impl Drop for ClockSteps {
    fn drop(&mut self) {
        self.0.abort();
    }
}
pub fn seattle() -> WeatherQuery {
    WeatherQuery {
        city: Some("Seattle, WA".into()),
        ..Default::default()
    }
}
pub fn coordinates(lat: f64, lon: f64) -> WeatherQuery {
    WeatherQuery {
        lat: Some(lat),
        lon: Some(lon),
        ..Default::default()
    }
}
/// Log output captured on the current thread while the value lives. `#[tokio::test]` runs
/// the service and the fixture on the test thread, so their events land here.
pub struct Logs {
    buffer: Arc<Mutex<Vec<u8>>>,
    _guard: tracing::subscriber::DefaultGuard,
}
impl Logs {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.buffer.lock().unwrap()).into_owned()
    }
}
struct LogWriter(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub fn capture_logs() -> Logs {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let writer = buffer.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(move || LogWriter(writer.clone()))
        .finish();
    Logs {
        buffer,
        _guard: tracing::subscriber::set_default(subscriber),
    }
}
