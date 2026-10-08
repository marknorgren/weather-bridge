mod common;

use common::{Faults, coordinates, fixture, seattle, wire};
use serde_json::{Value, json};
use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use weather_bridge::{
    ErrorCode,
    weather::{Config, Units, Weather, WeatherQuery},
};
#[tokio::test]
async fn normalization_nearest_station_and_cache() {
    let h = fixture(Faults::default()).await;
    let report = wire(h.weather.report(seattle()).await);
    assert_eq!(report["current"]["station"], "NEAR");
    assert_eq!(report["current"]["temperature"]["value"], 68.0);
    assert_eq!(report["current"]["stale"], true);
    assert_eq!(report["alertsStatus"], "checked");
    assert!(report["hourly"][0]["precipitationProbabilityPercent"].is_null());
    let before = h.hits.load(Ordering::SeqCst);
    assert_eq!(before, 6);
    let metric = wire(
        h.weather
            .report(WeatherQuery {
                units: Units::Metric,
                ..seattle()
            })
            .await,
    );
    assert_eq!(metric["current"]["temperature"]["value"], 20.0);
    assert_eq!(metric["current"]["windSpeed"]["value"], 36.0);
    assert_eq!(h.hits.load(Ordering::SeqCst), before);
}
#[tokio::test]
async fn alert_outage_is_never_reported_as_all_clear() {
    let h = fixture(Faults {
        alerts_fail: true,
        ..Default::default()
    })
    .await;
    for (name, result) in [
        ("report", wire(h.weather.report(seattle()).await)),
        ("alerts", wire(h.weather.alerts(seattle()).await)),
    ] {
        assert_eq!(result["alertsStatus"], "unavailable", "{name}");
        assert_eq!(result["alerts"], json!([]), "{name}");
        assert!(
            result["warnings"]
                .to_string()
                .contains("could not be checked"),
            "{name}"
        );
        assert!(!result.to_string().contains("do not expose"), "{name}");
    }
    let error = h.weather.report(coordinates(0., 0.)).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::OutsideCoverage);
}
#[tokio::test]
async fn alerts_without_a_features_list_are_unavailable_not_all_clear() {
    let h = fixture(Faults {
        alerts_malformed: true,
        ..Default::default()
    })
    .await;
    for (name, result) in [
        ("report", wire(h.weather.report(seattle()).await)),
        ("alerts", wire(h.weather.alerts(seattle()).await)),
    ] {
        assert_eq!(result["alertsStatus"], "unavailable", "{name}");
        assert_eq!(result["alerts"], json!([]), "{name}");
        assert!(
            result["warnings"]
                .to_string()
                .contains("could not be checked"),
            "{name}"
        );
    }
    assert_eq!(
        h.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.starts_with("/alerts/active"))
            .count(),
        2,
        "an invalid alerts document must not enter the shared document cache"
    );
}
#[tokio::test]
async fn malformed_points_are_retried_instead_of_cached_for_six_hours() {
    let h = fixture(Faults {
        points_malformed_once: true,
        ..Default::default()
    })
    .await;
    assert_eq!(
        h.weather.report(seattle()).await.unwrap_err().code,
        ErrorCode::UpstreamUnavailable
    );
    let report = wire(h.weather.report(seattle()).await);
    assert_eq!(report["forecast"][0]["condition"], "Sunny");
    assert_eq!(
        h.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.as_str() == "/points/47.61,-122.33")
            .count(),
        2
    );
}
#[tokio::test]
async fn malformed_forecasts_are_retried_instead_of_cached_for_two_minutes() {
    let h = fixture(Faults {
        forecast_missing_periods_once: true,
        ..Default::default()
    })
    .await;
    assert_eq!(
        h.weather.report(seattle()).await.unwrap_err().code,
        ErrorCode::UpstreamUnavailable
    );
    let report = wire(h.weather.report(seattle()).await);
    assert_eq!(report["forecast"][0]["condition"], "Sunny");
    assert_eq!(
        h.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.as_str() == "/forecast")
            .count(),
        2
    );
}
#[tokio::test]
async fn unusable_observations_are_retried_instead_of_cached_for_two_minutes() {
    let h = fixture(Faults {
        observation_missing_timestamp_once: true,
        ..Default::default()
    })
    .await;
    let first = wire(h.weather.report(seattle()).await);
    assert!(first["current"].is_null());
    assert!(
        first["warnings"]
            .to_string()
            .contains("No recent station observation")
    );
    let second = wire(h.weather.report(seattle()).await);
    assert_eq!(second["current"]["station"], "NEAR");
    assert_eq!(
        h.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.as_str() == "/stations/NEAR/observations/latest")
            .count(),
        2
    );
}
#[tokio::test]
async fn missing_and_off_origin_optional_links_degrade_only_their_sources() {
    let h = fixture(Faults {
        hourly_link_missing: true,
        stations_link_off_origin: true,
        ..Default::default()
    })
    .await;
    let report = wire(h.weather.report(seattle()).await);
    assert_eq!(report["forecast"][0]["condition"], "Sunny");
    assert_eq!(report["alertsStatus"], "checked");
    assert_eq!(report["hourly"], json!([]));
    assert!(report["sources"]["hourly"].is_null());
    assert!(report["current"].is_null());
    assert!(report["warnings"].to_string().contains("Hourly forecast"));
    assert!(
        report["warnings"]
            .to_string()
            .contains("Observation stations")
    );
    assert!(
        h.requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| request != "/hourly" && request != "/stations")
    );
    let _ = wire(h.weather.report(seattle()).await);
    assert_eq!(
        h.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.as_str() == "/points/47.61,-122.33")
            .count(),
        2,
        "partial grid discovery must be retried instead of cached for six hours"
    );
}
#[tokio::test]
async fn alerts_survive_points_and_forecast_outages() {
    for faults in [
        Faults {
            points_fail: true,
            ..Default::default()
        },
        Faults {
            forecast_fail: true,
            ..Default::default()
        },
    ] {
        let h = fixture(faults).await;
        assert_eq!(
            h.weather.report(seattle()).await.unwrap_err().code,
            ErrorCode::UpstreamUnavailable
        );
        let alerts = wire(h.weather.alerts(seattle()).await);
        assert_eq!(alerts["alertsStatus"], "checked");
        assert_eq!(alerts["location"]["name"], "Seattle, WA");
        assert_eq!(alerts["alerts"][0]["event"], "Wind Advisory");
        assert_eq!(
            alerts["alerts"][0]["instruction"],
            "Secure outdoor objects."
        );
        for key in [
            "units",
            "sources",
            "warnings",
            "assembledAt",
            "cacheMaxAgeSeconds",
        ] {
            assert!(!alerts[key].is_null(), "missing {key}");
        }
    }
}
#[tokio::test]
async fn links_and_sources_use_the_configured_nws_base_url() {
    let h = fixture(Faults::default()).await;
    let report = wire(h.weather.report(seattle()).await);
    for url in [
        &report["sources"]["forecast"]["url"],
        &report["sources"]["hourly"]["url"],
        &report["sources"]["alerts"]["url"],
        &report["current"]["sourceUrl"],
    ] {
        let url = url.as_str().unwrap();
        assert!(url.starts_with(&format!("{}/", h.base)), "{url}");
    }
    // The default origin is not trusted once another base URL is configured.
    let configured = Weather::configured(Config {
        nws_base_url: h.base.clone(),
        ..Config::default()
    })
    .unwrap();
    assert!(configured.report(seattle()).await.is_ok());
    assert_eq!(Config::default().nws_base_url, "https://api.weather.gov");
    for invalid in [
        "ftp://example.com",
        "https://example.com/v1",
        "https://example.com?x=1",
        "not a url",
    ] {
        let config = Config {
            nws_base_url: invalid.into(),
            ..Config::default()
        };
        assert!(Weather::configured(config).is_err(), "{invalid}");
    }
}
#[tokio::test]
async fn alerts_need_only_the_alerts_request() {
    let h = fixture(Faults::default()).await;
    h.weather.alerts(seattle()).await.unwrap();
    assert_eq!(
        *h.requests.lock().unwrap(),
        ["/alerts/active?point=47.6062,-122.3321"]
    );
}
#[tokio::test]
async fn hourly_needs_only_points_and_hourly() {
    let h = fixture(Faults {
        alerts_fail: true,
        forecast_fail: true,
        ..Default::default()
    })
    .await;
    let hourly = wire(h.weather.hourly(seattle()).await);
    assert_eq!(hourly["hourly"][0]["condition"], "Sunny");
    assert_eq!(hourly["alertsStatus"], "not-checked");
    assert_eq!(
        *h.requests.lock().unwrap(),
        ["/points/47.61,-122.33", "/hourly"]
    );
}
#[tokio::test]
async fn hourly_does_not_require_the_daily_forecast_link() {
    let h = fixture(Faults {
        forecast_link_missing: true,
        ..Default::default()
    })
    .await;
    let hourly = wire(h.weather.hourly(seattle()).await);
    assert_eq!(hourly["hourly"][0]["condition"], "Sunny");
    assert_eq!(hourly["alertsStatus"], "not-checked");
    assert_eq!(
        *h.requests.lock().unwrap(),
        ["/points/47.61,-122.33", "/hourly"]
    );
    assert_eq!(
        h.weather.report(seattle()).await.unwrap_err().code,
        ErrorCode::UpstreamUnavailable
    );
    assert_eq!(
        h.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.as_str() == "/points/47.61,-122.33")
            .count(),
        2,
        "incomplete discovery must be retried instead of cached for six hours"
    );
}
#[tokio::test]
async fn document_cache_keys_include_the_requested_type() {
    let h = fixture(Faults {
        shared_hourly_stations_link: true,
        ..Default::default()
    })
    .await;
    let report = wire(h.weather.report(seattle()).await);
    assert_eq!(report["forecast"][0]["condition"], "Sunny");
    assert_eq!(report["hourly"][0]["condition"], "Sunny");
    assert!(report["current"].is_null());
    assert!(
        report["warnings"]
            .to_string()
            .contains("Observation stations")
    );
    assert_eq!(
        h.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.as_str() == "/hourly")
            .count(),
        2,
        "forecast and stations fetches sharing a URL must validate independently"
    );
}
#[tokio::test]
async fn outside_coverage_is_typed_for_alerts_too() {
    let h = fixture(Faults::default()).await;
    let error = h.weather.alerts(coordinates(0., 0.)).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::OutsideCoverage);
    assert_eq!(error.code.http_status(), 422);
}
#[tokio::test]
async fn points_404_is_outside_coverage_for_reports_and_hourly() {
    let h = fixture(Faults::default()).await;
    let report = h.weather.report(coordinates(0., 0.)).await.unwrap_err();
    let hourly = h.weather.hourly(coordinates(0., 0.)).await.unwrap_err();
    for error in [report, hourly] {
        assert_eq!(error.code, ErrorCode::OutsideCoverage);
        assert_eq!(error.code.http_status(), 422);
    }
}
#[tokio::test]
async fn only_points_404_means_outside_coverage() {
    let h = fixture(Faults {
        forecast_missing: true,
        ..Default::default()
    })
    .await;
    let error = h.weather.report(seattle()).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamUnavailable);
    assert_eq!(error.code.http_status(), 502);
}
#[tokio::test]
async fn nearby_coordinates_share_one_rounded_upstream_point() {
    let h = fixture(Faults::default()).await;
    let first = wire(h.weather.report(coordinates(47.6062, -122.3321)).await);
    let hits = h.hits.load(Ordering::SeqCst);
    let nudged = wire(h.weather.report(coordinates(47.6131, -122.3274)).await);
    // Grid lookup, forecasts and observations are reused; only the precise alerts check is new.
    assert_eq!(h.hits.load(Ordering::SeqCst), hits + 1);
    assert_eq!(
        h.requests.lock().unwrap().last().unwrap(),
        "/alerts/active?point=47.6131,-122.3274"
    );
    assert_eq!(first["location"]["latitude"], 47.6062);
    assert_eq!(first["location"]["precision"], "coordinates");
    for report in [&first, &nudged] {
        assert_eq!(
            report["location"]["gridLookupPoint"],
            json!({"latitude":47.61,"longitude":-122.33})
        );
    }
}
#[tokio::test]
async fn alerts_use_the_precise_point_while_grid_lookup_is_rounded() {
    let h = fixture(Faults::default()).await;
    h.weather
        .report(coordinates(47.6131, -122.3274))
        .await
        .unwrap();
    h.weather
        .alerts(coordinates(47.6131, -122.3274))
        .await
        .unwrap();
    let requests = h.requests.lock().unwrap().clone();
    assert!(requests.contains(&"/points/47.61,-122.33".to_string()));
    let alerts: Vec<_> = requests
        .iter()
        .filter(|r| r.starts_with("/alerts/"))
        .collect();
    // Alerts retain four decimals instead of sharing the two-decimal grid point.
    assert_eq!(alerts, ["/alerts/active?point=47.6131,-122.3274"]);
}
#[tokio::test]
async fn uncached_report_is_not_serialized_by_the_limiter() {
    let h = fixture(Faults::default()).await;
    let started = Instant::now();
    h.weather.report(seattle()).await.unwrap();
    // Six upstream requests fit in the burst. The old limiter spaced starts one second apart,
    // so it needed at least five seconds; three seconds leaves headroom for a loaded runner.
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}
/// Status and Cache-Control for one GET through the HTTP app backed by the fixture.
async fn http_get(weather: &Arc<Weather>, path: &str) -> (u16, String) {
    use tower::ServiceExt;
    let app = weather_bridge::api::app(weather.clone(), weather_bridge::api::HttpConfig::default());
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri(path)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let cache = response
        .headers()
        .get("cache-control")
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default();
    (response.status().as_u16(), cache)
}
const SEATTLE: &str = "city=Seattle%2C%20WA";
/// A complete response may be cached only for the remaining life of its oldest source:
/// `public, max-age=N, s-maxage=N` with N at most the 120-second freshness window.
fn assert_fresh_for_at_most_two_minutes((status, cache): (u16, String), path: &str) {
    assert_eq!(status, 200, "{path}");
    let seconds: u64 = cache
        .strip_prefix("public, max-age=")
        .and_then(|rest| rest.split_once(", s-maxage="))
        .filter(|(max_age, s_maxage)| max_age == s_maxage)
        .and_then(|(max_age, _)| max_age.parse().ok())
        .unwrap_or_else(|| panic!("{path}: unexpected cache-control {cache:?}"));
    assert!((100..=120).contains(&seconds), "{path}: {cache}");
}
#[tokio::test]
async fn complete_reports_are_briefly_cacheable() {
    let h = fixture(Faults::default()).await;
    for route in ["/v1/weather", "/v1/forecast/hourly", "/v1/alerts"] {
        let path = format!("{route}?{SEATTLE}");
        assert_fresh_for_at_most_two_minutes(http_get(&h.weather, &path).await, &path);
    }
}
#[tokio::test]
async fn degraded_reports_and_upstream_failures_are_never_stored() {
    let h = fixture(Faults {
        alerts_fail: true,
        ..Default::default()
    })
    .await;
    for route in ["/v1/weather", "/v1/alerts"] {
        let path = format!("{route}?{SEATTLE}");
        assert_eq!(
            http_get(&h.weather, &path).await,
            (200, "no-store".into()),
            "{path}"
        );
    }
    let h = fixture(Faults {
        forecast_fail: true,
        ..Default::default()
    })
    .await;
    let path = format!("/v1/weather?{SEATTLE}");
    assert_eq!(http_get(&h.weather, &path).await, (502, "no-store".into()));
    let path = format!("/v1/alerts?{SEATTLE}");
    assert_fresh_for_at_most_two_minutes(http_get(&h.weather, &path).await, &path);
    let h = fixture(Faults::default()).await;
    assert_eq!(
        http_get(&h.weather, "/v1/weather?lat=0&lon=0").await,
        (422, "no-store".into())
    );

    let h = fixture(Faults {
        hourly_missing_periods: true,
        stations_missing_features: true,
        ..Default::default()
    })
    .await;
    let path = format!("/v1/weather?{SEATTLE}");
    assert_eq!(http_get(&h.weather, &path).await, (200, "no-store".into()));
    let report = wire(h.weather.report(seattle()).await);
    assert_eq!(report["forecast"][0]["condition"], "Sunny");
    assert_eq!(report["hourly"], json!([]));
    assert!(report["warnings"].to_string().contains("Hourly forecast"));
    assert!(
        report["warnings"]
            .to_string()
            .contains("Observation stations")
    );
}

