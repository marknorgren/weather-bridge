//! Contract test: real responses from the loopback fixture must validate against the
//! response schemas in `openapi.json`, and the OpenAPI component schemas must list the
//! same fields as the Rust response types.
//!
//! The validator is a small, fail-closed JSON Schema subset (see `Validator`) rather than
//! the `jsonschema` crate. That crate adds about 40 dev-dependency crates and, under the
//! `cargo deny` all-features graph, a Zlib-licensed crate outside the allowlist. The
//! subset covers every keyword `openapi.json` and the schemars output use; any other
//! keyword fails the test instead of being ignored.
mod common;

use axum::http::Request;
use common::{Faults, Harness, fixture};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tower::ServiceExt;
use weather_bridge::{
    Error, ErrorCode,
    api::{HttpConfig, OriginVerify, app},
    cities::Cities as CityIndex,
    mcp::output_schema,
    model::{ActiveAlerts, Cities, HourlyForecast, WeatherReport},
    weather::Weather,
};

fn spec() -> Value {
    serde_json::from_str(include_str!("../openapi.json")).unwrap()
}

/// Keywords that only annotate; they never affect validity.
const ANNOTATIONS: &[&str] = &[
    "description",
    "title",
    "format",
    "example",
    "examples",
    "default",
    "deprecated",
    "readOnly",
    "$schema",
    "$defs",
];

