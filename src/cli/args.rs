//! Command-line grammar. CLI-only types live here so the core library never depends on clap.
use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};
use std::net::SocketAddr;
use weather_bridge::{
    api::OriginVerify,
    weather::{Units, WeatherQuery},
};

/// Split a comma-separated allow-list, rejecting one with no usable entries.
fn parse_list(value: &str) -> Result<Vec<String>, String> {
    let items: Vec<String> = value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if items.is_empty() {
        Err("must contain at least one trusted entry".into())
    } else {
        Ok(items)
    }
}

/// Parses `--origin-verify` like the MCP allow-lists, but builds its own error so a
/// rejected value is never echoed to stderr or logs.
#[derive(Clone)]
struct OriginVerifyParser;
impl clap::builder::TypedValueParser for OriginVerifyParser {
    type Value = OriginVerify;
    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<OriginVerify, clap::Error> {
        let name = arg.map_or_else(|| "--origin-verify".into(), ToString::to_string);
        value
            .to_str()
            .ok_or_else(|| "must be valid UTF-8".to_owned())
            .and_then(OriginVerify::new)
            .map_err(|reason| {
                cmd.clone().error(
                    clap::error::ErrorKind::ValueValidation,
                    format!("invalid value for '{name}': {reason}"),
                )
            })
    }
}

const EXIT_CODES: &str = "\
Exit codes:
  0  success
  1  upstream or other failure (NWS unavailable, timeout, busy, internal error)
  2  usage error, invalid input, unsupported location, city not found or ambiguous
  3  printed, but incomplete: alerts could not be checked or a source is missing

Without --json, errors print to stderr. With --json, the error body prints to stdout.";

#[derive(Parser)]
#[command(
    version = weather_bridge::BUILD_VERSION,
    long_version = weather_bridge::BUILD_VERSION,
    about = "Friendly US weather by city, coordinates, API and MCP",
    after_help = EXIT_CODES
)]
pub struct Cli {
    /// Contact User-Agent for NWS requests; public deployments must set one.
    #[arg(long, env = "WEATHER_BRIDGE_USER_AGENT", global = true)]
    pub user_agent: Option<String>,
    #[command(subcommand)]
    pub command: Command,
}

/// Unit system for `--units`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum UnitsArg {
    /// Fahrenheit and mph.
    #[default]
    Us,
    /// Celsius and km/h.
    Metric,
}
impl From<UnitsArg> for Units {
    fn from(units: UnitsArg) -> Self {
        match units {
            UnitsArg::Us => Self::Us,
            UnitsArg::Metric => Self::Metric,
        }
    }
}

/// Where to get weather: a city name, a city ID, or coordinates. Exactly one.
#[derive(Args, Debug)]
#[command(group(
    ArgGroup::new("location")
        .required(true)
        .multiple(true)
        .args(["city", "city_id", "lat", "lon"])
))]
pub struct Place {
    /// Exact city name, optionally with a state: "Seattle, WA".
    #[arg(conflicts_with_all = ["city_id", "lat", "lon"])]
    pub city: Option<String>,
    /// GeoNames city ID, as listed by `cities`.
    #[arg(long, value_name = "ID", conflicts_with_all = ["lat", "lon"])]
    pub city_id: Option<u64>,
    /// Latitude in degrees; requires --lon.
    #[arg(long, requires = "lon", allow_hyphen_values = true)]
    pub lat: Option<f64>,
    /// Longitude in degrees; requires --lat.
    #[arg(long, requires = "lat", allow_hyphen_values = true)]
    pub lon: Option<f64>,
}

/// Options shared by `weather`, `hourly` and `alerts`.
#[derive(Args, Debug)]
pub struct Lookup {
    #[command(flatten)]
    pub place: Place,
    /// Unit system for temperatures and wind.
    #[arg(long, value_enum, default_value_t)]
    pub units: UnitsArg,
    /// Print the REST response envelope as JSON.
    #[arg(long)]
    pub json: bool,
}
impl Lookup {
    pub fn query(&self) -> WeatherQuery {
        WeatherQuery {
            city: self.place.city.clone(),
            city_id: self.place.city_id,
            lat: self.place.lat,
            lon: self.place.lon,
            units: self.units.into(),
        }
    }
}

