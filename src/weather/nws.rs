//! NWS HTTP client: fixed origin, paced and size-capped fetches, and two cache tiers
//! (documents for two minutes, grid lookups for six hours) with freshness tracking.
mod cache;
pub(super) mod documents;
mod limiter;

pub use cache::{CACHE_SECONDS, with_cache_lifetime};
use limiter::Limiter;

use super::GridPoint;
use crate::{
    Error, ErrorCode,
    diagnostics::{CacheResult, CacheTier, FailureReason, Metrics, UpstreamKind, UpstreamOutcome},
};
use cache::{Cached, POINTS_CACHE_SECONDS, source_expiry};
use documents::{Document, Points, PointsProperties, decode};
use moka::{Expiry, future::Cache};
use reqwest::StatusCode;
use serde_json::Value;
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::time::Instant;

/// Longest one upstream request may take, from connecting until its body is read.
pub(super) const UPSTREAM_REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
/// Upstream starts: a burst covers one uncached report, then one start per second.
const UPSTREAM_BURST: u32 = 6;
const UPSTREAM_INTERVAL: Duration = Duration::from_secs(1);
/// How long after its weather lookup begins a request may still wait for the pace above.
/// Waiting requests start oldest lookup first. A request that could not start within the
/// window is refused at once: the lookups in flight need more starts than the pace allows,
/// so the caller gets `BUSY`, or a warning for an optional source, instead of queueing
/// until the lookup deadline. A request with capacity available starts at any time.
pub(super) const UPSTREAM_WAIT_WINDOW: Duration = Duration::from_secs(6);

/// The weather lookup an upstream request belongs to, for pacing.
#[derive(Clone, Copy)]
struct Lookup {
    /// Lookups in the order they began; a lower number starts its requests first.
    sequence: u64,
    /// The start of the pacing wait window.
    started: Instant,
}
impl Lookup {
    fn begin() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        Self {
            sequence: SEQUENCE.fetch_add(1, Ordering::Relaxed),
            started: Instant::now(),
        }
    }
}
tokio::task_local! {
    static LOOKUP: Lookup;
}
/// Run one weather lookup, so its upstream requests are paced by when it began.
pub(super) async fn lookup<F: Future>(work: F) -> F::Output {
    LOOKUP.scope(Lookup::begin(), work).await
}
/// Largest upstream body accepted, in bytes.
const MAX_BODY_BYTES: usize = 2_000_000;

/// The generic client-facing error for any upstream failure.
pub(super) fn upstream() -> Error {
    Error::new(
        ErrorCode::UpstreamUnavailable,
        "The National Weather Service is temporarily unavailable. Please retry shortly.",
    )
}