/// Validates an instance against a schema in `root` (an OpenAPI document or a standalone
/// JSON Schema). Local `$ref`s only. Unsupported keywords are reported as errors.
struct Validator<'a> {
    root: &'a Value,
}
impl<'a> Validator<'a> {
    fn resolve(&self, reference: &str) -> &'a Value {
        let pointer = reference
            .strip_prefix('#')
            .unwrap_or_else(|| panic!("only local $ref is supported: {reference}"));
        self.root
            .pointer(pointer)
            .unwrap_or_else(|| panic!("unresolved $ref {reference}"))
    }
    fn errors(&self, schema: &Value, instance: &Value) -> Vec<String> {
        let mut errors = Vec::new();
        self.check(schema, instance, "$", &mut errors);
        errors
    }
    fn check(&self, schema: &Value, instance: &Value, at: &str, errors: &mut Vec<String>) {
        let schema = match schema {
            Value::Bool(true) => return,
            Value::Bool(false) => return errors.push(format!("{at}: undocumented response")),
            Value::Object(schema) => schema,
            other => panic!("schema at {at} is not an object: {other}"),
        };
        for (keyword, value) in schema {
            match keyword.as_str() {
                "$ref" => self.check(self.resolve(value.as_str().unwrap()), instance, at, errors),
                "type" => {
                    let types: Vec<&str> = match value {
                        Value::String(t) => vec![t],
                        Value::Array(ts) => ts.iter().map(|t| t.as_str().unwrap()).collect(),
                        other => panic!("bad type at {at}: {other}"),
                    };
                    if !types.iter().any(|t| has_type(instance, t)) {
                        errors.push(format!("{at}: expected {types:?}, got {instance}"));
                    }
                }
                "enum" => {
                    if !value.as_array().unwrap().contains(instance) {
                        errors.push(format!("{at}: {instance} not in enum {value}"));
                    }
                }
                "const" => {
                    if value != instance {
                        errors.push(format!("{at}: {instance} is not const {value}"));
                    }
                }
                "properties" => {
                    if let Some(object) = instance.as_object() {
                        for (name, sub) in value.as_object().unwrap() {
                            if let Some(field) = object.get(name) {
                                self.check(sub, field, &format!("{at}.{name}"), errors);
                            }
                        }
                    }
                }
                "required" => {
                    if let Some(object) = instance.as_object() {
                        for name in value.as_array().unwrap() {
                            let name = name.as_str().unwrap();
                            if !object.contains_key(name) {
                                errors.push(format!("{at}: missing required field {name}"));
                            }
                        }
                    }
                }
                "additionalProperties" => {
                    let Some(object) = instance.as_object() else {
                        continue;
                    };
                    let declared = schema.get("properties").and_then(Value::as_object);
                    for (name, field) in object {
                        if declared.is_some_and(|d| d.contains_key(name)) {
                            continue;
                        }
                        match value {
                            Value::Bool(false) => {
                                errors.push(format!("{at}: undocumented field {name}"));
                            }
                            sub => self.check(sub, field, &format!("{at}.{name}"), errors),
                        }
                    }
                }
                "items" => {
                    for (i, item) in instance.as_array().into_iter().flatten().enumerate() {
                        self.check(value, item, &format!("{at}[{i}]"), errors);
                    }
                }
                "minItems" | "maxItems" => {
                    if let Some(len) = instance.as_array().map(Vec::len) {
                        let bound = value.as_u64().unwrap() as usize;
                        let ok = if keyword == "minItems" {
                            len >= bound
                        } else {
                            len <= bound
                        };
                        if !ok {
                            errors.push(format!("{at}: {len} items violates {keyword} {bound}"));
                        }
                    }
                }
                "minimum" | "maximum" => {
                    if let Some(n) = instance.as_f64() {
                        let bound = value.as_f64().unwrap();
                        let ok = if keyword == "minimum" {
                            n >= bound
                        } else {
                            n <= bound
                        };
                        if !ok {
                            errors.push(format!("{at}: {n} violates {keyword} {bound}"));
                        }
                    }
                }
                "anyOf" | "oneOf" => {
                    let branches: Vec<Vec<String>> = value
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|sub| self.errors_at(sub, instance, at))
                        .collect();
                    let passing = branches.iter().filter(|e| e.is_empty()).count();
                    if passing == 0 {
                        // Report the closest branch whose type fits, so a nullable object
                        // shows its field errors rather than "expected null".
                        let type_miss = format!("{at}: expected");
                        let closest = branches
                            .into_iter()
                            .min_by_key(|e| (e.iter().any(|m| m.starts_with(&type_miss)), e.len()))
                            .unwrap_or_default();
                        errors.extend(closest);
                    } else if keyword == "oneOf" && passing > 1 {
                        errors.push(format!("{at}: {passing} oneOf branches match"));
                    }
                }
                k if ANNOTATIONS.contains(&k) => {}
                other => errors.push(format!("{at}: unsupported schema keyword {other}")),
            }
        }
    }
    fn errors_at(&self, schema: &Value, instance: &Value, at: &str) -> Vec<String> {
        let mut errors = Vec::new();
        self.check(schema, instance, at, &mut errors);
        errors
    }
}
fn has_type(instance: &Value, t: &str) -> bool {
    match t {
        "null" => instance.is_null(),
        "boolean" => instance.is_boolean(),
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "string" => instance.is_string(),
        "number" => instance.is_number(),
        "integer" => instance.is_i64() || instance.is_u64(),
        other => panic!("unknown type {other}"),
    }
}

/// The JSON schema documented for one operation's response status. An undocumented status
/// yields the `false` schema, which rejects every response.
fn response_schema<'a>(spec: &'a Value, route: &str, status: u16) -> &'a Value {
    static UNDOCUMENTED: Value = Value::Bool(false);
    let validator = Validator { root: spec };
    let Some(mut response) = spec["paths"][route]["get"]["responses"].get(status.to_string())
    else {
        return &UNDOCUMENTED;
    };
    while let Some(reference) = response.get("$ref").and_then(Value::as_str) {
        response = validator.resolve(reference);
    }
    &response["content"]["application/json"]["schema"]
}

struct Exchange {
    route: &'static str,
    status: u16,
    body: Value,
    label: String,
}
async fn get(weather: &Arc<Weather>, path: &'static str, verify: Option<OriginVerify>) -> Exchange {
    exchange(weather, path, verify, Vec::new()).await
}

