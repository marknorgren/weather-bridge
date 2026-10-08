mod common;

use axum::{body::Body, http::Request};
use common::{Faults, coordinates, fixture, seattle};
use http_body_util::BodyExt;
use serde_json::Value;
use std::{
    io::Write,
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};
use tower::ServiceExt;
use weather_bridge::{
    BUILD_REVISION, ErrorCode,
    api::{HttpConfig, OriginVerify, app},
    weather::Weather,
};

fn test_app() -> axum::Router {
    app(Arc::new(Weather::new().unwrap()), HttpConfig::default())
}

async fn get(path: &str) -> axum::response::Response {
    test_app()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn every_http_response_has_a_shared_request_id() {
    for path in ["/healthz", "/missing", "/v1/weather?city=Springfield"] {
        let response = get(path).await;
        let id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_else(|| panic!("missing request ID for {path}"));
        assert!(!id.is_empty(), "{path}");
    }

    let supplied = "01JZZZZZZZZZZZZZZZZZZZZZZZ";
    let response = test_app()
        .oneshot(
            Request::get("/healthz")
                .header("x-request-id", supplied)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["x-request-id"], supplied);

    let response = test_app()
        .oneshot(
            Request::get("/healthz")
                .header("x-request-id", "not safe")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(response.headers()["x-request-id"], "not safe");
}

#[tokio::test]
async fn middleware_errors_keep_request_correlation() {
    let protected = app(
        Arc::new(Weather::new().unwrap()),
        HttpConfig {
            origin_verify: Some(
                OriginVerify::new("0123456789abcdefghijklmnopqrstuvwxyzABCD").unwrap(),
            ),
            ..Default::default()
        },
    );
    let response = protected
        .oneshot(
            Request::get("/v1/cities?q=Seattle")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    assert!(response.headers().contains_key("x-request-id"));

    let response = test_app()
        .oneshot(
            Request::get("/v1/cities?q=Seattle")
                .header("content-length", "16385")
                .body(Body::from(vec![b'x'; 16_385]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 413);
    assert!(response.headers().contains_key("x-request-id"));
}

#[tokio::test]
async fn version_and_metrics_are_operator_endpoints() {
    let response = get("/version").await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["revision"], BUILD_REVISION);

    let response = get("/metrics").await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let content_type = response.headers()["content-type"].to_str().unwrap();
    assert!(content_type.starts_with("text/plain"), "{content_type}");
    let text = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    for metric in [
        "weather_bridge_http_requests_total",
        "weather_bridge_http_request_duration_seconds",
        "weather_bridge_upstream_requests_total",
        "weather_bridge_cache_access_total",
        "weather_bridge_partial_reports_total",
        "weather_bridge_busy_total",
    ] {
        assert!(text.contains(metric), "missing {metric} in:\n{text}");
    }
    assert!(
        !text.contains("Seattle") && !text.contains("city="),
        "{text}"
    );
}

#[tokio::test]
async fn metrics_report_cache_upstream_and_partial_behavior_with_fixed_labels() {
    let harness = fixture(Faults {
        alerts_fail: true,
        ..Default::default()
    })
    .await;
    harness.weather.report(seattle()).await.unwrap();
    harness.weather.report(seattle()).await.unwrap();
    harness.weather.alerts(seattle()).await.unwrap();
    let response = app(harness.weather.clone(), HttpConfig::default())
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let text = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    for expected in [
        "weather_bridge_cache_access_total{tier=\"document\",result=\"hit\"}",
        "weather_bridge_cache_access_total{tier=\"document\",result=\"miss\"}",
        "weather_bridge_upstream_failures_total{reason=\"status\"} 3",
        "weather_bridge_partial_reports_total{operation=\"weather\"} 2",
        "weather_bridge_partial_reports_total{operation=\"hourly\"} 0",
        "weather_bridge_partial_reports_total{operation=\"alerts\"} 1",
    ] {
        assert!(text.contains(expected), "missing {expected}:\n{text}");
    }
    for private in ["Seattle", "47.61", "point=", "/alerts/active"] {
        assert!(
            !text.contains(private),
            "metric leaked {private:?}:\n{text}"
        );
    }
}

#[tokio::test]
async fn malformed_successful_documents_are_upstream_failures_not_successes() {
    for (faults, kind) in [
        (
            Faults {
                points_malformed_once: true,
                ..Default::default()
            },
            "points",
        ),
        (
            Faults {
                forecast_missing_periods_once: true,
                ..Default::default()
            },
            "forecast",
        ),
    ] {
        let harness = fixture(faults).await;
        assert!(harness.weather.report(seattle()).await.is_err());
        assert!(harness.weather.report(seattle()).await.is_ok());
        let response = app(harness.weather.clone(), HttpConfig::default())
            .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let text = String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        for (outcome, count) in [("success", 1), ("failure", 1)] {
            let expected = format!(
                "weather_bridge_upstream_requests_total{{kind=\"{kind}\",outcome=\"{outcome}\"}} {count}"
            );
            assert!(text.contains(&expected), "missing {expected}:\n{text}");
        }
    }
}

async fn metrics_text(weather: Arc<Weather>) -> String {
    let response = app(weather, HttpConfig::default())
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap()
}

#[tokio::test]
async fn outside_coverage_outcomes_are_only_points_404_and_alerts_400() {
    let outside = fixture(Faults::default()).await;
    assert!(outside.weather.report(coordinates(0., 0.)).await.is_err());
    let text = metrics_text(outside.weather.clone()).await;
    for kind in ["points", "alerts"] {
        let expected = format!(
            "weather_bridge_upstream_requests_total{{kind=\"{kind}\",outcome=\"outside_coverage\"}} 1\n"
        );
        assert!(text.contains(&expected), "missing {expected}:\n{text}");
    }

    let missing = fixture(Faults {
        forecast_missing: true,
        ..Default::default()
    })
    .await;
    assert!(missing.weather.report(seattle()).await.is_err());
    let text = metrics_text(missing.weather.clone()).await;
    for expected in [
        "weather_bridge_upstream_requests_total{kind=\"forecast\",outcome=\"outside_coverage\"} 0\n",
        "weather_bridge_upstream_requests_total{kind=\"forecast\",outcome=\"failure\"} 1\n",
    ] {
        assert!(text.contains(expected), "missing {expected}:\n{text}");
    }
}

#[tokio::test]
async fn a_404_from_a_discovered_link_under_points_is_a_failure_not_outside_coverage() {
    // The endpoint kind comes from which grid lookup link was followed, not from the
    // shape of its path, so these 404s are upstream failures.
    let harness = fixture(Faults {
        links_under_points: true,
        ..Default::default()
    })
    .await;
    let report = harness.weather.report(seattle()).await.unwrap_err();
    assert_eq!(report.code, ErrorCode::UpstreamUnavailable);
    let hourly = harness.weather.hourly(seattle()).await.unwrap_err();
    assert_eq!(hourly.code, ErrorCode::UpstreamUnavailable);
    let text = metrics_text(harness.weather.clone()).await;
    for expected in [
        "weather_bridge_upstream_requests_total{kind=\"points\",outcome=\"success\"} 1\n",
        "weather_bridge_upstream_requests_total{kind=\"points\",outcome=\"outside_coverage\"} 0\n",
        "weather_bridge_upstream_requests_total{kind=\"forecast\",outcome=\"failure\"} 1\n",
        "weather_bridge_upstream_requests_total{kind=\"hourly\",outcome=\"failure\"} 2\n",
        "weather_bridge_upstream_requests_total{kind=\"stations\",outcome=\"failure\"} 1\n",
        "weather_bridge_upstream_failures_total{reason=\"status\"} 4\n",
    ] {
        assert!(text.contains(expected), "missing {expected}:\n{text}");
    }
}

#[test]
fn cli_version_includes_the_embedded_build_revision() {
    let output = Command::new(env!("CARGO_BIN_EXE_weather-bridge"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")), "{stdout}");
    assert!(stdout.contains(BUILD_REVISION), "{stdout}");
    assert!(stdout.contains("revision"), "{stdout}");
}

#[test]
fn stdio_mcp_keeps_stdout_protocol_only() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_weather-bridge"))
        .arg("mcp")
        .env("RUST_LOG", "weather_bridge=info")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let message = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "diagnostics-test", "version": "1"}
        }
    });
    writeln!(child.stdin.take().unwrap(), "{message}").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lines: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|_| panic!("non-protocol stdout: {line}"))
        })
        .collect();
    assert!(lines.iter().any(|message| message["id"] == 1), "{lines:?}");
}

