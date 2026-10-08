//! Route table: static pages and scripts, `/healthz`, `/openapi.json` and the `/v1` handlers.
use super::cache::CachePolicy;
use super::openapi::{ApiQuery, ApiResponse};
use super::{DocsUrl, security::PAGE_CSP};
use crate::{
    Error, ErrorCode,
    cities::city_query_schema,
    model::{ActiveAlerts, Cities, Completeness, HourlyForecast, WeatherReport},
    weather::{Weather, WeatherQuery, with_cache_lifetime},
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Query, State, rejection::QueryRejection},
    http::{
        HeaderValue,
        header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, X_FRAME_OPTIONS},
    },
    response::Html,
    routing::{MethodRouter, get},
};
use rmcp::schemars;
use serde::Deserialize;
use serde::Serialize;
#[cfg(test)]
use serde_json::Value;
use serde_json::json;
use std::sync::{Arc, LazyLock};

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchQuery {
    #[schemars(transform = city_query_schema, example = "Springfield, IL")]
    q: String,
}
/// Clients get one generic message; the specific rejection is logged at debug level.
fn query<T>(q: Result<Query<T>, QueryRejection>) -> Result<T, Error> {
    q.map(|Query(v)| v).map_err(|_| {
        tracing::debug!("rejected query string");
        Error::invalid("Invalid query. Use city=Seattle, WA or lat=47.6062&lon=-122.3321; units=us or metric. City search uses q=Seattle.")
    })
}
/// `openapi.json` in the compact form clients have always received, serialized once.
static OPENAPI: LazyLock<Bytes> = LazyLock::new(|| {
    let spec = serde_json::to_value(super::openapi()).expect("OpenAPI serializes");
    serde_json::to_vec(&spec)
        .expect("JSON values serialize")
        .into()
});
/// The `/developer` page with its `@@DOCS_URL@@` placeholder filled in. Rendered once per
/// app; `DocsUrl` guarantees the value is safe inside an HTML attribute.
fn developer_page(docs_url: &DocsUrl) -> String {
    include_str!("../../web/developer.html").replace("@@DOCS_URL@@", docs_url.as_str())
}
fn page(
    body: impl Into<Bytes>,
    csp: &'static str,
    frame: &'static str,
) -> MethodRouter<Arc<Weather>> {
    let body: Bytes = body.into();
    get(move || {
        let body = body.clone();
        async move {
            (
                [
                    (CONTENT_SECURITY_POLICY, HeaderValue::from_static(csp)),
                    (X_FRAME_OPTIONS, HeaderValue::from_static(frame)),
                    (CACHE_CONTROL, CachePolicy::PAGE.header_value()),
                ],
                Html(body),
            )
        }
    })
}
fn script(body: &'static str) -> MethodRouter<Arc<Weather>> {
    get(move || async move {
        (
            [
                (
                    CONTENT_TYPE,
                    HeaderValue::from_static("text/javascript; charset=utf-8"),
                ),
                (CACHE_CONTROL, CachePolicy::PAGE.header_value()),
            ],
            body,
        )
    })
}
/// Every route except `/mcp`, which `app` nests with its own transport.
pub(super) fn router(docs_url: &DocsUrl) -> Router<Arc<Weather>> {
    Router::new()
        .route(
            "/",
            page(include_str!("../../web/index.html"), PAGE_CSP, "DENY"),
        )
        .route(
            "/developer",
            page(developer_page(docs_url), PAGE_CSP, "DENY"),
        )
        .route(
            "/assets/weather.js",
            script(include_str!("../../web/weather.js")),
        )
        .route(
            "/assets/developer.js",
            script(include_str!("../../web/developer.js")),
        )
        .route("/healthz", get(|| async { Json(json!({"status":"ok"})) }))
        .route(
            "/openapi.json",
            get(|| async {
                (
                    [
                        (CONTENT_TYPE, HeaderValue::from_static("application/json")),
                        (CACHE_CONTROL, CachePolicy::PAGE.header_value()),
                    ],
                    OPENAPI.clone(),
                )
            }),
        )
        .merge(rest_router().0)
}