async fn exchange(
    weather: &Arc<Weather>,
    path: &'static str,
    verify: Option<OriginVerify>,
    body: Vec<u8>,
) -> Exchange {
    let config = HttpConfig {
        origin_verify: verify,
        ..HttpConfig::default()
    };
    let response = app(weather.clone(), config)
        .oneshot(
            Request::get(path)
                .header("content-length", body.len())
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Exchange {
        route: path.split('?').next().unwrap(),
        status,
        body: serde_json::from_slice(&bytes).unwrap(),
        label: format!("GET {path} -> {status}"),
    }
}

/// Every REST response class the fixture can produce, with the status it must have.
async fn exchanges() -> Vec<Exchange> {
    const SEATTLE: &str = "/v1/weather?city=Seattle%2C%20WA";
    let mut out = Vec::new();
    let h = fixture(Faults::default()).await;
    for path in [
        SEATTLE,
        "/v1/forecast/hourly?city=Seattle%2C%20WA",
        "/v1/alerts?city=Seattle%2C%20WA",
        "/v1/cities?q=Seattle",
    ] {
        out.push(expect(
            exchange(&h.weather, path, None, vec![b'x'; 16385]).await,
            413,
        ));
    }
    for (path, status) in [
        (SEATTLE, 200),
        ("/v1/weather?lat=47.6062&lon=-122.3321&units=metric", 200),
        ("/v1/forecast/hourly?city=Seattle%2C%20WA", 200),
        ("/v1/alerts?city=Seattle%2C%20WA", 200),
        ("/v1/cities?q=Springfield", 200),
        ("/v1/cities?q=a", 400),
        ("/v1/weather?city=Seattle&units=banana", 400),
        ("/v1/forecast/hourly?city=Seattle&lat=1&lon=1", 400),
        ("/v1/alerts?lat=1", 400),
        ("/v1/weather?city=Seattl", 404),
        ("/v1/weather?cityId=1", 404),
        ("/v1/weather?city=Springfield", 409),
        ("/v1/alerts?city=Springfield", 409),
        ("/v1/forecast/hourly?city=Springfield", 409),
        ("/v1/weather?lat=0&lon=0", 422),
        ("/v1/alerts?lat=0&lon=0", 422),
    ] {
        out.push(expect(get(&h.weather, path, None).await, status));
    }
    for (faults, path, status) in [
        (alerts_fail(), SEATTLE, 200),
        (alerts_fail(), "/v1/alerts?city=Seattle%2C%20WA", 200),
        (stations_fail(), SEATTLE, 200),
        (forecast_fail(), SEATTLE, 502),
        (
            points_fail(),
            "/v1/forecast/hourly?city=Seattle%2C%20WA",
            502,
        ),
    ] {
        let h: Harness = fixture(faults).await;
        out.push(expect(get(&h.weather, path, None).await, status));
    }
    let verify = OriginVerify::new("0123456789abcdefghijklmnopqrstuvwxyzABCD").unwrap();
    out.push(expect(get(&h.weather, SEATTLE, Some(verify)).await, 403));
    out
}
fn expect(exchange: Exchange, status: u16) -> Exchange {
    assert_eq!(
        exchange.status, status,
        "{}: {}",
        exchange.label, exchange.body
    );
    exchange
}
fn alerts_fail() -> Faults {
    Faults {
        alerts_fail: true,
        ..Default::default()
    }
}
fn stations_fail() -> Faults {
    Faults {
        stations_fail: true,
        ..Default::default()
    }
}
fn forecast_fail() -> Faults {
    Faults {
        forecast_fail: true,
        ..Default::default()
    }
}
fn points_fail() -> Faults {
    Faults {
        points_fail: true,
        ..Default::default()
    }
}

#[tokio::test]
async fn operator_origin_errors_match_openapi() {
    let spec = spec();
    let validator = Validator { root: &spec };
    let weather = Arc::new(Weather::new().unwrap());
    for path in ["/version", "/metrics"] {
        let verify = OriginVerify::new("0123456789abcdefghijklmnopqrstuvwxyzABCD").unwrap();
        let response = expect(get(&weather, path, Some(verify)).await, 403);
        let errors = validator.errors(response_schema(&spec, path, 403), &response.body);
        assert!(errors.is_empty(), "{path}: {errors:?}");
    }
}

#[tokio::test]
async fn fixture_responses_match_openapi() {
    let spec = spec();
    let validator = Validator { root: &spec };
    let mut failures = Vec::new();
    let exchanges = exchanges().await;
    for exchange in &exchanges {
        let schema = response_schema(&spec, exchange.route, exchange.status);
        for error in validator.errors(schema, &exchange.body) {
            failures.push(format!("{}: {error}", exchange.label));
        }
    }
    // The responses must include the shapes that only appear on degraded paths.
    let bodies: Vec<String> = exchanges.iter().map(|e| e.body.to_string()).collect();
    for needle in [
        r#""alertsStatus":"unavailable""#,
        r#""alertsStatus":"not-checked""#,
        r#""current":null"#,
        r#""choices":null"#,
        r#""choices":[{"#,
        r#""code":"FORBIDDEN""#,
        r#""code":"REQUEST_TOO_LARGE""#,
    ] {
        assert!(
            bodies.iter().any(|b| b.contains(needle)),
            "no response has {needle}"
        );
    }
    assert!(
        failures.is_empty(),
        "contract drift:\n{}",
        failures.join("\n")
    );
}

#[test]
fn every_error_code_matches_its_documented_response() {
    let spec = spec();
    let validator = Validator { root: &spec };
    let mut failures = Vec::new();
    for code in ErrorCode::ALL {
        let body = Error::new(code, "detail").body();
        let status = code.http_status();
        for route in ["/v1/weather", "/v1/forecast/hourly", "/v1/alerts"] {
            let schema = response_schema(&spec, route, status);
            for error in validator.errors(schema, &body) {
                failures.push(format!("{route} {status} {code:?}: {error}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "contract drift:\n{}",
        failures.join("\n")
    );
}

/// Response schemas must be closed: every object lists `additionalProperties: false`
/// and requires every property, so an added, removed or renamed field on either side
/// fails `fixture_responses_match_openapi`.
#[test]
fn openapi_response_schemas_are_closed() {
    const OPTIONAL: &[&str] = &["Attribution.license"];
    let spec = spec();
    let mut failures = Vec::new();
    let mut seen = BTreeSet::new();
    for (route, item) in spec["paths"].as_object().unwrap() {
        for (status, _) in item["get"]["responses"].as_object().unwrap() {
            let schema = response_schema(&spec, route, status.parse().unwrap());
            closed(&spec, schema, route, &mut seen, &mut failures, OPTIONAL);
        }
    }
    assert!(
        failures.is_empty(),
        "open schemas:\n{}",
        failures.join("\n")
    );
}
fn closed(
    spec: &Value,
    schema: &Value,
    name: &str,
    seen: &mut BTreeSet<String>,
    failures: &mut Vec<String>,
    optional: &[&str],
) {
    let Some(object) = schema.as_object() else {
        return;
    };
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        if seen.insert(reference.to_owned()) {
            let target = Validator { root: spec }.resolve(reference);
            let name = reference.rsplit('/').next().unwrap();
            closed(spec, target, name, seen, failures, optional);
        }
        return;
    }
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        if object.get("additionalProperties") != Some(&Value::Bool(false)) {
            failures.push(format!("{name}: missing additionalProperties: false"));
        }
        let required: BTreeSet<&str> = object
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        for property in properties.keys() {
            let qualified = format!("{name}.{property}");
            if !required.contains(property.as_str()) && !optional.contains(&qualified.as_str()) {
                failures.push(format!("{qualified}: not required"));
            }
        }
        for (property, sub) in properties {
            closed(
                spec,
                sub,
                &format!("{name}.{property}"),
                seen,
                failures,
                optional,
            );
        }
    } else if object.get("type") == Some(&json!("object")) {
        failures.push(format!("{name}: object without properties"));
    }
    for key in ["items", "additionalProperties"] {
        if let Some(sub) = object.get(key) {
            closed(spec, sub, name, seen, failures, optional);
        }
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        for sub in object
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            closed(spec, sub, name, seen, failures, optional);
        }
    }
}

/// The schemars schemas of the Rust response types (as advertised to MCP clients) and the
/// generated OpenAPI components must name the same fields and enum values.
#[test]
fn rust_types_and_openapi_components_agree() {
    let mut defs = BTreeMap::new();
    for schema in [
        output_schema::<WeatherReport>(),
        output_schema::<HourlyForecast>(),
        output_schema::<ActiveAlerts>(),
        output_schema::<Cities>(),
    ] {
        for (name, def) in schema["$defs"].as_object().unwrap() {
            defs.insert(name.clone(), def.clone());
        }
    }
    let spec = spec();
    let components = spec["components"]["schemas"]
        .as_object()
        .expect("openapi.json components.schemas");
    let mut failures = Vec::new();
    for (name, def) in &defs {
        let Some(component) = components.get(name) else {
            failures.push(format!("{name}: no OpenAPI component"));
            continue;
        };
        let (rust, openapi) = (shape(def), shape(component));
        if rust != openapi {
            failures.push(format!("{name}: Rust {rust:?} != OpenAPI {openapi:?}"));
        }
    }
    for (name, component) in components {
        let envelope = name.ends_with("Envelope");
        if !envelope && !defs.contains_key(name) && component.get("properties").is_some() {
            failures.push(format!("{name}: no Rust type"));
        }
    }
    assert!(failures.is_empty(), "type drift:\n{}", failures.join("\n"));
}
/// Property names for objects, allowed values for string enums.
fn shape(schema: &Value) -> BTreeSet<String> {
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        return properties.keys().cloned().collect();
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return values
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
    }
    if let Some(variants) = schema.get("oneOf").and_then(Value::as_array) {
        return variants
            .iter()
            .map(|v| v["const"].as_str().unwrap().to_owned())
            .collect();
    }
    BTreeSet::new()
}

/// MCP structuredContent must validate against the outputSchema the tool advertises.
#[tokio::test]
async fn mcp_structured_content_matches_advertised_output_schema() {
    let h = fixture(Faults::default()).await;
    let list = mcp(&h.weather, "tools/list", json!({})).await;
    let schemas: BTreeMap<String, Value> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().into(),
                t["outputSchema"].clone(),
            )
        })
        .collect();
    let seattle = json!({"city": "Seattle, WA"});
    let mut failures = Vec::new();
    for (tool, arguments, is_error) in [
        ("get_weather", seattle.clone(), false),
        ("get_hourly_forecast", seattle.clone(), false),
        ("get_active_alerts", seattle, false),
        ("search_cities", json!({"query": "Springfield"}), false),
        ("get_weather", json!({"city": "Springfield"}), true),
        ("search_cities", json!({"query": "a"}), true),
    ] {
        let call = json!({"name": tool, "arguments": arguments});
        let result = &mcp(&h.weather, "tools/call", call).await["result"];
        assert_eq!(result["isError"], is_error, "{tool}: {result}");
        let schema = &schemas[tool];
        for error in (Validator { root: schema }).errors(schema, &result["structuredContent"]) {
            failures.push(format!("{tool}: {error}"));
        }
    }
    assert!(failures.is_empty(), "MCP drift:\n{}", failures.join("\n"));
}
/// Every advertised city-name input (REST `q` and `city`, MCP `query` and `city`) must
/// carry the same length bounds the runtime enforces, so clients can validate before
/// sending.
#[tokio::test]
async fn advertised_city_query_bounds_match_runtime_limits() {
    let index = CityIndex::load().unwrap();
    let cases: Value =
        serde_json::from_str(include_str!("fixtures/city-query-inputs.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let query = case["query"].as_str().unwrap();
        let accepted = !index
            .search(query)
            .is_err_and(|error| error.code == ErrorCode::InvalidLocation);
        assert_eq!(
            accepted,
            case["valid"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
    }

    // JavaScript evaluates this pattern against the same cases. Here, prove all MCP
    // and REST inputs expose that tested pattern without incompatible raw bounds.
    let expected = spec()["paths"]["/v1/cities"]["get"]["parameters"][0]["schema"]["pattern"]
        .as_str()
        .expect("city input needs a trimmed-length pattern")
        .to_owned();
    let matches = |schema: &Value| {
        schema["pattern"] == expected
            && schema.get("minLength").is_none()
            && schema.get("maxLength").is_none()
    };
    let mut failures = Vec::new();
    let spec = spec();
    for (route, name) in [
        ("/v1/cities", "q"),
        ("/v1/weather", "city"),
        ("/v1/forecast/hourly", "city"),
        ("/v1/alerts", "city"),
    ] {
        let parameter = spec["paths"][route]["get"]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == name)
            .unwrap();
        let schema = &parameter["schema"];
        if !matches(schema) {
            failures.push(format!("OpenAPI {route} {name}: {schema}"));
        }
    }
    let h = fixture(Faults::default()).await;
    let list = mcp(&h.weather, "tools/list", json!({})).await;
    let tools = list["result"]["tools"].as_array().unwrap();
    for (tool, property) in [
        ("search_cities", "query"),
        ("get_weather", "city"),
        ("get_hourly_forecast", "city"),
        ("get_active_alerts", "city"),
    ] {
        let tool_schema = tools.iter().find(|t| t["name"] == tool).unwrap();
        let schema = &tool_schema["inputSchema"]["properties"][property];
        if !matches(schema) {
            failures.push(format!("MCP {tool} {property}: {schema}"));
        }
    }
    assert!(
        failures.is_empty(),
        "expected {expected:?}:\n{}",
        failures.join("\n")
    );
}
async fn mcp(weather: &Arc<Weather>, method: &str, params: Value) -> Value {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let response = app(weather.clone(), HttpConfig::default())
        .oneshot(
            Request::post("/mcp")
                .header("host", "127.0.0.1:8790")
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("mcp-protocol-version", "2025-11-25")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn generated_openapi_artifact_is_current() {
    assert!(
        weather_bridge::api::openapi_json() == include_str!("../openapi.json"),
        "run cargo run --locked --example export-openapi -- openapi.json"
    );
}

/// Generation must retain query requirements and endpoint-specific alert outcomes.
#[test]
fn generated_contract_preserves_location_and_alert_semantics() {
    let spec = spec();
    let city_query = &spec["paths"]["/v1/cities"]["get"]["parameters"][0];
    assert_eq!(city_query["name"], "q");
    assert_eq!(city_query["required"], true);
    assert!(city_query["schema"]["pattern"].is_string());
    for route in ["/v1/weather", "/v1/forecast/hourly", "/v1/alerts"] {
        let parameters = spec["paths"][route]["get"]["parameters"]
            .as_array()
            .unwrap();
        let names: BTreeSet<_> = parameters
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            BTreeSet::from(["city", "cityId", "lat", "lon", "units"])
        );
        assert!(parameters.iter().all(|p| p["required"] != true));
        let latitude = parameters.iter().find(|p| p["name"] == "lat").unwrap();
        assert_eq!(latitude["schema"]["minimum"], -90);
        assert_eq!(latitude["schema"]["maximum"], 90);
    }
    let validator = Validator { root: &spec };
    for (name, allowed, forbidden) in [
        (
            "WeatherReport",
            vec!["checked", "unavailable"],
            "not-checked",
        ),
        (
            "ActiveAlerts",
            vec!["checked", "unavailable"],
            "not-checked",
        ),
        ("HourlyForecast", vec!["not-checked"], "checked"),
    ] {
        let schema = &spec["components"]["schemas"][name]["properties"]["alertsStatus"];
        for value in allowed {
            assert!(
                validator.errors(schema, &json!(value)).is_empty(),
                "{name}: {value}"
            );
        }
        assert!(
            !validator.errors(schema, &json!(forbidden)).is_empty(),
            "{name}: accepted {forbidden}"
        );
    }
}
