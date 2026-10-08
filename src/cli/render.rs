//! Plain-text rendering for CLI output. Forecast and alert wording is source content and is
//! printed verbatim.
use chrono::DateTime;
use weather_bridge::{
    Error, ErrorCode,
    cities::City,
    model::{
        ActiveAlerts, Alert, AlertsStatus, HourlyForecast, Period, Quantity, Warning, WeatherReport,
    },
};

const SOURCES: &str = "Sources: National Weather Service · GeoNames";

pub fn city_label(city: &City) -> String {
    format!("{}, {}", city.name, city.state)
}

/// Observation, forecast summary, alert headlines and source warnings for `weather`.
pub fn weather_text(report: &WeatherReport) -> String {
    let mut out = format!("{}\n\n{}\n", report.location.name, report.summary);
    if let Some(current) = &report.current {
        // Values print as JSON numbers (68.0, not 68), as the CLI always has.
        let (value, unit) = current.temperature.as_ref().map_or_else(
            || ("null".to_owned(), ""),
            |t| {
                (
                    serde_json::Number::from_f64(t.value)
                        .map_or_else(|| "null".into(), |n| n.to_string()),
                    t.unit.as_str(),
                )
            },
        );
        out += &format!(
            "\nObserved: {value} {unit} · {} · {}{}\n",
            current.condition.as_deref().unwrap_or(""),
            current.observed_at,
            if current.stale { " (stale)" } else { "" }
        );
    }
    for alert in &report.alerts {
        let headline = alert.headline.as_deref().unwrap_or("Weather alert");
        out += &format!("\nALERT: {headline}\n");
    }
    for warning in &report.warnings {
        out += &format!("\nNote: {}\n", warning.message());
    }
    out + &format!("\n{SOURCES}\n")
}

