//! Small, process-local operational metrics with fixed labels and bounded storage.
//! No caller input is ever used as a label value.
use std::{
    fmt::Write,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

const HTTP_BUCKETS: [f64; 10] = [0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0];
const UPSTREAM_BUCKETS: [f64; 8] = [0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 12.0];

#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum HttpRoute {
    Home,
    Developer,
    Assets,
    Health,
    Version,
    Metrics,
    OpenApi,
    Cities,
    Weather,
    Hourly,
    Alerts,
    Mcp,
    Unmatched,
}
const HTTP_ROUTES: [(&str, HttpRoute); 13] = [
    ("home", HttpRoute::Home),
    ("developer", HttpRoute::Developer),
    ("assets", HttpRoute::Assets),
    ("health", HttpRoute::Health),
    ("version", HttpRoute::Version),
    ("metrics", HttpRoute::Metrics),
    ("openapi", HttpRoute::OpenApi),
    ("cities", HttpRoute::Cities),
    ("weather", HttpRoute::Weather),
    ("hourly", HttpRoute::Hourly),
    ("alerts", HttpRoute::Alerts),
    ("mcp", HttpRoute::Mcp),
    ("unmatched", HttpRoute::Unmatched),
];

impl HttpRoute {
    pub(crate) fn from_matched(path: Option<&str>) -> Self {
        match path {
            Some("/") => Self::Home,
            Some("/developer") => Self::Developer,
            Some("/assets/weather.js" | "/assets/developer.js") => Self::Assets,
            Some("/healthz") => Self::Health,
            Some("/version") => Self::Version,
            Some("/metrics") => Self::Metrics,
            Some("/openapi.json") => Self::OpenApi,
            Some("/v1/cities") => Self::Cities,
            Some("/v1/weather") => Self::Weather,
            Some("/v1/forecast/hourly") => Self::Hourly,
            Some("/v1/alerts") => Self::Alerts,
            Some("/mcp" | "/mcp/") => Self::Mcp,
            _ => Self::Unmatched,
        }
    }

    pub(crate) fn matched_path(self) -> &'static str {
        match self {
            Self::Home => "/",
            Self::Developer => "/developer",
            Self::Assets => "/assets/{asset}",
            Self::Health => "/healthz",
            Self::Version => "/version",
            Self::Metrics => "/metrics",
            Self::OpenApi => "/openapi.json",
            Self::Cities => "/v1/cities",
            Self::Weather => "/v1/weather",
            Self::Hourly => "/v1/forecast/hourly",
            Self::Alerts => "/v1/alerts",
            Self::Mcp => "/mcp",
            Self::Unmatched => "<unmatched>",
        }
    }
}

#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum UpstreamKind {
    Points,
    Forecast,
    Hourly,
    Alerts,
    Stations,
    Observation,
    /// No request uses this kind now that each path carries its kind from construction;
    /// its series stays so the rendered metric set does not change.
    Other,
}
const UPSTREAM_KINDS: [(&str, UpstreamKind); 7] = [
    ("points", UpstreamKind::Points),
    ("forecast", UpstreamKind::Forecast),
    ("hourly", UpstreamKind::Hourly),
    ("alerts", UpstreamKind::Alerts),
    ("stations", UpstreamKind::Stations),
    ("observation", UpstreamKind::Observation),
    ("other", UpstreamKind::Other),
];

/// The class of an HTTP response status.
#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum StatusClass {
    Informational,
    Success,
    Redirection,
    ClientError,
    ServerError,
    Other,
}
const STATUS_CLASSES: [(&str, StatusClass); 6] = [
    ("1xx", StatusClass::Informational),
    ("2xx", StatusClass::Success),
    ("3xx", StatusClass::Redirection),
    ("4xx", StatusClass::ClientError),
    ("5xx", StatusClass::ServerError),
    ("other", StatusClass::Other),
];

impl StatusClass {
    pub(crate) fn from_status(status: u16) -> Self {
        match status / 100 {
            1 => Self::Informational,
            2 => Self::Success,
            3 => Self::Redirection,
            4 => Self::ClientError,
            5 => Self::ServerError,
            _ => Self::Other,
        }
    }
}

