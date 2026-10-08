//! Offline development fixture: scenario behavior through the real HTTP and MCP app.
mod common;

use common::{DEV_FIXTURE_HEADER, DevScenario, dev_app, dev_fixture};
use serde_json::{Value, json};

async fn get_json(
    client: &reqwest::Client,
    base: &str,
    path: &str,
) -> (reqwest::StatusCode, Value) {
    let response = client.get(format!("{base}{path}")).send().await.unwrap();
    assert_eq!(
        response.headers()[DEV_FIXTURE_HEADER],
        "synthetic-offline-data"
    );
    let status = response.status();
    (status, response.json().await.unwrap())
}

async fn running(scenario: DevScenario) -> (common::Harness, String, tokio::task::JoinHandle<()>) {
    let fixture = dev_fixture(scenario).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = dev_app(fixture.weather.clone(), scenario);
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (fixture, base, task)
}

#[tokio::test]
async fn scenarios_are_explicit_and_reachable_over_http() {
    let client = reqwest::Client::new();
    for (scenario, stale, alerts_status) in [
        (DevScenario::Healthy, false, "checked"),
        (DevScenario::Stale, true, "checked"),
        (DevScenario::AlertsUnavailable, false, "unavailable"),
    ] {
        let (_fixture, base, task) = running(scenario).await;
        let page = client.get(&base).send().await.unwrap();
        assert_eq!(page.headers()[DEV_FIXTURE_HEADER], "synthetic-offline-data");
        let html = page.text().await.unwrap();
        assert!(html.contains("SYNTHETIC OFFLINE FIXTURE"), "{scenario:?}");
        assert!(html.contains(scenario.as_str()), "{scenario:?}");
        assert!(html.contains("value=\"Seattle, WA\""));
        let developer = client
            .get(format!("{base}/developer"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(developer.contains("SYNTHETIC OFFLINE FIXTURE"));

        let (status, report) = get_json(&client, &base, "/v1/weather?city=Seattle%2C%20WA").await;
        assert_eq!(status, reqwest::StatusCode::OK, "{scenario:?}");
        assert_eq!(report["data"]["current"]["stale"], stale, "{scenario:?}");
        assert_eq!(
            report["data"]["alertsStatus"], alerts_status,
            "{scenario:?}"
        );
        task.abort();
    }
}

#[tokio::test]
async fn development_fixture_disables_static_and_api_caching() {
    let (_fixture, base, task) = running(DevScenario::Healthy).await;
    let client = reqwest::Client::new();
    for path in ["/", "/assets/weather.js", "/v1/cities?q=Seattle"] {
        let response = client.get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(response.headers()["cache-control"], "no-store", "{path}");
    }
    task.abort();
}

#[tokio::test]
async fn fixture_exposes_ambiguity_and_mcp_without_network_access() {
    let (_fixture, base, task) = running(DevScenario::Healthy).await;
    let client = reqwest::Client::new();
    let (status, ambiguity) = get_json(&client, &base, "/v1/weather?city=Springfield").await;
    assert_eq!(status, reqwest::StatusCode::CONFLICT);
    assert_eq!(ambiguity["errors"][0]["code"], "AMBIGUOUS_CITY");
    assert!(ambiguity["errors"][0]["choices"].as_array().unwrap().len() > 1);

    let response = client
        .post(format!("{base}/mcp"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-11-25")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": "get_weather", "arguments": {"city": "Seattle, WA"}}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.headers()[DEV_FIXTURE_HEADER],
        "synthetic-offline-data"
    );
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        body["result"]["structuredContent"]["data"]["current"]["stale"],
        false
    );
    task.abort();
}