#[tokio::test]
async fn malformed_alert_collections_are_unavailable_and_never_cached() {
    for body in [
        "{}",
        "null",
        "[]",
        r#"{"features":null}"#,
        r#"{"features":{}}"#,
        r#"{"features":[null]}"#,
        r#"{"features":[{}]}"#,
        r#"{"features":[{"properties":{}}]}"#,
        r#"{"features":[{"properties":{"event":42}}]}"#,
        r#"{"features":[{"properties":{"event":" "}}]}"#,
        r#"{"features":[{"properties":{"event":"Wind Advisory","instruction":42}}]}"#,
    ] {
        let h = fixture(Faults {
            alerts_body: Some(body),
            ..Default::default()
        })
        .await;
        for result in [
            wire(h.weather.alerts(seattle()).await),
            wire(h.weather.report(seattle()).await),
        ] {
            assert_eq!(result["alertsStatus"], "unavailable", "{body}");
            assert_eq!(result["alerts"], json!([]), "{body}");
            assert!(
                result["warnings"]
                    .to_string()
                    .contains("could not be checked"),
                "{body}"
            );
        }
        for route in ["/v1/weather", "/v1/alerts"] {
            assert_eq!(
                http_get(&h.weather, &format!("{route}?{SEATTLE}")).await,
                (200, "no-store".into()),
                "{body}"
            );
        }
        assert_eq!(
            h.requests
                .lock()
                .unwrap()
                .iter()
                .filter(|p| p.starts_with("/alerts/"))
                .count(),
            4,
            "Malformed alerts must be retried, not cached: {body}"
        );
    }
}