/// How one upstream request ended.
#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum UpstreamOutcome {
    Success,
    OutsideCoverage,
    Failure,
}
const UPSTREAM_OUTCOMES: [(&str, UpstreamOutcome); 3] = [
    ("success", UpstreamOutcome::Success),
    ("outside_coverage", UpstreamOutcome::OutsideCoverage),
    ("failure", UpstreamOutcome::Failure),
];

/// Why an upstream request failed.
#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum FailureReason {
    Transport,
    Status,
    Oversized,
    Decode,
    Required,
}
const FAILURE_REASONS: [(&str, FailureReason); 5] = [
    ("transport", FailureReason::Transport),
    ("status", FailureReason::Status),
    ("oversized", FailureReason::Oversized),
    ("decode", FailureReason::Decode),
    ("required", FailureReason::Required),
];

/// An in-process NWS cache.
#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum CacheTier {
    /// Raw documents, kept for two minutes.
    Document,
    /// Grid lookups, kept for hours.
    Points,
}
const CACHE_TIERS: [(&str, CacheTier); 2] = [
    ("document", CacheTier::Document),
    ("points", CacheTier::Points),
];
/// Whether a cache lookup found its entry.
#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum CacheResult {
    Hit,
    Miss,
}
const CACHE_RESULTS: [(&str, CacheResult); 2] =
    [("hit", CacheResult::Hit), ("miss", CacheResult::Miss)];

impl CacheResult {
    pub(crate) fn hit_if(hit: bool) -> Self {
        if hit { Self::Hit } else { Self::Miss }
    }
}

/// A weather operation, for counting partial reports.
#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum Operation {
    Weather,
    Hourly,
    Alerts,
}
const OPERATIONS: [(&str, Operation); 3] = [
    ("weather", Operation::Weather),
    ("hourly", Operation::Hourly),
    ("alerts", Operation::Alerts),
];

pub(crate) struct Metrics {
    http_requests: Box<[AtomicU64]>,
    http_duration_buckets: Box<[AtomicU64]>,
    http_duration_sum_micros: Box<[AtomicU64]>,
    upstream_requests: Box<[AtomicU64]>,
    upstream_duration_buckets: Box<[AtomicU64]>,
    upstream_duration_sum_micros: Box<[AtomicU64]>,
    upstream_failures: Box<[AtomicU64]>,
    cache_access: Box<[AtomicU64]>,
    partial_reports: Box<[AtomicU64]>,
    busy: AtomicU64,
}

fn counters(count: usize) -> Box<[AtomicU64]> {
    (0..count).map(|_| AtomicU64::new(0)).collect()
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            http_requests: counters(HTTP_ROUTES.len() * STATUS_CLASSES.len()),
            http_duration_buckets: counters(HTTP_ROUTES.len() * (HTTP_BUCKETS.len() + 1)),
            http_duration_sum_micros: counters(HTTP_ROUTES.len()),
            upstream_requests: counters(UPSTREAM_KINDS.len() * UPSTREAM_OUTCOMES.len()),
            upstream_duration_buckets: counters(
                UPSTREAM_KINDS.len() * (UPSTREAM_BUCKETS.len() + 1),
            ),
            upstream_duration_sum_micros: counters(UPSTREAM_KINDS.len()),
            upstream_failures: counters(FAILURE_REASONS.len()),
            cache_access: counters(CACHE_TIERS.len() * CACHE_RESULTS.len()),
            partial_reports: counters(OPERATIONS.len()),
            busy: AtomicU64::new(0),
        }
    }
}

impl Metrics {
    pub(crate) fn http_response(&self, route: HttpRoute, class: StatusClass, latency: Duration) {
        self.http_requests[route as usize * STATUS_CLASSES.len() + class as usize]
            .fetch_add(1, Ordering::Relaxed);
        observe_histogram(
            &self.http_duration_buckets,
            route as usize,
            &HTTP_BUCKETS,
            latency,
        );
        self.http_duration_sum_micros[route as usize].fetch_add(micros(latency), Ordering::Relaxed);
    }