#[test]
fn local_logs_are_readable_and_lambda_logs_are_json() {
    fn startup_log(lambda: bool) -> String {
        let mut command = Command::new(env!("CARGO_BIN_EXE_weather-bridge"));
        command
            .args(["serve", "--bind", "127.0.0.1:0"])
            .env("RUST_LOG", "weather_bridge=info")
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if lambda {
            command.env("AWS_LAMBDA_FUNCTION_NAME", "diagnostics-test");
        } else {
            command.env_remove("AWS_LAMBDA_FUNCTION_NAME");
        }
        let mut child = command.spawn().unwrap();
        std::thread::sleep(Duration::from_secs(2));
        child.kill().unwrap();
        String::from_utf8(child.wait_with_output().unwrap().stderr).unwrap()
    }

    let local = startup_log(false);
    assert!(local.contains("Weather Bridge ready"), "{local}");
    assert!(!local.trim_start().starts_with('{'), "{local}");

    let deployed = startup_log(true);
    let event: Value = deployed
        .lines()
        .find_map(|line| serde_json::from_str(line).ok())
        .unwrap_or_else(|| panic!("expected JSON startup log: {deployed}"));
    assert_eq!(event["fields"]["message"], "Weather Bridge ready");
    assert_eq!(event["fields"]["revision"], BUILD_REVISION);
}

#[tokio::test]
async fn malformed_alert_entries_are_retried_and_counted_as_required_field_failures() {
    let harness = fixture(Faults {
        alerts_body: Some(
            r#"{"features":[{"properties":{"event":"Wind Advisory","instruction":42}}]}"#,
        ),
        ..Default::default()
    })
    .await;
    for _ in 0..2 {
        let result =
            serde_json::to_value(harness.weather.alerts(seattle()).await.unwrap()).unwrap();
        assert_eq!(result["alertsStatus"], "unavailable");
    }
    let text = metrics_text(harness.weather.clone()).await;
    for expected in [
        "weather_bridge_upstream_requests_total{kind=\"alerts\",outcome=\"failure\"} 2",
        "weather_bridge_upstream_requests_total{kind=\"alerts\",outcome=\"success\"} 0",
        "weather_bridge_upstream_failures_total{reason=\"required\"} 2",
    ] {
        assert!(text.contains(expected), "missing {expected}:\n{text}");
    }
}