#[derive(Clone)]
pub(super) struct Client {
    http: reqwest::Client,
    /// Raw documents, keyed by URL and requested type and validated before admission.
    cache: Cache<String, Cached<Arc<Value>>>,
    points: Cache<String, PointCacheEntry>,
    limiter: Arc<Limiter>,
    metrics: Arc<Metrics>,
    /// The configured NWS origin, such as `https://api.weather.gov`, without a trailing
    /// slash. Requests, the discovered-link check and response sources all use it.
    base: String,
}
#[derive(Clone)]
struct PointCacheEntry {
    cached: Cached<Grid>,
    complete: bool,
}
/// A grid lookup: its properties and the three links it discovered, each checked once
/// against the configured NWS origin. A link is reported, and its absence logged, only
/// when an operation asks for it.
#[derive(Clone)]
pub(super) struct Grid {
    pub(super) properties: PointsProperties,
    forecast: Option<Link>,
    hourly: Option<Link>,
    stations: Option<Link>,
}
/// An upstream path and the endpoint kind it was built or discovered as. The kind is
/// fixed by the constructor and drives metrics and the outside-coverage rule; it is never
/// guessed from the path.
#[derive(Clone)]
pub(super) struct UpstreamPath {
    kind: UpstreamKind,
    path: String,
}
impl UpstreamPath {
    /// The grid lookup for a point.
    fn points(point: GridPoint) -> Self {
        Self {
            kind: UpstreamKind::Points,
            path: format!("/points/{}", point.query()),
        }
    }
    /// Active alerts for a point, at the four-decimal precision alerts use.
    pub(super) fn alerts(latitude: f64, longitude: f64) -> Self {
        Self {
            kind: UpstreamKind::Alerts,
            path: format!("/alerts/active?point={latitude:.4},{longitude:.4}"),
        }
    }
    /// The latest observation from one station.
    pub(super) fn observation(station: &str) -> Self {
        Self {
            kind: UpstreamKind::Observation,
            path: format!("/stations/{station}/observations/latest"),
        }
    }
    /// The path on the NWS origin, including any query.
    pub(super) fn as_str(&self) -> &str {
        &self.path
    }
}
/// A link that a grid lookup can discover.
#[derive(Clone, Copy)]
enum LinkKind {
    Forecast,
    Hourly,
    Stations,
}
impl LinkKind {
    /// The grid lookup property that carries this link, as logged when it is missing.
    fn name(self) -> &'static str {
        match self {
            Self::Forecast => "forecast",
            Self::Hourly => "forecastHourly",
            Self::Stations => "observationStations",
        }
    }
    fn upstream_kind(self) -> UpstreamKind {
        match self {
            Self::Forecast => UpstreamKind::Forecast,
            Self::Hourly => UpstreamKind::Hourly,
            Self::Stations => UpstreamKind::Stations,
        }
    }
}
/// A discovered link on the configured NWS origin.
#[derive(Clone)]
pub(super) struct Link {
    path: UpstreamPath,
    url: String,
}
impl Link {
    /// The upstream path, for fetching.
    pub(super) fn path(&self) -> &UpstreamPath {
        &self.path
    }
    /// The NWS URL, as reported in response sources.
    pub(super) fn url(&self) -> &str {
        &self.url
    }
}
impl Grid {
    /// The daily forecast link.
    pub(super) fn forecast(&self) -> Result<&Link, Error> {
        required_link(&self.forecast, LinkKind::Forecast)
    }
    /// The hourly forecast link.
    pub(super) fn hourly(&self) -> Result<&Link, Error> {
        required_link(&self.hourly, LinkKind::Hourly)
    }
    /// The observation stations link.
    pub(super) fn stations(&self) -> Result<&Link, Error> {
        required_link(&self.stations, LinkKind::Stations)
    }
    fn is_complete(&self) -> bool {
        self.forecast.is_some() && self.hourly.is_some() && self.stations.is_some()
    }
}
/// All outbound paths originate from the configured NWS origin, so a missing link or one
/// elsewhere is an upstream failure.
fn required_link(link: &Option<Link>, kind: LinkKind) -> Result<&Link, Error> {
    link.as_ref().ok_or_else(|| {
        tracing::warn!(
            link = kind.name(),
            "NWS grid lookup link is missing or not on the NWS origin"
        );
        upstream()
    })
}
struct Fetched {
    value: Value,
    kind: UpstreamKind,
    latency: Duration,
}
struct PointExpiry;
impl Expiry<String, PointCacheEntry> for PointExpiry {
    fn expire_after_create(
        &self,
        key: &String,
        value: &PointCacheEntry,
        created_at: std::time::Instant,
    ) -> Option<Duration> {
        let _ = (key, created_at);
        Some(if value.complete {
            Duration::from_secs(POINTS_CACHE_SECONDS)
        } else {
            Duration::ZERO
        })
    }