#[derive(Subcommand)]
pub enum Command {
    /// Serve the city explorer, REST API, and MCP at /mcp.
    Serve {
        #[arg(long, env = "WEATHER_BRIDGE_BIND", default_value = "127.0.0.1:8790")]
        bind: SocketAddr,
        /// Comma-separated trusted MCP Host values. Default: loopback only.
        #[arg(long, env = "WEATHER_BRIDGE_MCP_HOSTS", value_parser = parse_list)]
        mcp_hosts: Option<::std::vec::Vec<String>>,
        /// Comma-separated trusted MCP Origin values. Default: loopback only.
        #[arg(long, env = "WEATHER_BRIDGE_MCP_ORIGINS", value_parser = parse_list)]
        mcp_origins: Option<::std::vec::Vec<String>>,
        /// Shared value CloudFront sends as X-Weather-Bridge-Origin-Verify (32+ visible
        /// ASCII characters). When set, other requests except /healthz get 403.
        #[arg(long, env = "WEATHER_BRIDGE_ORIGIN_VERIFY", hide_env_values = true, value_parser = OriginVerifyParser)]
        origin_verify: Option<OriginVerify>,
        /// HTTPS base URL of the developer docs linked from /developer. A trailing / is added.
        #[arg(long, env = "WEATHER_BRIDGE_DOCS_URL", default_value = weather_bridge::api::DEFAULT_DOCS_URL, value_parser = weather_bridge::api::DocsUrl::new)]
        docs_url: weather_bridge::api::DocsUrl,
    },
    /// Current observation, forecast and alerts, e.g. weather "Seattle, WA".
    Weather(Lookup),
    /// Next 24 hourly forecast periods. Alerts are not checked.
    Hourly(Lookup),
    /// Active alerts, reporting whether they could be checked.
    Alerts(Lookup),
    /// Search city names without making network requests.
    Cities {
        /// City name or prefix, optionally with a state: "Spring" or "Springfield, IL".
        query: String,
        /// Print the REST response envelope as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Run the MCP server over stdio. Logs go to stderr.
    Mcp,
}

#[cfg(test)]
mod tests {
    use super::*;
    const SECRET: &str = "0123456789abcdefghijklmnopqrstuvwxyzABCD";
    #[test]
    fn allow_lists_reject_empty_values_instead_of_panicking() {
        assert_eq!(
            parse_list(" mcp.example.com, ,api.example.com ").unwrap(),
            ["mcp.example.com", "api.example.com"]
        );
        assert!(parse_list(" , ").is_err());
    }
    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("weather-bridge").chain(args.iter().copied()))
    }
    fn lookup(args: &[&str]) -> Lookup {
        match parse(args).unwrap().command {
            Command::Weather(l) | Command::Hourly(l) | Command::Alerts(l) => l,
            _ => panic!("not a lookup command"),
        }
    }
    #[test]
    fn origin_verify_is_parsed_once_and_short_values_fail_at_startup() {
        let cli = parse(&["serve", "--origin-verify", SECRET]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Serve {
                origin_verify: Some(_),
                ..
            }
        ));
        let cli = parse(&["serve"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Serve {
                origin_verify: None,
                ..
            }
        ));
        let short = "too-short-to-be-a-secret";
        let error = parse(&["serve", "--origin-verify", short])
            .err()
            .expect("a short value must stop startup");
        assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
        let rendered = error.to_string();
        assert!(rendered.contains("at least 32 characters"), "{rendered}");
        assert!(!rendered.contains(short), "{rendered}");
    }
    #[test]
    fn each_location_mode_builds_a_query() {
        let q = lookup(&["weather", "Seattle, WA"]).query();
        assert_eq!(q.city.as_deref(), Some("Seattle, WA"));
        assert!(q.city_id.is_none() && q.lat.is_none() && q.lon.is_none());

        let q = lookup(&["hourly", "--city-id", "5809844", "--units", "metric"]).query();
        assert_eq!(q.city_id, Some(5809844));
        assert!(matches!(q.units, Units::Metric));

        let q = lookup(&["alerts", "--lat", "47.6", "--lon", "-122.3"]).query();
        assert_eq!((q.lat, q.lon), (Some(47.6), Some(-122.3)));
        assert!(q.city.is_none() && matches!(q.units, Units::Us));
    }
    #[test]
    fn invalid_location_combinations_are_usage_errors() {
        for args in [
            &["weather"][..],
            &["weather", "Seattle", "--city-id", "1"],
            &["weather", "Seattle", "--lat", "1", "--lon", "2"],
            &["weather", "--city-id", "1", "--lat", "1", "--lon", "2"],
            &["weather", "--lat", "1"],
            &["weather", "--lon", "2"],
            &["weather", "Seattle", "--lon", "2"],
            &["hourly", "--lat", "x", "--lon", "2"],
            &["alerts", "Seattle", "--units", "kelvin"],
        ] {
            let error = parse(args).err().unwrap_or_else(|| panic!("{args:?}"));
            assert_eq!(error.exit_code(), 2, "{args:?}");
        }
    }
    #[test]
    fn units_are_a_value_enum() {
        assert_eq!(lookup(&["weather", "Seattle"]).units, UnitsArg::Us);
        assert_eq!(
            lookup(&["weather", "Seattle", "--units", "metric"]).units,
            UnitsArg::Metric
        );
    }
}
