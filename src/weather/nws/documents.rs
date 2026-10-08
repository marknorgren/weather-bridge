//! Typed NWS input documents, limited to the fields this service reads. Unknown fields are
//! ignored.
//!
//! NWS is external input, so every field is lenient: a missing, null or wrongly typed value
//! becomes `None` (or the empty default) instead of failing the whole document, and a JSON
//! array is accepted only where a list is expected. Whether a document is usable, for
//! example whether a forecast has periods, is the caller's decision.
use serde::{Deserialize, Deserializer, de::DeserializeOwned};
use serde_json::{Number, Value};

/// A typed upstream document with the structural fields needed before it is reusable.
/// Optional scalar fields remain lenient; the envelope, required lists and observation
/// timestamp participate in cache admission.
pub(crate) trait Document: DeserializeOwned + Default {
    fn missing_required(&self, raw: &Value) -> Option<&'static str>;
}

impl Document for Value {
    fn missing_required(&self, _raw: &Value) -> Option<&'static str> {
        (!self.is_object()).then_some("document")
    }
}

fn missing_object(raw: &Value, field: &'static str) -> Option<&'static str> {
    (!raw.get(field).is_some_and(Value::is_object)).then_some(field)
}

/// Decode a document, or its empty default when the body is not a JSON object.
pub(crate) fn decode<T: DeserializeOwned + Default>(value: &Value) -> T {
    if value.is_array() {
        return T::default();
    }
    T::deserialize(value).unwrap_or_default()
}
fn lenient_value<T: DeserializeOwned + Default>(value: Value) -> T {
    if value.is_array() {
        // A struct must not be filled positionally from an array.
        return T::default();
    }
    T::deserialize(value).unwrap_or_default()
}
/// Field deserializer: a missing, null or malformed value becomes `T::default()`.
fn lenient<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(d: D) -> Result<T, D::Error> {
    Ok(lenient_value(Value::deserialize(d)?))
}
/// List deserializer: `None` unless the value is an array; each malformed item becomes
/// `T::default()` so one bad item never hides the others.
fn lenient_list<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    d: D,
) -> Result<Option<Vec<T>>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Array(items) => Some(items.into_iter().map(lenient_value).collect()),
        _ => None,
    })
}

