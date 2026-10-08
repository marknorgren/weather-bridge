//! NWS documents to response types: forecast periods, alerts and station observations.
use super::{
    Units,
    convert::{
        TemperatureScale, WmoUnit, distance, observed_temperature, round, temperature, wind_speed,
        wind_text,
    },
    nws::documents::{AlertFeature, Forecast, ForecastPeriod, LatestObservation, Stations},
};
use crate::model::{Alert, IssuedSource, Observation, Period};
use chrono::{DateTime, Utc};

/// Observations older than this are marked stale.
const STALE_AFTER_SECONDS: i64 = 2 * 60 * 60;
/// How many of the nearest stations are tried for an observation.
const STATIONS_TRIED: usize = 3;

/// Up to `limit` periods, or `None` when the document has no periods list.
pub(super) fn periods(forecast: &Forecast, limit: usize, units: Units) -> Option<Vec<Period>> {
    let periods = forecast.properties.periods.as_ref()?;
    Some(
        periods
            .iter()
            .take(limit)
            .map(|p| period(p, units))
            .collect(),
    )
}
fn period(p: &ForecastPeriod, units: Units) -> Period {
    Period {
        name: p.name.clone(),
        starts_at: p.start_time.clone(),
        ends_at: p.end_time.clone(),
        is_daytime: p.is_daytime,
        temperature: TemperatureScale::from_forecast_unit(p.temperature_unit.as_deref())
            .and_then(|scale| temperature(p.temperature, scale, units)),
        precipitation_probability_percent: p.probability_of_precipitation.value.clone(),
        wind: wind_text(
            p.wind_speed.as_deref().unwrap_or(""),
            p.wind_direction.as_deref().unwrap_or(""),
            units,
        ),
        condition: p.short_forecast.clone(),
        detail: p.detailed_forecast.clone(),
    }
}
/// The source entry for a forecast document; `None` when the document was unavailable.
pub(super) fn issued_source(url: String, forecast: Option<&Forecast>) -> IssuedSource {
    IssuedSource {
        url,
        issued_at: forecast.and_then(|f| f.properties.update_time.clone()),
    }
}
pub(super) fn alert_list(features: &[AlertFeature]) -> Vec<Alert> {
    features
        .iter()
        .map(|f| {
            let a = &f.properties;
            Alert {
                id: f.id.clone(),
                event: a.event.clone(),
                severity: a.severity.clone(),
                headline: a.headline.clone(),
                description: a.description.clone(),
                instruction: a.instruction.clone(),
                effective_at: a.effective.clone(),
                expires_at: a.expires.clone(),
                area: a.area_desc.clone(),
            }
        })
        .collect()
}
/// Station IDs to try for an observation: of the stations nearest to the point, closest
/// first, those with a plain alphanumeric ID, with their distance in km. Stations without
/// coordinates are skipped.
pub(super) fn nearest_stations(stations: &Stations, lat: f64, lon: f64) -> Vec<(f64, &str)> {
    let mut ranked: Vec<_> = stations
        .features
        .iter()
        .flatten()
        .filter_map(|station| {
            let (station_lat, station_lon) = station.geometry.lat_lon()?;
            Some((distance(lat, lon, station_lat, station_lon), station))
        })
        .collect();
    ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
    ranked
        .into_iter()
        .take(STATIONS_TRIED)
        .filter_map(|(km, station)| {
            let id = station.properties.station_identifier.as_deref()?;
            id.chars()
                .all(|c| c.is_ascii_alphanumeric())
                .then_some((km, id))
        })
        .collect()
}
/// A station observation, or `None` when it has no timestamp.
pub(super) fn observation(
    latest: &LatestObservation,
    station: &str,
    km: f64,
    units: Units,
    source_url: String,
) -> Option<Observation> {
    let o = &latest.properties;
    let observed_at = o.timestamp.clone()?;
    let age = DateTime::parse_from_rfc3339(&observed_at)
        .ok()
        .map(|t| (Utc::now() - t.with_timezone(&Utc)).num_seconds().max(0));
    Some(Observation {
        observed_at,
        age_seconds: age,
        stale: age.is_none_or(|age| age > STALE_AFTER_SECONDS),
        station: station.into(),
        station_distance_km: round(km),
        condition: o.text_description.clone(),
        temperature: observed_temperature(
            o.temperature.as_f64(),
            WmoUnit::parse(o.temperature.unit_code.as_deref()),
            units,
        ),
        humidity_percent: o.relative_humidity.value.clone(),
        wind_speed: wind_speed(
            o.wind_speed.as_f64(),
            WmoUnit::parse(o.wind_speed.unit_code.as_deref()),
            units,
        ),
        wind_direction_degrees: o.wind_direction.value.clone(),
        source_url,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::weather::nws::documents::{Alerts, decode};
    use serde_json::json;
    #[test]
    fn malformed_upstream_fields_become_null_not_other_json_types() {
        let forecast: Forecast = decode(&json!({"properties": {"periods": [{
            "name": 7, "startTime": {"x": 1}, "isDaytime": "yes", "shortForecast": ["Sunny"],
            "detailedForecast": "Sunny.", "probabilityOfPrecipitation": {"value": "20"},
            "temperature": 68, "temperatureUnit": "F"
        }]}}));
        let periods = periods(&forecast, 14, Units::Us).unwrap();
        let wire = serde_json::to_value(&periods[0]).unwrap();
        for key in [
            "name",
            "startsAt",
            "isDaytime",
            "condition",
            "precipitationProbabilityPercent",
        ] {
            assert!(wire[key].is_null(), "{key}: {}", wire[key]);
        }
        assert_eq!(wire["detail"], "Sunny.");
        assert_eq!(wire["temperature"], json!({"value": 68.0, "unit": "°F"}));
        // An unrecognized forecast unit yields no temperature rather than a misread one;
        // the rest of the period is kept.
        let forecast: Forecast = decode(&json!({"properties": {"periods": [{
            "detailedForecast": "Sunny.", "temperature": 293, "temperatureUnit": "K"
        }]}}));
        let kelvin = super::periods(&forecast, 14, Units::Us).unwrap();
        let wire = serde_json::to_value(&kelvin[0]).unwrap();
        assert!(wire["temperature"].is_null(), "{}", wire["temperature"]);
        assert_eq!(wire["detail"], "Sunny.");
        let alerts: Alerts = decode(
            &json!({"features":[{"id": 1, "properties": {"event": false, "headline": "Wind"}}]}),
        );
        let wire = serde_json::to_value(alert_list(&alerts.features.unwrap())).unwrap();
        assert!(wire[0]["id"].is_null() && wire[0]["event"].is_null());
        assert_eq!(wire[0]["headline"], "Wind");
    }
    #[test]
    fn a_malformed_period_is_kept_as_an_empty_period() {
        let forecast: Forecast =
            decode(&json!({"properties": {"periods": [7, {"name": "Today"}]}}));
        let wire = serde_json::to_value(periods(&forecast, 14, Units::Us).unwrap()).unwrap();
        assert_eq!(wire[0]["name"], json!(null));
        assert_eq!(wire[0]["wind"], "");
        assert_eq!(wire[1]["name"], "Today");
        assert!(periods(&decode(&json!({})), 14, Units::Us).is_none());
    }
    #[test]
    fn only_the_three_nearest_stations_with_plain_ids_are_tried() {
        let stations: Stations = decode(&json!({"features": [
            {"geometry": {"coordinates": [-100.4, 40.0]}, "properties": {"stationIdentifier": "FOURTH"}},
            {"geometry": {"coordinates": [-100.1, 40.0]}, "properties": {"stationIdentifier": "bad-id"}},
            {"geometry": {"coordinates": "x"}, "properties": {"stationIdentifier": "NOGEO"}},
            {"geometry": {"coordinates": [-100.0, 40.0]}, "properties": {"stationIdentifier": "NEAR"}},
            {"geometry": {"coordinates": [-100.2, 40.0]}, "properties": {"stationIdentifier": "THIRD"}}
        ]}));
        let ids: Vec<_> = nearest_stations(&stations, 40.0, -100.0)
            .into_iter()
            .map(|(_, id)| id)
            .collect();
        // bad-id uses one of the three slots, so FOURTH is never tried.
        assert_eq!(ids, ["NEAR", "THIRD"]);
    }
    #[test]
    fn observation_needs_a_timestamp_and_keeps_numbers_verbatim() {
        let latest: LatestObservation = decode(&json!({"properties": {
            "temperature": {"value": 20, "unitCode": "wmoUnit:degC"}}}));
        assert!(observation(&latest, "KSEA", 1.0, Units::Us, String::new()).is_none());
        let latest: LatestObservation = decode(&json!({"properties": {
            "timestamp": "bogus", "relativeHumidity": {"value": 55},
            "windDirection": {"value": 200.0}, "temperature": {"value": 20}}}));
        let wire = serde_json::to_value(
            observation(&latest, "KSEA", 1.04, Units::Us, "u".into()).unwrap(),
        )
        .unwrap();
        assert_eq!(wire["stale"], true, "an invalid timestamp is stale");
        assert_eq!(wire["ageSeconds"], json!(null));
        assert_eq!(wire["humidityPercent"].to_string(), "55");
        assert_eq!(wire["windDirectionDegrees"].to_string(), "200.0");
        assert_eq!(
            wire["temperature"],
            json!(null),
            "no unit code, no temperature"
        );
        assert_eq!(wire["stationDistanceKm"], 1.0);
    }
    #[test]
    fn observed_quantities_keep_their_wire_unit_strings() {
        let latest: LatestObservation = decode(&json!({"properties": {
            "timestamp": "2026-10-01T12:00:00+00:00",
            "temperature": {"value": 20, "unitCode": "wmoUnit:degC"},
            "windSpeed": {"value": 10, "unitCode": "wmoUnit:kn"}}}));
        let wire = |units| {
            serde_json::to_value(observation(&latest, "KSEA", 1.0, units, "u".into()).unwrap())
                .unwrap()
        };
        let us = wire(Units::Us);
        assert_eq!(us["temperature"], json!({"value": 68.0, "unit": "°F"}));
        assert_eq!(us["windSpeed"], json!({"value": 11.5, "unit": "mph"}));
        let metric = wire(Units::Metric);
        assert_eq!(metric["temperature"], json!({"value": 20.0, "unit": "°C"}));
        assert_eq!(metric["windSpeed"], json!({"value": 18.5, "unit": "km/h"}));
    }
}