/// The same documented route registration constructs runtime routes and the offline contract.
pub(super) fn rest_router() -> (Router<Arc<Weather>>, aide::openapi::OpenApi) {
    use aide::{
        axum::{ApiRouter, routing::get_with},
        openapi::{Info, OpenApi, Server},
    };
    super::openapi::initialize();
    let mut spec = OpenApi {
        info: Info {
            title: "Weather Bridge".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: Some(DESCRIPTION.into()),
            ..Default::default()
        },
        servers: vec![Server {
            url: "/".into(),
            description: Some("The Weather Bridge server that serves this document".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let router = ApiRouter::new()
        .api_route("/version", get_with(version, |op| {
            let mut op = op.id("getVersion").summary("Running version and embedded source revision");
            super::openapi::errors(op.inner_mut(), &[ErrorCode::Forbidden]);
            op
        }))
        .api_route("/metrics", get_with(metrics, |op| {
            let mut op = op.id("getMetrics").summary("Bounded Prometheus operational metrics with fixed labels");
            super::openapi::errors(op.inner_mut(), &[ErrorCode::Forbidden]);
            op
        }))
        .api_route("/v1/cities", get_with(cities, |op| {
            let op = op.id("searchCities").summary("Search US city names or prefixes; optional state qualifier");
            let mut op = op;
            super::openapi::success_description(op.inner_mut(), "Up to 10 exact or prefix matches ordered by population");
            super::openapi::errors(op.inner_mut(), &[ErrorCode::InvalidLocation, ErrorCode::Forbidden, ErrorCode::RequestTooLarge, ErrorCode::UpstreamTimeout]);
            op
        }))
        .api_route("/v1/weather", get_with(weather_report, |op| {
            let op = op.id("getWeather").summary("Weather by city or coordinates");
            let op = op.description("Supply exactly one location mode: city, cityId, or both lat and lon. Forecasts, observations and alerts are cached up to 120 seconds and NWS grid lookups up to six hours; city centers approximate a point. US/territory coverage.");
            let mut op = op;
            super::openapi::success_description(op.inner_mut(), "Weather report; may contain partial-source warnings.");
            super::openapi::errors(op.inner_mut(), &ErrorCode::ALL);
            op
        }))
        .api_route("/v1/forecast/hourly", get_with(hourly, |op| {
            let op = op.id("getHourlyForecast").summary("Next 24 hourly forecast periods");
            let op = op.description("Supply exactly one location mode: city, cityId, or both lat and lon. Fetches only the NWS grid lookup and hourly forecast; alerts are not checked (alertsStatus is not-checked). Hourly data are cached up to 120 seconds and grid lookups up to six hours. US/territory coverage.");
            let mut op = op;
            super::openapi::success_description(op.inner_mut(), "Hourly forecast periods. A failed hourly fetch returns 502.");
            super::openapi::errors(op.inner_mut(), &ErrorCode::ALL);
            op
        }))
        .api_route("/v1/alerts", get_with(alerts, |op| {
            let op = op.id("getActiveAlerts").summary("Active official alerts; inspect alertsStatus");
            let op = op.description("Supply exactly one location mode: city, cityId, or both lat and lon. Fetches only NWS active alerts, independent of forecast availability. alertsStatus unavailable means alerts could not be checked, not that there are none. Cached up to 120 seconds. US/territory coverage.");
            let mut op = op;
            super::openapi::success_description(op.inner_mut(), "Active alerts and alert-check status; an alert-check failure is alertsStatus unavailable with a warning.");
            super::openapi::errors(op.inner_mut(), &ErrorCode::ALL);
            op
        }))
        .finish_api(&mut spec);
    (router, spec)
}
#[derive(Serialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields, inline)]
struct Version {
    version: &'static str,
    revision: &'static str,
}

async fn version() -> Json<Version> {
    Json(Version {
        version: env!("CARGO_PKG_VERSION"),
        revision: crate::BUILD_REVISION,
    })
}

async fn metrics(State(weather): State<Arc<Weather>>) -> String {
    weather.metrics().render()
}
const DESCRIPTION: &str = "Friendly NWS weather with local GeoNames city lookup. MCP at /mcp uses Streamable HTTP, not REST. GET and HEAD on /v1/* and /openapi.json send Access-Control-Allow-Origin: * (public, read-only, no credentials), so browser clients on any origin can call the API. Responses carry Cache-Control: complete reports are public with max-age and s-maxage equal to the remaining life of their oldest upstream source (at most 120 s); 400/404/409 errors are public, max-age=60; city search is public, max-age=86400; reports with alertsStatus unavailable or a partial-source warning, 422 and 5xx errors are no-store.";
/// Wrap a service result in the JSON envelope with its Cache-Control class.
fn respond<T: serde::Serialize>(
    result: Result<T, Error>,
    policy: impl FnOnce(&T) -> CachePolicy,
) -> ApiResponse<T> {
    let cache = match &result {
        Ok(data) => policy(data),
        Err(error) => CachePolicy::for_error(error.code),
    };
    ApiResponse { result, cache }
}
async fn cities(
    State(w): State<Arc<Weather>>,
    ApiQuery(q): ApiQuery<SearchQuery>,
) -> ApiResponse<Cities> {
    respond(
        query(q).and_then(|q| w.search_cities(&q.q)).map(Cities),
        |_| CachePolicy::CITIES,
    )
}
/// Parse the query, run one weather operation under `with_cache_lifetime`, and pick the
/// cache policy from the result's completeness and the oldest source's remaining life.
async fn weather_response<T, F>(
    q: Result<Query<WeatherQuery>, QueryRejection>,
    operation: impl FnOnce(WeatherQuery) -> F,
) -> ApiResponse<T>
where
    T: Completeness + Serialize,
    F: Future<Output = Result<T, Error>>,
{
    let (result, lifetime) = match query(q) {
        Ok(q) => with_cache_lifetime(operation(q)).await,
        Err(e) => (Err(e), 0),
    };
    respond(result, |r| CachePolicy::report(r.is_complete(), lifetime))
}
async fn weather_report(
    State(w): State<Arc<Weather>>,
    ApiQuery(q): ApiQuery<WeatherQuery>,
) -> ApiResponse<WeatherReport> {
    weather_response(q, |q| w.report(q)).await
}
async fn hourly(
    State(w): State<Arc<Weather>>,
    ApiQuery(q): ApiQuery<WeatherQuery>,
) -> ApiResponse<HourlyForecast> {
    weather_response(q, |q| w.hourly(q)).await
}
async fn alerts(
    State(w): State<Arc<Weather>>,
    ApiQuery(q): ApiQuery<WeatherQuery>,
) -> ApiResponse<ActiveAlerts> {
    weather_response(q, |q| w.alerts(q)).await
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{DEFAULT_DOCS_URL as DOCS_URL, test_support::*};
    use crate::weather::CACHE_SECONDS;
    use axum::http::StatusCode;
    use http_body_util::BodyExt;

    fn max_age(policy: CachePolicy) -> u64 {
        policy.max_age().expect("policy is cacheable")
    }

    #[test]
    fn description_states_the_current_cache_policy() {
        let api = format!(
            "400/404/409 errors are public, max-age={}",
            max_age(CachePolicy::API)
        );
        let cities = format!(
            "city search is public, max-age={}",
            max_age(CachePolicy::CITIES)
        );
        let reports = format!("(at most {CACHE_SECONDS} s)");
        let uncached = format!(
            "partial-source warning, 422 and 5xx errors are {}",
            CachePolicy::NO_STORE
                .header_value()
                .to_str()
                .expect("ascii")
        );
        for phrase in [api, cities, reports, uncached] {
            assert!(
                DESCRIPTION.contains(&phrase),
                "DESCRIPTION lacks {phrase:?}"
            );
        }
    }

    #[tokio::test]
    async fn city_search_and_bad_queries() {
        let app = test_app();
        for (path, status) in [
            ("/v1/cities?q=Seattle", 200),
            ("/v1/weather?city=Springfield", 409),
            ("/v1/weather?lat=999&lon=0", 400),
            ("/v1/weather?city=Seattle&units=banana", 400),
            ("/v1/weather?city=Seattle&unknown=1", 400),
            ("/v1/weather?city=London,GB", 404),
        ] {
            let response = send(&app, get_with(path, None)).await;
            assert_eq!(response.status().as_u16(), status, "{path}");
        }
    }
    #[tokio::test]
    async fn openapi_is_served_as_compact_json_with_stable_headers() {
        let request = axum::http::Request::get("/openapi.json")
            .header("x-request-id", "stable-test-id")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = send(&test_app(), request).await;
        assert_eq!(response.status(), StatusCode::OK);
        let mut headers: Vec<String> = response
            .headers()
            .iter()
            .map(|(name, value)| format!("{name}: {}", value.to_str().unwrap()))
            .collect();
        headers.sort();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let spec: Value = serde_json::from_str(include_str!("../../openapi.json")).unwrap();
        assert_eq!(body, serde_json::to_vec(&spec).unwrap());
        let length = format!("content-length: {}", body.len());
        assert_eq!(
            headers,
            [
                "access-control-allow-origin: *",
                "cache-control: public, max-age=300",
                &length,
                "content-security-policy: default-src 'none'; base-uri 'none'; \
                 form-action 'none'; frame-ancestors 'none'",
                "content-type: application/json",
                "referrer-policy: no-referrer",
                "strict-transport-security: max-age=31536000",
                "vary: accept-encoding",
                "x-content-type-options: nosniff",
                "x-frame-options: DENY",
                "x-request-id: stable-test-id",
            ]
        );
    }
    #[tokio::test]
    async fn developer_page_links_to_the_configured_docs_url() {
        let docs_url = "https://docs.example.com/weather-bridge/";
        let app = crate::api::app(
            Arc::new(Weather::new().unwrap()),
            crate::api::HttpConfig {
                docs_url: crate::api::DocsUrl::new(docs_url).unwrap(),
                ..Default::default()
            },
        );
        let response = send(&app, get_with("/developer", None)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let html = std::str::from_utf8(&body).unwrap();
        assert!(html.contains(&format!("href=\"{docs_url}\"")), "{html}");
        assert!(
            html.contains(&format!("href=\"{docs_url}rest.html\"")),
            "{html}"
        );
        assert!(!html.contains(DOCS_URL), "default docs URL still linked");
        assert!(!html.contains("@@"), "unreplaced placeholder");
    }
    #[tokio::test]
    async fn rest_docs_live_on_github_pages_not_in_the_lambda() {
        let app = test_app();
        for path in [
            "/docs/rest",
            "/assets/reference.js",
            "/assets/scalar-1.72.1.js",
        ] {
            let response = send(&app, get_with(path, None)).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
        let response = send(&app, get_with("/developer", None)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let html = std::str::from_utf8(&body).unwrap();
        assert!(
            html.contains(&format!("href=\"{DOCS_URL}rest.html\"")),
            "{html}"
        );
        assert!(html.contains("href=\"/openapi.json\""));
        assert!(!html.contains("<iframe"));
        assert!(!html.contains("@@"), "unreplaced placeholder");
        assert!(DOCS_URL.starts_with("https://") && DOCS_URL.ends_with('/'));
    }
}