#[tokio::test]
async fn valid_empty_alert_collection_is_checked_and_cacheable() {
    let h = fixture(Faults {
        alerts_body: Some(r#"{"features":[]}"#),
        ..Default::default()
    })
    .await;
    let alerts = wire(h.weather.alerts(seattle()).await);
    assert_eq!(alerts["alertsStatus"], "checked");
    assert_eq!(alerts["alerts"], json!([]));
    assert_eq!(alerts["warnings"], json!([]));
    assert_fresh_for_at_most_two_minutes(
        http_get(&h.weather, &format!("/v1/alerts?{SEATTLE}")).await,
        "alerts",
    );
    assert_eq!(h.hits.load(Ordering::SeqCst), 1);
}
/// Slow but successful required sources plus hung observation stations must still yield
/// the forecast. Paused time lets the fixture's delays and the timeouts run instantly.
#[tokio::test(start_paused = true)]
async fn slow_observation_stations_degrade_the_report_instead_of_timing_it_out() {
    let h = fixture(Faults {
        required_delay: Duration::from_secs(8),
        observation_delay: Duration::from_secs(3600),
        three_stations: true,
        ..Default::default()
    })
    .await;
    let _clock = common::small_clock_steps();
    let started = tokio::time::Instant::now();
    let report = wire(h.weather.report(seattle()).await);
    assert_eq!(report["forecast"][0]["condition"], "Sunny");
    assert!(report["current"].is_null(), "never make up an observation");
    assert!(
        report["warnings"]
            .to_string()
            .contains("No recent station observation"),
        "{}",
        report["warnings"]
    );
    assert!(
        started.elapsed() < Duration::from_secs(45),
        "{:?}",
        started.elapsed()
    );
}
/// Start `count` reports at once, each for its own grid point so none shares a cached
/// document, and return each outcome with how long it took.
async fn cold_reports(count: u32) -> (common::Harness, Vec<(Result<Value, ErrorCode>, Duration)>) {
    let h = fixture(Faults {
        every_point_covered: true,
        fresh_observation: true,
        ..Default::default()
    })
    .await;
    let _clock = common::small_clock_steps();
    let lookups: Vec<_> = (0..count)
        .map(|i| {
            let weather = h.weather.clone();
            tokio::spawn(async move {
                let started = tokio::time::Instant::now();
                let result = weather
                    .report(coordinates(40.0 + f64::from(i), -100.0))
                    .await;
                let result = result
                    .map(|report| serde_json::to_value(report).unwrap())
                    .map_err(|e| e.code);
                (result, started.elapsed())
            })
        })
        .collect();
    let mut outcomes = Vec::new();
    for lookup in lookups {
        outcomes.push(lookup.await.unwrap());
    }
    (h, outcomes)
}
/// More cold lookups than upstream pacing can serve: the overflow is refused as BUSY
/// within seconds instead of queueing until the lookup deadline.
#[tokio::test(start_paused = true)]
async fn cold_burst_beyond_upstream_pacing_is_refused_quickly_as_busy() {
    let (h, outcomes) = cold_reports(8).await;
    let mut busy = 0;
    let mut succeeded = 0;
    for (result, elapsed) in outcomes {
        match result {
            Ok(_) => succeeded += 1,
            Err(ErrorCode::Busy) => {
                busy += 1;
                // Refused by the end of the 6 s pacing wait window, plus loopback time.
                assert!(elapsed < Duration::from_secs(7), "slow BUSY: {elapsed:?}");
            }
            Err(code) => panic!("overload must be BUSY, not {code:?} after {elapsed:?}"),
        }
        assert!(elapsed < Duration::from_secs(20), "{elapsed:?}");
    }
    assert!(succeeded > 0, "pacing still serves the oldest cold lookups");
    assert!(busy > 0, "the overflow must be refused");
    assert!(
        metrics_text(&h.weather)
            .await
            .contains(&format!("weather_bridge_busy_total {busy}\n")),
        "every refused lookup is counted once"
    );
}
/// Refusing overload must not cost modest concurrency: two cold reports at once fit the
/// pacing burst and window, so both are served in full.
#[tokio::test(start_paused = true)]
async fn two_cold_reports_at_once_are_both_served_in_full() {
    let (_h, outcomes) = cold_reports(2).await;
    for (result, elapsed) in outcomes {
        let report = result.unwrap_or_else(|code| panic!("{code:?} after {elapsed:?}"));
        assert_eq!(report["warnings"], json!([]), "{elapsed:?}");
        assert_eq!(report["current"]["station"], "NEAR");
    }
}
async fn metrics_text(weather: &Arc<Weather>) -> String {
    use tower::ServiceExt;
    let app = weather_bridge::api::app(weather.clone(), weather_bridge::api::HttpConfig::default());
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/metrics")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}