    fn expire_after_update(
        &self,
        key: &String,
        value: &PointCacheEntry,
        updated_at: std::time::Instant,
        _duration_until_expiry: Option<Duration>,
    ) -> Option<Duration> {
        self.expire_after_create(key, value, updated_at)
    }
}
/// The canonical origin of a configured base URL, or why it is not a plain origin.
fn origin(base: &str) -> anyhow::Result<String> {
    let url = reqwest::Url::parse(base)
        .map_err(|e| anyhow::anyhow!("NWS base URL {base:?} is invalid: {e}"))?;
    let plain = matches!(url.scheme(), "http" | "https")
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none();
    anyhow::ensure!(
        plain,
        "NWS base URL {base:?} must be an http or https origin without a path or query"
    );
    Ok(url.origin().ascii_serialization())
}
impl Client {
    pub(super) fn new(
        base: &str,
        user_agent: Option<String>,
        metrics: Arc<Metrics>,
    ) -> anyhow::Result<Self> {
        let base = origin(base)?;
        Ok(Self {
            http: reqwest::Client::builder()
                .user_agent(
                    user_agent
                        .unwrap_or_else(|| "WeatherBridge/0.1 (open-source Rust demo)".into()),
                )
                .timeout(UPSTREAM_REQUEST_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            cache: Cache::builder()
                .max_capacity(256)
                .time_to_live(Duration::from_secs(CACHE_SECONDS))
                .build(),
            points: Cache::builder()
                .max_capacity(1024)
                .time_to_live(Duration::from_secs(POINTS_CACHE_SECONDS))
                .expire_after(PointExpiry)
                .build(),
            limiter: Arc::new(Limiter::new(UPSTREAM_BURST, UPSTREAM_INTERVAL)),
            metrics,
            base,
        })
    }
    /// The NWS URL of an upstream path, as reported in response sources.
    pub(super) fn public_url(&self, path: &UpstreamPath) -> String {
        format!("{}{}", self.base, path.as_str())
    }
    /// A URL discovered in a grid lookup, if it is on the configured NWS origin. The
    /// link's endpoint kind is the property it came from, whatever its path looks like.
    fn link(&self, kind: LinkKind, url: Option<&str>) -> Option<Link> {
        url.and_then(|url| url.strip_prefix(self.base.as_str()))
            .filter(|path| path.starts_with('/') && !path.starts_with("//"))
            .map(|path| {
                let path = UpstreamPath {
                    kind: kind.upstream_kind(),
                    path: path.to_owned(),
                };
                Link {
                    url: self.public_url(&path),
                    path,
                }
            })
    }
    /// A document, from the two-minute cache or NWS, decoded leniently as `T`.
    pub(super) async fn fetch<T: Document>(&self, path: &UpstreamPath) -> Result<T, Error> {
        let url = self.public_url(path);
        let cache_key = Self::cache_key::<T>(&url);
        let loaded = Arc::new(AtomicBool::new(false));
        let loaded_by_cache = loaded.clone();
        let cached = self
            .cache
            .try_get_with(cache_key, async move {
                loaded_by_cache.store(true, Ordering::Relaxed);
                let fetched = self.fetch_uncached(path).await?;
                let (_, value) = self.validate_fetched::<T>(fetched, path.as_str())?;
                Ok::<_, Error>(Cached::fresh(Arc::new(value), CACHE_SECONDS))
            })
            .await
            .map_err(|e| {
                // A partial response must not pin an upstream outage in a cache.
                source_expiry(Instant::now());
                (*e).clone()
            });
        self.metrics
            .cache(CacheTier::Document, loaded_result(&loaded));
        let cached = cached?;
        source_expiry(cached.expires_at);
        validate(&cached.value, path.as_str(), &self.metrics)
    }
    fn cache_key<T>(url: &str) -> String {
        format!("{}\0{url}", std::any::type_name::<T>())
    }
    fn point_cache_entry(&self, value: Points) -> PointCacheEntry {
        let properties = value.properties;
        let grid = Grid {
            forecast: self.link(LinkKind::Forecast, properties.forecast.as_deref()),
            hourly: self.link(LinkKind::Hourly, properties.forecast_hourly.as_deref()),
            stations: self.link(
                LinkKind::Stations,
                properties.observation_stations.as_deref(),
            ),
            properties,
        };
        PointCacheEntry {
            complete: grid.is_complete(),
            cached: Cached::fresh(grid, POINTS_CACHE_SECONDS),
        }
    }
    /// Grid lookup, cached for hours and reduced to the fields this service uses.
    pub(super) async fn point(&self, point: GridPoint) -> Result<Grid, Error> {
        let path = UpstreamPath::points(point);
        let loaded = Arc::new(AtomicBool::new(false));
        let loaded_by_cache = loaded.clone();
        let cached = self
            .points
            .try_get_with(path.as_str().to_owned(), async move {
                loaded_by_cache.store(true, Ordering::Relaxed);
                let fetched = self.fetch_uncached(&path).await?;
                let (points, _) = self.validate_fetched::<Points>(fetched, path.as_str())?;
                Ok::<_, Error>(self.point_cache_entry(points))
            })
            .await
            .map_err(|e| (*e).clone());
        self.metrics
            .cache(CacheTier::Points, loaded_result(&loaded));
        let cached = cached?;
        source_expiry(cached.cached.expires_at);
        Ok(cached.cached.value)
    }
    /// A grid lookup only if one is already cached; never spends an upstream call.
    pub(super) async fn cached_point(&self, point: GridPoint) -> Option<Grid> {
        let cached = self.points.get(UpstreamPath::points(point).as_str()).await;
        self.metrics
            .cache(CacheTier::Points, CacheResult::hit_if(cached.is_some()));
        let cached = cached?;
        source_expiry(cached.cached.expires_at);
        Some(cached.cached.value)
    }
    /// One upstream request for `path`. Every failure cause is logged here, once per
    /// request (concurrent callers share it), and replaced by the generic client error.
    async fn fetch_uncached(&self, path: &UpstreamPath) -> Result<Fetched, Error> {
        let kind = path.kind;
        let lookup = LOOKUP
            .try_with(|lookup| *lookup)
            .unwrap_or_else(|_| Lookup::begin());
        // A refusal makes no request, so it is neither an upstream outcome nor a failure.
        self.limiter
            .acquire(lookup.sequence, lookup.started + UPSTREAM_WAIT_WINDOW)
            .await
            .map_err(|_| super::busy())?;
        let started = std::time::Instant::now();
        let result = self.fetch_uncached_inner(path).await;
        let latency = started.elapsed();
        match result {
            Ok(value) => Ok(Fetched {
                value,
                kind,
                latency,
            }),
            Err(error) => {
                let outcome = if error.code == ErrorCode::OutsideCoverage {
                    UpstreamOutcome::OutsideCoverage
                } else {
                    UpstreamOutcome::Failure
                };
                self.metrics.upstream_request(kind, outcome, latency);
                Err(error)
            }
        }
    }

    /// A 2xx response is successful only after the requested document type accepts its
    /// required fields. This is also the sole outcome event for that upstream request.
    fn validate_fetched<T: Document>(
        &self,
        fetched: Fetched,
        path: &str,
    ) -> Result<(T, Value), Error> {
        let result = validate::<T>(&fetched.value, path, &self.metrics);
        self.metrics.upstream_request(
            fetched.kind,
            if result.is_ok() {
                UpstreamOutcome::Success
            } else {
                UpstreamOutcome::Failure
            },
            fetched.latency,
        );
        result.map(|document| (document, fetched.value))
    }

    async fn fetch_uncached_inner(&self, upstream: &UpstreamPath) -> Result<Value, Error> {
        let path = upstream.as_str();
        let mut response = self
            .http
            .get(self.public_url(upstream))
            .header("Accept", "application/geo+json")
            .send()
            .await
            .map_err(|e| self.failed(path, Failure::Transport(e)))?;
        let status = response.status();
        if outside_coverage(upstream.kind, status) {
            return Err(Error::new(
                ErrorCode::OutsideCoverage,
                "NWS has no forecast for this location. Try a location in the United States or a supported territory.",
            ));
        }
        if !status.is_success() {
            return Err(self.failed(path, Failure::Status(status)));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| self.failed(path, Failure::Transport(e)))?
        {
            if bytes.len() + chunk.len() > MAX_BODY_BYTES {
                return Err(self.failed(path, Failure::Oversized));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|e| self.failed(path, Failure::Decode(e)))
    }

    fn failed(&self, path: &str, failure: Failure) -> Error {
        self.metrics.upstream_failure(failure.reason());
        tracing::warn!(path = %log_path(path), cause = %failure, "NWS request failed");
        upstream()
    }
}
/// A cache lookup missed when its loader ran.
fn loaded_result(loaded: &AtomicBool) -> CacheResult {
    CacheResult::hit_if(!loaded.load(Ordering::Relaxed))
}
/// Whether `status` is the NWS signal for a location outside its coverage: a 404 from
/// /points, or 400 "out of bounds" from /alerts/active. Any other unsuccessful status,
/// including a 404 from a discovered forecast or station link, is an upstream failure.
fn outside_coverage(kind: UpstreamKind, status: StatusCode) -> bool {
    match kind {
        UpstreamKind::Points => status == StatusCode::NOT_FOUND,
        UpstreamKind::Alerts => status == StatusCode::BAD_REQUEST,
        _ => false,
    }
}
fn validate<T: Document>(value: &Value, path: &str, metrics: &Metrics) -> Result<T, Error> {
    let document: T = decode(value);
    if let Some(field) = document.missing_required(value) {
        metrics.upstream_failure(FailureReason::Required);
        return required(None, path, field);
    }
    Ok(document)
}
/// Why an upstream request failed. Logged for operators, never shown to clients.
enum Failure {
    Transport(reqwest::Error),
    Status(StatusCode),
    Oversized,
    Decode(serde_json::Error),
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(e) => {
                let kind = if e.is_timeout() {
                    "timeout"
                } else if e.is_connect() {
                    "connect"
                } else if e.is_body() {
                    "body"
                } else {
                    "request"
                };
                // The error's own message names the URL, whose query can hold caller
                // coordinates, so only its causes are written; the path is logged apart.
                write!(f, "transport error ({kind})")?;
                let mut source = std::error::Error::source(e);
                while let Some(cause) = source {
                    write!(f, ": {cause}")?;
                    source = cause.source();
                }
                Ok(())
            }
            Self::Status(status) => write!(f, "HTTP status {status}"),
            Self::Oversized => write!(f, "body exceeds {MAX_BODY_BYTES} bytes"),
            Self::Decode(e) => write!(f, "invalid JSON: {e}"),
        }
    }
}
impl Failure {
    fn reason(&self) -> FailureReason {
        match self {
            Self::Transport(_) => FailureReason::Transport,
            Self::Status(_) => FailureReason::Status,
            Self::Oversized => FailureReason::Oversized,
            Self::Decode(_) => FailureReason::Decode,
        }
    }
}
/// The upstream path without its query: alert queries carry the caller's coordinates.
fn log_path(path: &str) -> &str {
    path.split_once('?').map_or(path, |(path, _)| path)
}
/// A field the caller cannot do without, such as forecast periods. A missing one is
/// logged and becomes the generic upstream error.
pub(super) fn required<T>(value: Option<T>, path: &str, field: &'static str) -> Result<T, Error> {
    value.ok_or_else(|| {
        tracing::warn!(path = %log_path(path), missing = %field, "NWS document is missing a required field");
        upstream()
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn client(base: &str) -> Client {
        Client::new(base, None, Arc::new(Metrics::default())).unwrap()
    }
    fn forecast_path(path: &str) -> UpstreamPath {
        UpstreamPath {
            kind: UpstreamKind::Forecast,
            path: path.to_owned(),
        }
    }
    #[test]
    fn grid_links_carry_their_kind_whatever_their_path() {
        let c = client("https://api.weather.gov");
        for kind in [LinkKind::Forecast, LinkKind::Hourly, LinkKind::Stations] {
            let link = c
                .link(kind, Some("https://api.weather.gov/points/1,2"))
                .unwrap();
            assert_eq!(link.path().kind as usize, kind.upstream_kind() as usize);
            assert!(!outside_coverage(link.path().kind, StatusCode::NOT_FOUND));
        }
        assert!(outside_coverage(
            UpstreamPath::points(GridPoint::new(1.0, 2.0)).kind,
            StatusCode::NOT_FOUND
        ));
        assert!(outside_coverage(
            UpstreamPath::alerts(0.0, 0.0).kind,
            StatusCode::BAD_REQUEST
        ));
        assert!(!outside_coverage(
            UpstreamPath::observation("KSEA").kind,
            StatusCode::NOT_FOUND
        ));
    }
    #[test]
    fn base_url_must_be_a_plain_origin() {
        assert_eq!(
            client("https://api.weather.gov/").base,
            "https://api.weather.gov"
        );
        assert_eq!(
            client("http://127.0.0.1:8080").base,
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            client("HTTPS://API.Weather.gov:443").base,
            "https://api.weather.gov"
        );
        for invalid in [
            "api.weather.gov",
            "ftp://api.weather.gov",
            "https://api.weather.gov/v1",
            "https://api.weather.gov?x=1",
            "https://api.weather.gov#x",
            "https://user:pw@api.weather.gov",
        ] {
            assert!(
                Client::new(invalid, None, Arc::new(Metrics::default())).is_err(),
                "{invalid}"
            );
        }
    }
    #[test]
    fn discovered_urls_stay_on_the_configured_origin() {
        let c = client("http://127.0.0.1:8080");
        let link = c
            .link(LinkKind::Forecast, Some("http://127.0.0.1:8080/forecast"))
            .unwrap();
        assert_eq!(link.path().as_str(), "/forecast");
        assert_eq!(link.url(), "http://127.0.0.1:8080/forecast");
        for other in [
            "https://api.weather.gov/forecast",
            "http://127.0.0.1:80800/forecast",
            "http://127.0.0.1:8080//evil.example",
            "https://127.0.0.1:8080/forecast",
        ] {
            assert!(c.link(LinkKind::Forecast, Some(other)).is_none(), "{other}");
        }
        assert_eq!(
            c.public_url(&forecast_path("/forecast")),
            "http://127.0.0.1:8080/forecast"
        );
    }
    #[test]
    fn discovered_urls_stay_on_fixed_origin() {
        let c = client("https://api.weather.gov");
        assert!(c.link(LinkKind::Forecast, None).is_none());
        assert!(
            c.link(LinkKind::Forecast, Some("https://evil.example/forecast"))
                .is_none()
        );
        assert!(
            c.link(
                LinkKind::Forecast,
                Some("https://api.weather.gov//evil.example")
            )
            .is_none()
        );
        assert!(
            c.link(
                LinkKind::Forecast,
                Some("https://api.weather.gov.evil.example/x")
            )
            .is_none()
        );
        assert_eq!(
            c.link(
                LinkKind::Forecast,
                Some("https://api.weather.gov/gridpoints/SEW/1,1/forecast")
            )
            .unwrap()
            .path()
            .as_str(),
            "/gridpoints/SEW/1,1/forecast"
        );
    }
    #[tokio::test]
    async fn cache_lifetime_uses_the_oldest_actual_source_expiry() {
        let c = client("https://api.weather.gov");
        let now = Instant::now();
        for (path, seconds) in [("/old", 7), ("/new", 100)] {
            c.cache
                .insert(
                    Client::cache_key::<Value>(&format!("{}{path}", c.base)),
                    Cached {
                        value: Arc::new(serde_json::json!({ "cached": path })),
                        expires_at: now + Duration::from_secs(seconds),
                    },
                )
                .await;
        }
        let (old, new) = (forecast_path("/old"), forecast_path("/new"));
        let (values, lifetime) = with_cache_lifetime(async {
            tokio::join!(c.fetch::<Value>(&old), c.fetch::<Value>(&new))
        })
        .await;
        assert_eq!(values.0.unwrap()["cached"], "/old");
        assert_eq!(values.1.unwrap()["cached"], "/new");
        assert!(
            (5..=7).contains(&lifetime),
            "oldest source, not a fresh 120 s: {lifetime}"
        );
    }
    #[tokio::test]
    async fn a_failed_source_makes_the_response_uncacheable() {
        // The base points at a closed port, so the fetch fails without contacting NWS.
        let c = client("http://127.0.0.1:9");
        let (result, lifetime) =
            with_cache_lifetime(async { c.fetch::<Value>(&forecast_path("/missing")).await }).await;
        assert!(result.is_err());
        assert_eq!(lifetime, 0);
    }
    #[tokio::test]
    async fn incomplete_point_completion_cannot_remove_a_newer_healthy_entry() {
        let c = client("https://api.weather.gov");
        let key = "/points/47.61,-122.33".to_owned();
        let partial: Points = decode(&serde_json::json!({"properties": {
            "forecastHourly": "https://api.weather.gov/hourly"
        }}));
        let healthy: Points = decode(&serde_json::json!({"properties": {
            "forecast": "https://api.weather.gov/forecast",
            "forecastHourly": "https://api.weather.gov/hourly",
            "observationStations": "https://api.weather.gov/stations"
        }}));

        let older_partial_completion = c
            .points
            .try_get_with(key.clone(), async {
                Ok::<_, Error>(c.point_cache_entry(partial))
            })
            .await;
        let older_partial_completion = older_partial_completion.unwrap();
        assert!(
            c.points.get(&key).await.is_none(),
            "incomplete discovery must expire at insertion"
        );

        let healthy_completion = c
            .points
            .try_get_with(key.clone(), async {
                Ok::<_, Error>(c.point_cache_entry(healthy))
            })
            .await;
        assert!(healthy_completion.unwrap().complete);
        drop(older_partial_completion);
        let reused: Result<PointCacheEntry, Arc<Error>> = c
            .points
            .try_get_with(key, async {
                panic!("a newer healthy entry must remain cached")
            })
            .await;
        assert!(reused.unwrap().complete);
    }
    #[test]
    fn point_expiry_applies_to_creates_updates_and_keeps_read_deadlines() {
        let c = client("https://api.weather.gov");
        let partial = c.point_cache_entry(decode(&serde_json::json!({"properties": {
            "forecastHourly": "https://api.weather.gov/hourly"
        }})));
        let healthy = c.point_cache_entry(decode(&serde_json::json!({"properties": {
            "forecast": "https://api.weather.gov/forecast",
            "forecastHourly": "https://api.weather.gov/hourly",
            "observationStations": "https://api.weather.gov/stations"
        }})));
        let expiry = PointExpiry;
        let key = "point".to_owned();
        let now = std::time::Instant::now();
        assert_eq!(
            expiry.expire_after_create(&key, &partial, now),
            Some(Duration::ZERO)
        );
        assert_eq!(
            expiry.expire_after_update(&key, &partial, now, Some(Duration::from_secs(9))),
            Some(Duration::ZERO)
        );
        assert_eq!(
            expiry.expire_after_update(&key, &healthy, now, Some(Duration::from_secs(9))),
            Some(Duration::from_secs(POINTS_CACHE_SECONDS))
        );
        let remaining = Some(Duration::from_secs(9));
        assert_eq!(
            expiry.expire_after_read(&key, &healthy, now, remaining, now),
            remaining,
            "reads must preserve the existing deadline rather than refresh it"
        );
    }
    #[tokio::test]
    async fn cancelled_point_initialization_allows_a_healthy_retry() {
        let c = client("https://api.weather.gov");
        let cache = c.points.clone();
        let key = "point".to_owned();
        let started = Arc::new(tokio::sync::Semaphore::new(0));
        let task_started = started.clone();
        let task_cache = cache.clone();
        let task_key = key.clone();
        let task = tokio::spawn(async move {
            let _: Result<PointCacheEntry, Arc<Error>> = task_cache
                .try_get_with(task_key, async move {
                    task_started.add_permits(1);
                    std::future::pending::<Result<PointCacheEntry, Error>>().await
                })
                .await;
        });
        let _permit = started.acquire().await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());

        let healthy = c.point_cache_entry(decode(&serde_json::json!({"properties": {
            "forecast": "https://api.weather.gov/forecast",
            "forecastHourly": "https://api.weather.gov/hourly",
            "observationStations": "https://api.weather.gov/stations"
        }})));
        let cached = cache
            .try_get_with(key, async { Ok::<_, Error>(healthy) })
            .await
            .unwrap();
        assert!(cached.complete);
    }
}
