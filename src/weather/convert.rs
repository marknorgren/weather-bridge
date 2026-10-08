//! Unit conversion, rounding, wind text and great-circle distance. Pure functions.
use super::Units;
use crate::model::{Quantity, Unit};

/// Great-circle (haversine) distance in kilometres between two latitude/longitude points.
pub(super) fn distance(a: f64, b: f64, c: f64, d: f64) -> f64 {
    let x = ((c - a).to_radians() / 2.0).sin().powi(2)
        + a.to_radians().cos() * c.to_radians().cos() * ((d - b).to_radians() / 2.0).sin().powi(2);
    6371.0 * 2.0 * x.sqrt().asin()
}
/// Round to one decimal place.
pub(super) fn round(n: f64) -> f64 {
    (n * 10.0).round() / 10.0
}
pub(super) fn quantity(value: Option<f64>, unit: Unit) -> Option<Quantity> {
    value.map(|v| Quantity {
        value: round(v),
        unit,
    })
}
/// The scale of an NWS forecast period's `temperatureUnit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TemperatureScale {
    Celsius,
    Fahrenheit,
}
impl TemperatureScale {
    /// A forecast `temperatureUnit`: "F" or absent is Fahrenheit (the NWS default) and "C"
    /// is Celsius. Any other unit is `None`, so the temperature is omitted, not misread.
    pub(super) fn from_forecast_unit(unit: Option<&str>) -> Option<Self> {
        match unit {
            None | Some("F") => Some(Self::Fahrenheit),
            Some("C") => Some(Self::Celsius),
            Some(_) => None,
        }
    }
}
/// An NWS observation WMO unit code this service converts. Codes outside this set have no
/// value here, so the measurement they label yields no quantity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WmoUnit {
    DegreesCelsius,
    DegreesFahrenheit,
    KilometresPerHour,
    MetresPerSecond,
    Knots,
}
impl WmoUnit {
    /// Parse a `unitCode` such as "wmoUnit:degC"; `None` when absent or not converted here.
    pub(super) fn parse(code: Option<&str>) -> Option<Self> {
        match code? {
            "wmoUnit:degC" => Some(Self::DegreesCelsius),
            "wmoUnit:degF" => Some(Self::DegreesFahrenheit),
            "wmoUnit:km_h-1" => Some(Self::KilometresPerHour),
            "wmoUnit:m_s-1" => Some(Self::MetresPerSecond),
            "wmoUnit:kn" => Some(Self::Knots),
            _ => None,
        }
    }
}
/// A temperature on the `source` scale converted to the selected units.
pub(super) fn temperature(
    value: Option<f64>,
    source: TemperatureScale,
    units: Units,
) -> Option<Quantity> {
    let c = value.map(|v| match source {
        TemperatureScale::Fahrenheit => (v - 32.0) * 5.0 / 9.0,
        TemperatureScale::Celsius => v,
    });
    match units {
        Units::Us => quantity(c.map(|v| v * 9.0 / 5.0 + 32.0), Unit::Fahrenheit),
        Units::Metric => quantity(c, Unit::Celsius),
    }
}
/// An observed temperature with its WMO unit. Missing or non-temperature units yield none.
pub(super) fn observed_temperature(
    value: Option<f64>,
    unit: Option<WmoUnit>,
    units: Units,
) -> Option<Quantity> {
    match unit? {
        WmoUnit::DegreesCelsius => temperature(value, TemperatureScale::Celsius, units),
        WmoUnit::DegreesFahrenheit => temperature(value, TemperatureScale::Fahrenheit, units),
        WmoUnit::KilometresPerHour | WmoUnit::MetresPerSecond | WmoUnit::Knots => None,
    }
}
/// An observed wind speed with its WMO unit, as mph or km/h. Missing or non-speed units
/// yield none.
pub(super) fn wind_speed(
    value: Option<f64>,
    unit: Option<WmoUnit>,
    units: Units,
) -> Option<Quantity> {
    let metres_per_second = value.and_then(|v| match unit? {
        WmoUnit::KilometresPerHour => Some(v / 3.6),
        WmoUnit::MetresPerSecond => Some(v),
        WmoUnit::Knots => Some(v * 0.514444),
        WmoUnit::DegreesCelsius | WmoUnit::DegreesFahrenheit => None,
    });
    match units {
        Units::Us => quantity(metres_per_second.map(|v| v * 2.236936), Unit::MilesPerHour),
        Units::Metric => quantity(metres_per_second.map(|v| v * 3.6), Unit::KilometresPerHour),
    }
}
/// NWS forecast wind text such as "5 to 10 mph" plus direction; mph ranges become km/h in
/// metric. Text that is not a plain mph value or range passes through unchanged.
pub(super) fn wind_text(speed: &str, direction: &str, units: Units) -> String {
    if matches!(units, Units::Metric) && speed.ends_with(" mph") {
        let converted = speed
            .trim_end_matches(" mph")
            .split(" to ")
            .map(|n| n.parse::<f64>().map(|v| format!("{:.0}", v * 1.609344)))
            .collect::<Result<Vec<_>, _>>();
        if let Ok(parts) = converted {
            return format!("{} km/h {direction}", parts.join("–"));
        }
    }
    format!("{speed} {direction}").trim().into()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unit_conversion_and_missing_values() {
        assert_eq!(
            temperature(Some(20.0), TemperatureScale::Celsius, Units::Us)
                .unwrap()
                .value,
            68.0
        );
        assert!(temperature(None, TemperatureScale::Celsius, Units::Metric).is_none());
        assert_eq!(
            wind_text("5 to 10 mph", "NW", Units::Metric),
            "8–16 km/h NW"
        );
    }
    #[test]
    fn rounding_is_one_decimal() {
        assert_eq!(round(21.14), 21.1);
        assert_eq!(round(21.15), 21.2);
        assert_eq!(round(-3.04), -3.0);
        let q = quantity(Some(6.7108), Unit::MilesPerHour).unwrap();
        assert_eq!((q.value, q.unit.as_str()), (6.7, "mph"));
    }
    #[test]
    fn forecast_temperatures_default_to_fahrenheit_sources() {
        assert_eq!(
            temperature(Some(68.0), TemperatureScale::Fahrenheit, Units::Us)
                .unwrap()
                .value,
            68.0
        );
        let c = temperature(Some(68.0), TemperatureScale::Fahrenheit, Units::Metric).unwrap();
        assert_eq!((c.value, c.unit.as_str()), (20.0, "°C"));
        assert_eq!(
            temperature(Some(-40.0), TemperatureScale::Celsius, Units::Us)
                .unwrap()
                .value,
            -40.0
        );
    }
    #[test]
    fn observed_values_need_a_known_unit_code() {
        let t = observed_temperature(Some(20.0), WmoUnit::parse(Some("wmoUnit:degC")), Units::Us)
            .unwrap();
        assert_eq!((t.value, t.unit.as_str()), (68.0, "°F"));
        let t = observed_temperature(
            Some(70.0),
            WmoUnit::parse(Some("wmoUnit:degF")),
            Units::Metric,
        )
        .unwrap();
        assert_eq!(t.value, 21.1);
        assert!(
            observed_temperature(Some(20.0), WmoUnit::parse(Some("wmoUnit:K")), Units::Us)
                .is_none()
        );
        assert!(observed_temperature(Some(20.0), WmoUnit::parse(None), Units::Us).is_none());

        let w = wind_speed(
            Some(36.0),
            WmoUnit::parse(Some("wmoUnit:km_h-1")),
            Units::Metric,
        )
        .unwrap();
        assert_eq!((w.value, w.unit.as_str()), (36.0, "km/h"));
        assert_eq!(
            wind_speed(Some(3.0), WmoUnit::parse(Some("wmoUnit:m_s-1")), Units::Us)
                .unwrap()
                .value,
            6.7
        );
        assert_eq!(
            wind_speed(
                Some(10.0),
                WmoUnit::parse(Some("wmoUnit:kn")),
                Units::Metric
            )
            .unwrap()
            .value,
            18.5
        );
        assert!(
            wind_speed(
                Some(10.0),
                WmoUnit::parse(Some("wmoUnit:furlong")),
                Units::Us
            )
            .is_none()
        );
        assert!(wind_speed(None, WmoUnit::parse(Some("wmoUnit:kn")), Units::Us).is_none());
    }
    #[test]
    fn unit_codes_are_parsed_once_into_known_units_only() {
        assert_eq!(
            WmoUnit::parse(Some("wmoUnit:degC")),
            Some(WmoUnit::DegreesCelsius)
        );
        assert_eq!(WmoUnit::parse(Some("wmoUnit:kn")), Some(WmoUnit::Knots));
        assert_eq!(WmoUnit::parse(Some("wmoUnit:K")), None);
        assert_eq!(WmoUnit::parse(Some("degC")), None);
        // A known unit of the wrong kind yields no value rather than a misread one.
        let kn = WmoUnit::parse(Some("wmoUnit:kn"));
        assert!(observed_temperature(Some(20.0), kn, Units::Us).is_none());
        let deg_c = WmoUnit::parse(Some("wmoUnit:degC"));
        assert!(wind_speed(Some(20.0), deg_c, Units::Us).is_none());

        use TemperatureScale::{Celsius, Fahrenheit};
        assert_eq!(TemperatureScale::from_forecast_unit(None), Some(Fahrenheit));
        assert_eq!(
            TemperatureScale::from_forecast_unit(Some("F")),
            Some(Fahrenheit)
        );
        assert_eq!(
            TemperatureScale::from_forecast_unit(Some("C")),
            Some(Celsius)
        );
        assert_eq!(TemperatureScale::from_forecast_unit(Some("K")), None);
    }
    #[test]
    fn wind_text_converts_only_plain_mph_values() {
        assert_eq!(wind_text("5 to 10 mph", "NW", Units::Us), "5 to 10 mph NW");
        assert_eq!(
            wind_text("about 5 mph", "S", Units::Metric),
            "about 5 mph S"
        );
        assert_eq!(wind_text("", "", Units::Metric), "");
    }
    #[test]
    fn haversine_distance_in_kilometres() {
        assert_eq!(distance(47.6, -122.3, 47.6, -122.3), 0.0);
        // One degree of latitude is about 111.2 km.
        assert_eq!(round(distance(40.0, -100.0, 41.0, -100.0)), 111.2);
        // Seattle to Portland, OR is about 233 km.
        let km = distance(47.6062, -122.3321, 45.5152, -122.6784);
        assert!((230.0..236.0).contains(&km), "{km}");
    }
}
