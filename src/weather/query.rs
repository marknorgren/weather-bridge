//! Weather queries and location resolution: which place a query names, and the rounded
//! grid-lookup point used for it.
use super::convert::distance;
use crate::{
    Error,
    cities::{Cities, city_query_schema},
    model::{Location, Precision},
};
use rmcp::schemars;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(description = "Unit system for numeric temperatures and winds.")]
pub enum Units {
    #[default]
    Us,
    Metric,
}
#[derive(Clone, Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WeatherQuery {
    /// Exact city name, optionally qualified with state, e.g. Seattle, WA. Ambiguous names return choices.
    #[schemars(transform = city_query_schema, example = "Seattle, WA")]
    pub city: Option<String>,
    /// GeoNames city ID from search_cities. Alternative to city or coordinates.
    #[schemars(range(min = 1))]
    pub city_id: Option<u64>,
    /// Latitude. The NWS grid lookup uses the value rounded to two decimals (see location.gridLookupPoint); alerts use four decimals.
    #[schemars(range(min = -90, max = 90))]
    pub lat: Option<f64>,
    /// Longitude. The NWS grid lookup uses the value rounded to two decimals (see location.gridLookupPoint); alerts use four decimals.
    #[schemars(range(min = -180, max = 180))]
    pub lon: Option<f64>,
    #[serde(default)]
    pub units: Units,
}
/// Point for the NWS /points grid lookup, rounded to two decimals (about 1 km). Nearby
/// queries share the grid lookup and the forecasts it leads to, so nudging coordinates
/// cannot force unbounded uncached work. Alerts use four-decimal coordinate precision
/// instead because warning polygons have precise edges.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
#[schemars(
    description = "Point sent to the NWS /points grid lookup: the location rounded to two decimals (about 1 km) so nearby requests share the grid lookup and its forecasts. Alerts use four-decimal coordinate precision rather than this grid point."
)]
pub struct GridPoint {
    pub latitude: f64,
    pub longitude: f64,
}
impl GridPoint {
    pub fn new(latitude: f64, longitude: f64) -> Self {
        // Adding 0.0 turns -0.0 into 0.0 so both spellings share one cache key.
        let grid = |n: f64| (n * 100.0).round() / 100.0 + 0.0;
        Self {
            latitude: grid(latitude),
            longitude: grid(longitude),
        }
    }
    /// The point as the /points path segment, e.g. `47.61,-122.33`.
    pub(super) fn query(self) -> String {
        format!("{:.2},{:.2}", self.latitude, self.longitude)
    }
}
/// Coordinates within this distance of a city's center are named "Near" that city.
const NEARBY_CITY_KM: f64 = 25.0;

/// The location a query names. Exactly one mode is allowed: city name, city ID, or both
/// coordinates.
pub(super) fn resolve(cities: &Cities, q: &WeatherQuery) -> Result<Location, Error> {
    let modes = usize::from(q.city.is_some())
        + usize::from(q.city_id.is_some())
        + usize::from(q.lat.is_some() || q.lon.is_some());
    if modes != 1 {
        return Err(Error::invalid(
            "Supply one location: city, cityId, or both lat and lon. Example: ?city=Seattle%2C%20WA",
        ));
    }
    let city = if let Some(name) = &q.city {
        Some(cities.resolve(name)?)
    } else if let Some(id) = q.city_id {
        Some(cities.by_id(id)?)
    } else {
        None
    };
    if let Some(c) = city {
        return Ok(Location {
            name: format!("{}, {}", c.name, c.state),
            latitude: c.latitude,
            longitude: c.longitude,
            time_zone: Some(c.time_zone),
            city_id: Some(c.id),
            precision: Precision::CityCenter,
            grid_lookup_point: GridPoint::new(c.latitude, c.longitude),
        });
    }
    let (Some(lat), Some(lon)) = (q.lat, q.lon) else {
        return Err(Error::invalid("Both lat and lon are required."));
    };
    if !lat.is_finite()
        || !lon.is_finite()
        || !(-90.0..=90.0).contains(&lat)
        || !(-180.0..=180.0).contains(&lon)
    {
        return Err(Error::invalid(
            "Latitude must be -90 to 90 and longitude -180 to 180.",
        ));
    }
    let nearby = cities
        .nearest(lat, lon)
        .filter(|city| distance(lat, lon, city.latitude, city.longitude) <= NEARBY_CITY_KM);
    Ok(Location {
        name: nearby
            .as_ref()
            .map(|city| format!("Near {}, {}", city.name, city.state))
            .unwrap_or_else(|| format!("{lat:.4}, {lon:.4}")),
        latitude: lat,
        longitude: lon,
        time_zone: nearby.map(|city| city.time_zone),
        city_id: None,
        precision: Precision::Coordinates,
        grid_lookup_point: GridPoint::new(lat, lon),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_location_combinations() {
        let cities = Cities::load().unwrap();
        assert!(resolve(&cities, &WeatherQuery::default()).is_err());
        assert!(
            resolve(
                &cities,
                &WeatherQuery {
                    city: Some("Seattle".into()),
                    lat: Some(1.),
                    lon: Some(1.),
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert!(
            resolve(
                &cities,
                &WeatherQuery {
                    lat: Some(f64::NAN),
                    lon: Some(1.),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    #[test]
    fn grid_point_rounds_to_two_decimals() {
        assert_eq!(
            GridPoint::new(47.6062, -122.3321),
            GridPoint::new(47.6149, -122.3260)
        );
        assert_eq!(GridPoint::new(47.6062, -122.3321).query(), "47.61,-122.33");
        assert_eq!(GridPoint::new(-0.001, 0.004).query(), "0.00,0.00");
    }
}