/// `/points/{lat},{lon}`: the grid lookup that links to forecasts and stations.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Points {
    #[serde(deserialize_with = "lenient")]
    pub properties: PointsProperties,
}
impl Document for Points {
    fn missing_required(&self, raw: &Value) -> Option<&'static str> {
        missing_object(raw, "properties")
    }
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct PointsProperties {
    #[serde(deserialize_with = "lenient")]
    pub forecast: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub forecast_hourly: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub observation_stations: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub relative_location: RelativeLocation,
    #[serde(deserialize_with = "lenient")]
    pub time_zone: Option<String>,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct RelativeLocation {
    #[serde(deserialize_with = "lenient")]
    pub properties: Place,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Place {
    #[serde(deserialize_with = "lenient")]
    pub city: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub state: Option<String>,
}

/// A daily (`forecast`) or hourly (`forecastHourly`) forecast.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Forecast {
    #[serde(deserialize_with = "lenient")]
    pub properties: ForecastProperties,
}
impl Document for Forecast {
    fn missing_required(&self, raw: &Value) -> Option<&'static str> {
        missing_object(raw, "properties")
            .or_else(|| self.properties.periods.is_none().then_some("periods"))
    }
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ForecastProperties {
    #[serde(deserialize_with = "lenient")]
    pub update_time: Option<String>,
    /// Required by callers: `None` means the document has no periods list.
    #[serde(deserialize_with = "lenient_list")]
    pub periods: Option<Vec<ForecastPeriod>>,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ForecastPeriod {
    #[serde(deserialize_with = "lenient")]
    pub name: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub start_time: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub end_time: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub is_daytime: Option<bool>,
    #[serde(deserialize_with = "lenient")]
    pub temperature: Option<f64>,
    #[serde(deserialize_with = "lenient")]
    pub temperature_unit: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub wind_speed: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub wind_direction: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub probability_of_precipitation: Measurement,
    #[serde(deserialize_with = "lenient")]
    pub short_forecast: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub detailed_forecast: Option<String>,
}
/// An NWS quantitative value. The number keeps its JSON form (55 stays 55).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Measurement {
    #[serde(deserialize_with = "lenient")]
    pub value: Option<Number>,
    #[serde(deserialize_with = "lenient")]
    pub unit_code: Option<String>,
}
impl Measurement {
    pub fn as_f64(&self) -> Option<f64> {
        self.value.as_ref().and_then(Number::as_f64)
    }
}

/// The observation stations for a grid, in NWS order.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Stations {
    #[serde(deserialize_with = "lenient_list")]
    pub features: Option<Vec<Station>>,
}
impl Document for Stations {
    fn missing_required(&self, _raw: &Value) -> Option<&'static str> {
        self.features.is_none().then_some("features")
    }
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Station {
    #[serde(deserialize_with = "lenient")]
    pub geometry: Geometry,
    #[serde(deserialize_with = "lenient")]
    pub properties: StationProperties,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Geometry {
    /// GeoJSON order: longitude, latitude.
    #[serde(deserialize_with = "lenient_list")]
    pub coordinates: Option<Vec<Option<f64>>>,
}
impl Geometry {
    /// Latitude and longitude, when both are numbers.
    pub fn lat_lon(&self) -> Option<(f64, f64)> {
        let coordinates = self.coordinates.as_ref()?;
        Some(((*coordinates.get(1)?)?, (*coordinates.first()?)?))
    }
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct StationProperties {
    #[serde(deserialize_with = "lenient")]
    pub station_identifier: Option<String>,
}

/// `/stations/{id}/observations/latest`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct LatestObservation {
    #[serde(deserialize_with = "lenient")]
    pub properties: ObservationProperties,
}
impl Document for LatestObservation {
    fn missing_required(&self, raw: &Value) -> Option<&'static str> {
        missing_object(raw, "properties")
            .or_else(|| self.properties.timestamp.is_none().then_some("timestamp"))
    }
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ObservationProperties {
    #[serde(deserialize_with = "lenient")]
    pub timestamp: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub text_description: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub temperature: Measurement,
    #[serde(deserialize_with = "lenient")]
    pub relative_humidity: Measurement,
    #[serde(deserialize_with = "lenient")]
    pub wind_speed: Measurement,
    #[serde(deserialize_with = "lenient")]
    pub wind_direction: Measurement,
}

/// `/alerts/active?point=...`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Alerts {
    /// Required by callers: without a features list no all-clear can be inferred.
    #[serde(deserialize_with = "lenient_list")]
    pub features: Option<Vec<AlertFeature>>,
}
impl Document for Alerts {
    fn missing_required(&self, raw: &Value) -> Option<&'static str> {
        let Some(features) = raw.get("features").and_then(Value::as_array) else {
            return Some("features");
        };
        // A malformed entry cannot support an all-clear. Validate the original
        // values before lenient decoding can turn malformed instructions into None.
        features
            .iter()
            .any(|feature| {
                let properties = &feature["properties"];
                !feature.is_object()
                    || !properties.is_object()
                    || properties["event"]
                        .as_str()
                        .is_none_or(|event| event.trim().is_empty())
                    || !(feature["id"].is_null() || feature["id"].is_string())
                    || [
                        "severity",
                        "headline",
                        "description",
                        "instruction",
                        "effective",
                        "expires",
                        "areaDesc",
                    ]
                    .iter()
                    .any(|key| !(properties[*key].is_null() || properties[*key].is_string()))
            })
            .then_some("features.properties")
    }
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct AlertFeature {
    #[serde(deserialize_with = "lenient")]
    pub id: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub properties: AlertProperties,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AlertProperties {
    #[serde(deserialize_with = "lenient")]
    pub event: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub severity: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub headline: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub description: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub instruction: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub effective: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub expires: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub area_desc: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn well_formed_documents_decode_every_used_field() {
        let points: Points = decode(&json!({"properties": {
            "forecast": "https://api.weather.gov/f", "forecastHourly": "https://api.weather.gov/h",
            "observationStations": "https://api.weather.gov/s", "timeZone": "America/Chicago",
            "relativeLocation": {"properties": {"city": "Omaha", "state": "NE"}}, "extra": 1}}));
        let p = &points.properties;
        assert_eq!(p.forecast.as_deref(), Some("https://api.weather.gov/f"));
        assert_eq!(
            p.forecast_hourly.as_deref(),
            Some("https://api.weather.gov/h")
        );
        assert_eq!(
            p.observation_stations.as_deref(),
            Some("https://api.weather.gov/s")
        );
        assert_eq!(p.time_zone.as_deref(), Some("America/Chicago"));
        assert_eq!(
            p.relative_location.properties.city.as_deref(),
            Some("Omaha")
        );

        let forecast: Forecast = decode(&json!({"properties": {"updateTime": "t", "periods": [
            {"name": "Today", "temperature": 68, "isDaytime": true,
             "probabilityOfPrecipitation": {"value": 20, "unitCode": "wmoUnit:percent"}}]}}));
        let periods = forecast.properties.periods.unwrap();
        assert_eq!(periods[0].name.as_deref(), Some("Today"));
        assert_eq!(periods[0].temperature, Some(68.0));
        assert_eq!(periods[0].is_daytime, Some(true));
        // Integers keep their JSON form.
        assert_eq!(
            periods[0].probability_of_precipitation.value,
            Some(Number::from(20))
        );

        let stations: Stations = decode(&json!({"features": [
            {"geometry": {"coordinates": [-122.3, 47.6]}, "properties": {"stationIdentifier": "KSEA"}}]}));
        let station = &stations.features.unwrap()[0];
        assert_eq!(station.geometry.lat_lon(), Some((47.6, -122.3)));
        assert_eq!(
            station.properties.station_identifier.as_deref(),
            Some("KSEA")
        );

        let alerts: Alerts = decode(&json!({"features": [
            {"id": "urn:1", "properties": {"event": "Wind Advisory", "areaDesc": "Seattle"}}]}));
        let alert = &alerts.features.unwrap()[0];
        assert_eq!(alert.id.as_deref(), Some("urn:1"));
        assert_eq!(alert.properties.area_desc.as_deref(), Some("Seattle"));
    }

    #[test]
    fn malformed_fields_become_none_without_failing_the_document() {
        let forecast: Forecast = decode(&json!({"properties": {"updateTime": 5, "periods": [
            {"name": 7, "startTime": {"x": 1}, "isDaytime": "yes", "temperature": "68",
             "windSpeed": 10, "probabilityOfPrecipitation": "20", "detailedForecast": "Sunny."},
            7, "text", null, [1]]}}));
        assert_eq!(forecast.properties.update_time, None);
        let periods = forecast.properties.periods.unwrap();
        assert_eq!(
            periods.len(),
            5,
            "malformed items stay in place as empty periods"
        );
        let first = &periods[0];
        assert_eq!(first.name, None);
        assert_eq!(first.start_time, None);
        assert_eq!(first.is_daytime, None);
        assert_eq!(first.temperature, None);
        assert_eq!(first.wind_speed, None);
        assert_eq!(first.probability_of_precipitation.value, None);
        assert_eq!(first.detailed_forecast.as_deref(), Some("Sunny."));
        assert!(periods[1..].iter().all(|p| p.name.is_none()));

        let observation: LatestObservation = decode(&json!({"properties": {
            "timestamp": "2026-01-01T00:00:00Z", "textDescription": 4,
            "temperature": {"value": "1", "unitCode": "wmoUnit:degC"},
            "relativeHumidity": {"value": 55.25}, "windSpeed": "fast", "windDirection": [200]}}));
        let o = &observation.properties;
        assert_eq!(o.timestamp.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(o.text_description, None);
        assert_eq!(o.temperature.as_f64(), None);
        assert_eq!(o.temperature.unit_code.as_deref(), Some("wmoUnit:degC"));
        assert_eq!(o.relative_humidity.as_f64(), Some(55.25));
        assert_eq!(o.wind_speed.value, None);
        assert_eq!(o.wind_direction.value, None);

        let stations: Stations = decode(&json!({"features": [
            {"geometry": {"coordinates": "x"}, "properties": {"stationIdentifier": 9}},
            {"geometry": {"coordinates": [-100.0, "47"]}}, 7]}));
        let features = stations.features.unwrap();
        assert_eq!(features.len(), 3);
        assert!(features.iter().all(|s| s.geometry.lat_lon().is_none()));
        assert_eq!(features[0].properties.station_identifier, None);

        let alerts: Alerts = decode(
            &json!({"features": [{"id": 1, "properties": {"event": false, "headline": "Wind"}}, "x"]}),
        );
        let features = alerts.features.unwrap();
        assert_eq!(features[0].id, None);
        assert_eq!(features[0].properties.event, None);
        assert_eq!(features[0].properties.headline.as_deref(), Some("Wind"));
        assert_eq!(features[1].properties.headline, None);
    }

    #[test]
    fn missing_required_lists_are_none_and_arrays_never_fill_structs() {
        let forecast: Forecast = decode(&json!({"properties": {"periods": "none"}}));
        assert!(forecast.properties.periods.is_none());
        let forecast: Forecast = decode(&json!({"properties": {}}));
        assert!(forecast.properties.periods.is_none());
        for body in [
            json!(null),
            json!([]),
            json!({}),
            json!([1, 2]),
            json!({"features": {}}),
        ] {
            let alerts: Alerts = decode(&body);
            assert!(alerts.features.is_none(), "{body}");
        }
        // A struct is never filled positionally from an array.
        let points: Points = decode(&json!({"properties": ["https://api.weather.gov/f"]}));
        assert_eq!(points.properties.forecast, None);
        let points: Points = decode(&json!([{"forecast": "https://api.weather.gov/f"}]));
        assert_eq!(points.properties.forecast, None);
    }
}