    pub(crate) fn upstream_request(
        &self,
        kind: UpstreamKind,
        outcome: UpstreamOutcome,
        latency: Duration,
    ) {
        self.upstream_requests[kind as usize * UPSTREAM_OUTCOMES.len() + outcome as usize]
            .fetch_add(1, Ordering::Relaxed);
        observe_histogram(
            &self.upstream_duration_buckets,
            kind as usize,
            &UPSTREAM_BUCKETS,
            latency,
        );
        self.upstream_duration_sum_micros[kind as usize]
            .fetch_add(micros(latency), Ordering::Relaxed);
    }

    pub(crate) fn upstream_failure(&self, reason: FailureReason) {
        self.upstream_failures[reason as usize].fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn cache(&self, tier: CacheTier, result: CacheResult) {
        self.cache_access[tier as usize * CACHE_RESULTS.len() + result as usize]
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn partial(&self, operation: Operation) {
        self.partial_reports[operation as usize].fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn busy(&self) {
        self.busy.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn render(&self) -> String {
        let mut out = String::with_capacity(16_384);
        help(
            &mut out,
            "weather_bridge_build_info",
            "Build identity for this process",
            "gauge",
        );
        writeln!(
            out,
            "weather_bridge_build_info{{version=\"{}\",revision=\"{}\"}} 1",
            env!("CARGO_PKG_VERSION"),
            crate::BUILD_REVISION
        )
        .unwrap();

        help(
            &mut out,
            "weather_bridge_http_requests_total",
            "HTTP responses by matched route and status class",
            "counter",
        );
        for (route_name, route) in HTTP_ROUTES {
            for (class_name, class) in STATUS_CLASSES {
                writeln!(out, "weather_bridge_http_requests_total{{route=\"{route_name}\",status_class=\"{class_name}\"}} {}", load(&self.http_requests, route as usize * STATUS_CLASSES.len() + class as usize)).unwrap();
            }
        }
        render_histograms(
            &mut out,
            "weather_bridge_http_request_duration_seconds",
            "HTTP response latency by matched route",
            "route",
            &HTTP_ROUTES,
            (
                &HTTP_BUCKETS,
                &self.http_duration_buckets,
                &self.http_duration_sum_micros,
            ),
        );

        help(
            &mut out,
            "weather_bridge_upstream_requests_total",
            "NWS request outcomes by fixed endpoint kind",
            "counter",
        );
        for (kind_name, kind) in UPSTREAM_KINDS {
            for (outcome_name, outcome) in UPSTREAM_OUTCOMES {
                writeln!(out, "weather_bridge_upstream_requests_total{{kind=\"{kind_name}\",outcome=\"{outcome_name}\"}} {}", load(&self.upstream_requests, kind as usize * UPSTREAM_OUTCOMES.len() + outcome as usize)).unwrap();
            }
        }
        render_histograms(
            &mut out,
            "weather_bridge_upstream_request_duration_seconds",
            "NWS request latency by fixed endpoint kind",
            "kind",
            &UPSTREAM_KINDS,
            (
                &UPSTREAM_BUCKETS,
                &self.upstream_duration_buckets,
                &self.upstream_duration_sum_micros,
            ),
        );

        help(
            &mut out,
            "weather_bridge_upstream_failures_total",
            "NWS failure causes",
            "counter",
        );
        for (name, reason) in FAILURE_REASONS {
            writeln!(
                out,
                "weather_bridge_upstream_failures_total{{reason=\"{name}\"}} {}",
                load(&self.upstream_failures, reason as usize)
            )
            .unwrap();
        }
        help(
            &mut out,
            "weather_bridge_cache_access_total",
            "In-process NWS cache access by tier and result",
            "counter",
        );
        for (tier_name, tier) in CACHE_TIERS {
            for (result_name, result) in CACHE_RESULTS {
                writeln!(out, "weather_bridge_cache_access_total{{tier=\"{tier_name}\",result=\"{result_name}\"}} {}", load(&self.cache_access, tier as usize * CACHE_RESULTS.len() + result as usize)).unwrap();
            }
        }
        help(
            &mut out,
            "weather_bridge_partial_reports_total",
            "Successful responses missing an optional weather source",
            "counter",
        );
        for (name, operation) in OPERATIONS {
            writeln!(
                out,
                "weather_bridge_partial_reports_total{{operation=\"{name}\"}} {}",
                load(&self.partial_reports, operation as usize)
            )
            .unwrap();
        }
        help(
            &mut out,
            "weather_bridge_busy_total",
            "Weather operations rejected by admission control",
            "counter",
        );
        writeln!(
            out,
            "weather_bridge_busy_total {}",
            self.busy.load(Ordering::Relaxed)
        )
        .unwrap();
        out
    }
}

fn micros(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

fn observe_histogram(storage: &[AtomicU64], series: usize, buckets: &[f64], value: Duration) {
    let seconds = value.as_secs_f64();
    let width = buckets.len() + 1;
    let bucket = buckets
        .iter()
        .position(|upper| seconds <= *upper)
        .unwrap_or(buckets.len());
    storage[series * width + bucket].fetch_add(1, Ordering::Relaxed);
}

fn render_histograms<E: Copy + Into<usize>>(
    out: &mut String,
    name: &str,
    description: &str,
    label: &str,
    series: &[(&str, E)],
    histogram: (&[f64], &[AtomicU64], &[AtomicU64]),
) {
    let (buckets, counts, sums) = histogram;
    help(out, name, description, "histogram");
    let width = buckets.len() + 1;
    for (series_name, series_id) in series {
        let series_id = (*series_id).into();
        // Each observation increments one exclusive bucket. Loading every bucket once and
        // accumulating the snapshot locally makes all rendered buckets monotonic and keeps
        // +Inf identical to _count even while observations are arriving.
        let snapshot: Vec<u64> = (0..width)
            .map(|bucket| load(counts, series_id * width + bucket))
            .collect();
        let mut cumulative = 0;
        for (bucket, upper) in buckets.iter().enumerate() {
            cumulative += snapshot[bucket];
            writeln!(
                out,
                "{name}_bucket{{{label}=\"{series_name}\",le=\"{upper}\"}} {}",
                cumulative
            )
            .unwrap();
        }
        let count = cumulative + snapshot[buckets.len()];
        writeln!(
            out,
            "{name}_bucket{{{label}=\"{series_name}\",le=\"+Inf\"}} {count}"
        )
        .unwrap();
        writeln!(
            out,
            "{name}_sum{{{label}=\"{series_name}\"}} {:.6}",
            load(sums, series_id) as f64 / 1_000_000.0
        )
        .unwrap();
        writeln!(out, "{name}_count{{{label}=\"{series_name}\"}} {count}").unwrap();
    }
}

impl From<HttpRoute> for usize {
    fn from(value: HttpRoute) -> Self {
        value as usize
    }
}
impl From<UpstreamKind> for usize {
    fn from(value: UpstreamKind) -> Self {
        value as usize
    }
}

fn help(out: &mut String, name: &str, description: &str, metric_type: &str) {
    writeln!(out, "# HELP {name} {description}").unwrap();
    writeln!(out, "# TYPE {name} {metric_type}").unwrap();
}

fn load(counters: &[AtomicU64], index: usize) -> u64 {
    counters[index].load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observations_increment_one_exclusive_histogram_bucket() {
        let storage = counters(HTTP_BUCKETS.len() + 1);
        observe_histogram(&storage, 0, &HTTP_BUCKETS, Duration::from_millis(70));
        let observed: Vec<_> = storage
            .iter()
            .map(|counter| counter.load(Ordering::Relaxed))
            .collect();
        assert_eq!(observed.iter().sum::<u64>(), 1, "{observed:?}");
        assert_eq!(observed[2], 1, "70 ms belongs only in the <= 0.1 s bucket");
    }

    #[test]
    fn rendering_accumulates_one_snapshot_and_matches_count_to_infinity() {
        let storage = counters(HTTP_BUCKETS.len() + 1);
        storage[0].store(2, Ordering::Relaxed);
        storage[1].store(3, Ordering::Relaxed);
        storage[HTTP_BUCKETS.len()].store(5, Ordering::Relaxed);
        let sums = counters(1);
        let mut output = String::new();
        render_histograms(
            &mut output,
            "test_duration_seconds",
            "test",
            "route",
            &[("home", HttpRoute::Home)],
            (&HTTP_BUCKETS, &storage, &sums),
        );
        assert!(
            output.contains("test_duration_seconds_bucket{route=\"home\",le=\"0.05\"} 5"),
            "{output}"
        );
        assert!(
            output.contains("test_duration_seconds_bucket{route=\"home\",le=\"+Inf\"} 10"),
            "{output}"
        );
        assert!(
            output.contains("test_duration_seconds_count{route=\"home\"} 10"),
            "{output}"
        );
    }

    #[test]
    fn typed_labels_count_their_own_series() {
        let metrics = Metrics::default();
        metrics.upstream_request(
            UpstreamKind::Points,
            UpstreamOutcome::OutsideCoverage,
            Duration::ZERO,
        );
        for _ in 0..2 {
            metrics.upstream_request(
                UpstreamKind::Alerts,
                UpstreamOutcome::Failure,
                Duration::ZERO,
            );
        }
        for (count, (_, reason)) in FAILURE_REASONS.into_iter().enumerate() {
            for _ in 0..=count {
                metrics.upstream_failure(reason);
            }
        }
        metrics.cache(CacheTier::Points, CacheResult::Hit);
        for _ in 0..2 {
            metrics.cache(CacheTier::Points, CacheResult::Miss);
        }
        let output = metrics.render();
        for expected in [
            "weather_bridge_upstream_requests_total{kind=\"points\",outcome=\"success\"} 0",
            "weather_bridge_upstream_requests_total{kind=\"points\",outcome=\"outside_coverage\"} 1",
            "weather_bridge_upstream_requests_total{kind=\"alerts\",outcome=\"failure\"} 2",
            "weather_bridge_upstream_failures_total{reason=\"transport\"} 1",
            "weather_bridge_upstream_failures_total{reason=\"status\"} 2",
            "weather_bridge_upstream_failures_total{reason=\"oversized\"} 3",
            "weather_bridge_upstream_failures_total{reason=\"decode\"} 4",
            "weather_bridge_upstream_failures_total{reason=\"required\"} 5",
            "weather_bridge_cache_access_total{tier=\"document\",result=\"hit\"} 0",
            "weather_bridge_cache_access_total{tier=\"points\",result=\"hit\"} 1",
            "weather_bridge_cache_access_total{tier=\"points\",result=\"miss\"} 2",
        ] {
            assert!(
                output.contains(&format!("{expected}\n")),
                "missing {expected}:\n{output}"
            );
        }
    }

    #[test]
    fn every_status_maps_to_its_rendered_status_class() {
        let metrics = Metrics::default();
        for (status, times) in [
            (0, 1),
            (99, 1),
            (100, 2),
            (199, 1),
            (200, 3),
            (299, 1),
            (301, 5),
            (404, 6),
            (499, 1),
            (503, 7),
            (600, 1),
            (u16::MAX, 1),
        ] {
            for _ in 0..times {
                metrics.http_response(
                    HttpRoute::Weather,
                    StatusClass::from_status(status),
                    Duration::ZERO,
                );
            }
        }
        let output = metrics.render();
        for (class, count) in [
            ("1xx", 3),
            ("2xx", 4),
            ("3xx", 5),
            ("4xx", 7),
            ("5xx", 7),
            ("other", 4),
        ] {
            let expected = format!(
                "weather_bridge_http_requests_total{{route=\"weather\",status_class=\"{class}\"}} {count}\n"
            );
            assert!(output.contains(&expected), "missing {expected}:\n{output}");
        }
        assert!(
            output.contains(
                "weather_bridge_http_requests_total{route=\"home\",status_class=\"other\"} 0\n"
            ),
            "{output}"
        );
    }
}
