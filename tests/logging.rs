//! Log capture needs its own test binary: tracing caches per-callsite interest process-wide,
//! so events from tests running concurrently on other threads could race the capture.
mod common;

use axum::{body::Body, http::Request};
use common::{Faults, capture_logs, fixture, seattle, wire};
use tower::ServiceExt;
use weather_bridge::{
    ErrorCode,
    api::{HttpConfig, app},
    weather::{Config, Weather},
};

/// Every upstream failure cause is logged at WARN with the upstream path (never the query,
/// which holds caller coordinates), while clients get only the generic message.
#[tokio::test]
async fn upstream_failure_causes_are_logged_but_not_returned() {
    let logs = capture_logs();
    let h = fixture(Faults {
        alerts_fail: true,
        hourly_not_json: true,
        stations_oversized: true,
        ..Default::default()
    })
    .await;
    let report = wire(h.weather.report(seattle()).await);
    assert_eq!(report["warnings"].as_array().unwrap().len(), 4, "{report}");
    let text = logs.text();
    for expected in [
        "WARN",
        "path=/alerts/active ",
        "HTTP status 503",
        "path=/hourly ",
        "invalid JSON",
        "path=/stations ",
        "body exceeds 2000000 bytes",
    ] {
        assert!(
            text.contains(expected),
            "missing {expected:?} in logs:\n{text}"
        );
    }
    assert!(!text.contains("point="), "query logged:\n{text}");
    let body = report.to_string();
    // Timestamps and source URLs can legitimately contain the digits 503.
    for cause in [
        "HTTP status 503",
        "invalid JSON",
        "exceeds",
        "do not expose",
        "<html>",
    ] {
        assert!(!body.contains(cause), "{cause:?} leaked into {body}");
    }

    let h = fixture(Faults {
        alerts_malformed: true,
        ..Default::default()
    })
    .await;
    h.weather.alerts(seattle()).await.unwrap();
    let text = logs.text();
    assert!(text.contains("missing=features"), "{text}");

    // A grid link is checked, and its warning logged, only when an operation needs it:
    // the hourly endpoint never warns about a missing daily forecast link.
    let warning = "NWS grid lookup link is missing or not on the NWS origin";
    let h = fixture(Faults {
        forecast_link_missing: true,
        ..Default::default()
    })
    .await;
    h.weather.hourly(seattle()).await.unwrap();
    assert!(!logs.text().contains(warning), "{}", logs.text());
    assert_eq!(
        h.weather.report(seattle()).await.unwrap_err().code,
        ErrorCode::UpstreamUnavailable
    );
    let text = logs.text();
    assert!(text.contains(warning), "{text}");
    assert!(text.contains("link=\"forecast\""), "{text}");
    assert!(!text.contains("link=\"forecastHourly\""), "{text}");
    let h = fixture(Faults {
        hourly_link_missing: true,
        stations_link_off_origin: true,
        ..Default::default()
    })
    .await;
    wire(h.weather.report(seattle()).await);
    let text = logs.text();
    for link in ["forecastHourly", "observationStations"] {
        assert!(text.contains(&format!("link=\"{link}\"")), "{text}");
    }

    // A closed port: the connection is refused before any request is sent.
    let weather = Weather::configured(Config {
        nws_base_url: "http://127.0.0.1:9".into(),
        ..Config::default()
    })
    .unwrap();
    let error = weather.report(seattle()).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamUnavailable);
    let text = logs.text();
    assert!(text.contains("path=/points/47.61,-122.33 "), "{text}");
    assert!(text.contains("transport error (connect)"), "{text}");
    let body = error.body().to_string();
    assert!(
        !body.contains("connect") && !body.contains("127.0.0.1"),
        "{body}"
    );

    // Request traces use the registered route instead of the raw URI, and carry only a
    // validated correlation ID from request headers.
    let secret = "DO_NOT_LOG_THIS_QUERY_VALUE";
    let response = app(
        std::sync::Arc::new(Weather::new().unwrap()),
        HttpConfig::default(),
    )
    .oneshot(
        Request::get(format!("/v1/weather?units={secret}"))
            .header("x-request-id", "diagnostic-test-id")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 400);
    let text = logs.text();
    assert!(text.contains("route=\"/v1/weather\""), "{text}");
    assert!(text.contains("request_id=diagnostic-test-id"), "{text}");
    assert!(text.contains("status=400"), "{text}");
    assert!(!text.contains(secret), "query leaked into logs:\n{text}");
    assert!(
        !text.contains("units="),
        "raw query leaked into logs:\n{text}"
    );
}