/// A readable table of cities, one per line.
pub fn cities_table(cities: &[City]) -> String {
    if cities.is_empty() {
        return "No cities match.\n".into();
    }
    let rows: Vec<[String; 5]> = cities
        .iter()
        .map(|c| {
            [
                c.id.to_string(),
                city_label(c),
                c.population.to_string(),
                format!("{:.4}", c.latitude),
                format!("{:.4}", c.longitude),
            ]
        })
        .collect();
    let head = ["ID", "CITY", "POPULATION", "LATITUDE", "LONGITUDE"];
    let width: Vec<usize> = (0..5)
        .map(|i| {
            rows.iter()
                .map(|r| r[i].chars().count())
                .chain([head[i].len()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: [&str; 5]| {
        let mut out = String::new();
        for (i, cell) in cells.iter().enumerate() {
            // Numbers align right, the city name left.
            if i == 1 {
                out += &format!("{cell:<w$}  ", w = width[i]);
            } else {
                out += &format!("{cell:>w$}  ", w = width[i]);
            }
        }
        out.trim_end().to_owned() + "\n"
    };
    let mut out = line(head);
    for row in &rows {
        out += &line([&row[0], &row[1], &row[2], &row[3], &row[4]]);
    }
    out + &format!("\n{SOURCES}\n")
}

fn quantity(q: &Option<Quantity>) -> String {
    q.as_ref()
        .map_or_else(|| "-".into(), |q| format!("{}{}", q.value, q.unit))
}
fn note(out: &mut String, warnings: &[Warning], skip: Option<Warning>) {
    for warning in warnings.iter().filter(|w| Some(**w) != skip) {
        *out += &format!("\nNote: {}\n", warning.message());
    }
}
/// One line stating what the alert check found. A failed check is never shown as "none".
fn alerts_line(status: AlertsStatus, count: usize) -> String {
    match (status, count) {
        (AlertsStatus::Unavailable, _) => Warning::AlertsUnavailable.message().into(),
        (AlertsStatus::NotChecked, _) => "Alerts: not checked".into(),
        (AlertsStatus::Checked, 0) => "Alerts: none active".into(),
        (AlertsStatus::Checked, n) => format!("Alerts: {n} active"),
    }
}

fn hour(period: &Period) -> String {
    let when = period.starts_at.as_deref().map_or_else(
        || period.name.clone().unwrap_or_else(|| "-".into()),
        |s| {
            DateTime::parse_from_rfc3339(s)
                .map_or_else(|_| s.to_owned(), |t| t.format("%a %H:%M").to_string())
        },
    );
    let rain = period
        .precipitation_probability_percent
        .as_ref()
        .map_or_else(|| "-".into(), |p| format!("{p}%"));
    format!(
        "{when:<10} {:>8} {rain:>5} rain  {}  {}\n",
        quantity(&period.temperature),
        period.wind,
        period.condition.as_deref().unwrap_or("")
    )
}

/// Compact hourly listing: one line per period.
pub fn hourly_text(forecast: &HourlyForecast) -> String {
    let mut out = format!("{}\nHourly forecast\n\n", forecast.location.name);
    if forecast.hourly.is_empty() {
        out += "No hourly periods available.\n";
    }
    for period in &forecast.hourly {
        out += &hour(period);
    }
    note(&mut out, &forecast.warnings, None);
    out + &format!("\n{SOURCES}\n")
}

fn alert(alert: &Alert) -> String {
    let mut out = format!(
        "\n{}{}\n",
        alert.event.as_deref().unwrap_or("Weather alert"),
        alert
            .severity
            .as_deref()
            .map_or_else(String::new, |s| format!(" ({s})"))
    );
    for (label, value) in [
        ("Headline", &alert.headline),
        ("Area", &alert.area),
        ("Effective", &alert.effective_at),
        ("Expires", &alert.expires_at),
    ] {
        if let Some(value) = value {
            out += &format!("  {label}: {value}\n");
        }
    }
    for (label, value) in [
        ("Description", &alert.description),
        ("Instructions", &alert.instruction),
    ] {
        if let Some(value) = value {
            out += &format!("\n  {label}:\n");
            for line in value.lines() {
                out += &format!("    {line}\n");
            }
        }
    }
    out
}

/// Alerts with their original instructions and an explicit check status.
pub fn alerts_text(alerts: &ActiveAlerts) -> String {
    let mut out = format!(
        "{}\n\n{}\n",
        alerts.location.name,
        alerts_line(alerts.alerts_status.into(), alerts.alerts.len())
    );
    for a in &alerts.alerts {
        out += &alert(a);
    }
    note(&mut out, &alerts.warnings, Some(Warning::AlertsUnavailable));
    out + &format!("\n{SOURCES}\n")
}

/// Error text for stderr: the detail, then numbered choices and a hint when the city
/// was ambiguous or unknown.
pub fn error_text(error: &Error) -> String {
    let mut out = format!("error: {}\n", error.detail);
    if let Some(choices) = error.choices.as_ref().filter(|c| !c.is_empty()) {
        let heading = match error.code {
            ErrorCode::AmbiguousCity => "Matches:",
            _ => "Did you mean:",
        };
        out += &format!("\n{heading}\n");
        let width = choices.len().to_string().len();
        for (i, city) in choices.iter().enumerate() {
            out += &format!(
                "  {:>width$}. {} (id {})\n",
                i + 1,
                city_label(city),
                city.id
            );
        }
        out += "\nHint: rerun with --city-id <ID>, or qualify the name as \"City, ST\".\n";
    }
    out
}

/// 2 for usage, invalid input, unsupported location, unknown or ambiguous city; 1 for
/// upstream and other failures.
/// Exit code for printed output: 3 when the result is incomplete (alerts could not be
/// checked or a source is missing), otherwise 0.
pub fn success_exit_code(complete: bool) -> u8 {
    if complete { 0 } else { 3 }
}

pub fn exit_code(code: ErrorCode) -> u8 {
    match code {
        ErrorCode::InvalidLocation
        | ErrorCode::RequestTooLarge
        | ErrorCode::CityNotFound
        | ErrorCode::AmbiguousCity
        | ErrorCode::OutsideCoverage => 2,
        ErrorCode::UpstreamUnavailable
        | ErrorCode::UpstreamTimeout
        | ErrorCode::Busy
        | ErrorCode::Forbidden => 1,
    }
}

#[cfg(test)]
#[path = "../../tests/common/mod.rs"]
mod common;

#[cfg(test)]
mod tests {
    use super::*;
    use weather_bridge::{
        cities::Cities,
        model::{
            AlertSources, HourlySources, IssuedSource, Location, Precision, QuerySource, Unit,
        },
        weather::{GridPoint, Units, WeatherQuery},
    };
    #[tokio::test]
    async fn cli_plain_text_renders_the_typed_report() {
        use super::common::{Faults, coordinates, fixture, seattle};
        let h = fixture(Faults::default()).await;
        let report = h.weather.report(seattle()).await.unwrap();
        assert_eq!(
            weather_text(&report),
            "Seattle, WA\n\nSunny today.\n\n\
             Observed: 68.0 °F · Clear · 2020-01-01T12:00:00Z (stale)\n\n\
             ALERT: Wind Advisory until 6 PM\n\n\
             Sources: National Weather Service · GeoNames\n"
        );
        let h = fixture(Faults {
            alerts_fail: true,
            ..Default::default()
        })
        .await;
        let metric = WeatherQuery {
            units: Units::Metric,
            ..coordinates(47.6062, -122.3321)
        };
        assert_eq!(
            weather_text(&h.weather.report(metric).await.unwrap()),
            "Near Seattle, WA\n\nSunny today.\n\n\
             Observed: 20.0 °C · Clear · 2020-01-01T12:00:00Z (stale)\n\n\
             Note: Alerts could not be checked. This does not mean there are no alerts.\n\n\
             Sources: National Weather Service · GeoNames\n"
        );
    }
    #[test]
    fn exit_codes_follow_the_documented_split() {
        for code in ErrorCode::ALL {
            let expected = match code {
                ErrorCode::InvalidLocation
                | ErrorCode::RequestTooLarge
                | ErrorCode::CityNotFound
                | ErrorCode::AmbiguousCity
                | ErrorCode::OutsideCoverage => 2,
                _ => 1,
            };
            assert_eq!(exit_code(code), expected, "{code:?}");
        }
    }
    #[test]
    fn ambiguity_error_lists_every_choice_and_a_hint() {
        let cities = Cities::load().unwrap();
        let error = cities.resolve("Franklin").unwrap_err();
        let text = error_text(&error);
        assert!(text.starts_with("error: Several cities share that name"));
        assert!(text.contains("1. Franklin, TN (id "), "{text}");
        assert!(text.contains("19. Franklin, "), "{text}");
        assert!(text.contains("--city-id") && text.contains("\"City, ST\""));
        let plain = error_text(&Error::invalid("Bad input."));
        assert_eq!(plain, "error: Bad input.\n");
    }
    #[test]
    fn cities_table_has_a_header_and_aligned_rows() {
        let table = cities_table(&Cities::load().unwrap().search("Seattle, WA").unwrap());
        let mut lines = table.lines();
        let head = lines.next().unwrap();
        assert!(
            head.starts_with("     ID  CITY") || head.starts_with("ID"),
            "{head}"
        );
        assert!(lines.next().unwrap().contains("Seattle, WA"));
        assert_eq!(cities_table(&[]), "No cities match.\n");
    }
    fn location() -> Location {
        Location {
            name: "Seattle, WA".into(),
            latitude: 47.6,
            longitude: -122.3,
            time_zone: None,
            city_id: Some(1),
            precision: Precision::CityCenter,
            grid_lookup_point: GridPoint::new(47.6, -122.3),
        }
    }
    fn alerts(status: AlertsStatus, list: Vec<Alert>, warnings: Vec<Warning>) -> ActiveAlerts {
        ActiveAlerts {
            location: location(),
            units: Units::Us,
            alerts: list,
            alerts_status: match status {
                AlertsStatus::Checked => weather_bridge::model::CheckedAlertsStatus::Checked,
                AlertsStatus::Unavailable => {
                    weather_bridge::model::CheckedAlertsStatus::Unavailable
                }
                AlertsStatus::NotChecked => panic!("active alerts check status required"),
            },
            sources: AlertSources {
                alerts: QuerySource { url: "u".into() },
            },
            warnings,
            assembled_at: "t".into(),
            cache_max_age_seconds: 0,
        }
    }
    #[test]
    fn failed_alert_checks_are_never_shown_as_no_alerts() {
        let failed = alerts_text(&alerts(
            AlertsStatus::Unavailable,
            vec![],
            vec![Warning::AlertsUnavailable],
        ));
        assert!(
            failed.contains("Alerts could not be checked. This does not mean"),
            "{failed}"
        );
        assert!(!failed.contains("none active"), "{failed}");
        assert_eq!(
            failed.matches("could not be checked").count(),
            1,
            "{failed}"
        );
        let none = alerts_text(&alerts(AlertsStatus::Checked, vec![], vec![]));
        assert!(none.contains("Alerts: none active"), "{none}");
    }
    #[test]
    fn failed_alert_checks_exit_3_not_0() {
        use weather_bridge::model::Completeness;
        let failed = alerts(
            AlertsStatus::Unavailable,
            vec![],
            vec![Warning::AlertsUnavailable],
        );
        assert_eq!(success_exit_code(failed.is_complete()), 3);
        let none = alerts(AlertsStatus::Checked, vec![], vec![]);
        assert_eq!(success_exit_code(none.is_complete()), 0);
    }
    #[test]
    fn alerts_keep_the_original_instruction() {
        let text = alerts_text(&alerts(
            AlertsStatus::Checked,
            vec![Alert {
                id: Some("a".into()),
                event: Some("Flood Watch".into()),
                severity: Some("Moderate".into()),
                headline: Some("H".into()),
                description: Some("Line one\nLine two".into()),
                instruction: Some("Move to higher ground.".into()),
                effective_at: None,
                expires_at: Some("2026-10-02T00:00:00Z".into()),
                area: Some("King".into()),
            }],
            vec![],
        ));
        assert!(text.contains("Alerts: 1 active"), "{text}");
        assert!(text.contains("Flood Watch (Moderate)"), "{text}");
        assert!(
            text.contains("Instructions:\n    Move to higher ground."),
            "{text}"
        );
        assert!(text.contains("    Line two"), "{text}");
    }
    fn period() -> Period {
        Period {
            name: Some("Now".into()),
            starts_at: Some("2026-10-01T15:00:00-07:00".into()),
            ends_at: None,
            is_daytime: Some(true),
            temperature: Some(Quantity {
                value: 58.0,
                unit: Unit::Fahrenheit,
            }),
            precipitation_probability_percent: Some(10.into()),
            wind: "W 5 mph".into(),
            condition: Some("Cloudy".into()),
            detail: None,
        }
    }
    #[test]
    fn hourly_periods_print_one_compact_line_each() {
        let forecast = HourlyForecast {
            location: location(),
            units: Units::Us,
            hourly: vec![period(), period()],
            alerts_status: weather_bridge::model::HourlyAlertsStatus::NotChecked,
            sources: HourlySources {
                hourly: IssuedSource {
                    url: "u".into(),
                    issued_at: None,
                },
            },
            warnings: vec![Warning::AlertsNotChecked],
            assembled_at: "t".into(),
            cache_max_age_seconds: 0,
        };
        let text = hourly_text(&forecast);
        let lines: Vec<_> = text.lines().filter(|l| l.contains("Cloudy")).collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert!(
            lines[0].contains("Thu 15:00") && lines[0].contains("58°F"),
            "{text}"
        );
        assert!(
            lines[0].contains("10% rain") && lines[0].contains("W 5 mph"),
            "{text}"
        );
        assert!(text.contains("does not check alerts"), "{text}");
    }
}
