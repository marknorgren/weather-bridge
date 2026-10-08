//! Cache-Control policy for every response class.
use crate::{ErrorCode, weather::CACHE_SECONDS};
use axum::http::HeaderValue;

/// Cache-Control classes. CloudFront honors these on `/v1/*` (capped at 120 s) and
/// browsers honor them everywhere. Anything without an explicit class is `no-store`.
/// This type owns every number and renders the header value in one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CachePolicy {
    /// Shared and browser caches may keep the answer for `max_age` seconds.
    Public {
        max_age: u64,
    },
    /// A complete report, cacheable for the remaining life of its oldest source.
    Report {
        seconds: u64,
    },
    NoStore,
}
impl CachePolicy {
    /// Stable validation, not-found and ambiguity answers.
    pub(super) const API: Self = Self::Public { max_age: 60 };
    pub(super) const CITIES: Self = Self::Public { max_age: 86400 };
    pub(super) const PAGE: Self = Self::Public { max_age: 300 };
    pub(super) const NO_STORE: Self = Self::NoStore;

    /// A report is cacheable only when complete (see `model::is_complete`) and only for the
    /// remaining life of its oldest upstream source, at most the 120-second freshness window.
    /// Browsers and CloudFront therefore never extend data past the in-process promise. A
    /// failed alert check or a partial-source warning is never served again from a cache; the
    /// fixed hourly "alerts not checked" note is informational and does not count.
    pub(super) fn report(complete: bool, lifetime_seconds: u64) -> Self {
        match lifetime_seconds.min(CACHE_SECONDS) {
            seconds @ 1.. if complete => Self::Report { seconds },
            _ => Self::NoStore,
        }
    }
    /// Validation, not-found and ambiguity answers (400, 404, 409) are stable for a given
    /// query. Coverage (422), BUSY, timeouts, upstream failures and request rejections are
    /// transient.
    pub(super) fn for_error(code: ErrorCode) -> Self {
        match code {
            ErrorCode::InvalidLocation | ErrorCode::CityNotFound | ErrorCode::AmbiguousCity => {
                Self::API
            }
            ErrorCode::OutsideCoverage
            | ErrorCode::UpstreamUnavailable
            | ErrorCode::UpstreamTimeout
            | ErrorCode::Busy
            | ErrorCode::Forbidden
            | ErrorCode::RequestTooLarge => Self::NO_STORE,
        }
    }
    /// Seconds a cache may keep the response, or `None` for `no-store`.
    #[cfg(test)]
    pub(super) fn max_age(self) -> Option<u64> {
        match self {
            Self::Public { max_age: s } | Self::Report { seconds: s } => Some(s),
            Self::NoStore => None,
        }
    }
    /// The only place a Cache-Control value is rendered.
    pub(super) fn header_value(self) -> HeaderValue {
        let text = match self {
            Self::Public { max_age } => format!("public, max-age={max_age}"),
            Self::Report { seconds } => format!("public, max-age={seconds}, s-maxage={seconds}"),
            Self::NoStore => "no-store".to_owned(),
        };
        HeaderValue::from_str(&text).expect("cache-control is visible ASCII")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;

    #[tokio::test]
    async fn cache_control_by_response_class() {
        let app = test_app();
        for (path, status, cache) in [
            ("/", 200, "public, max-age=300"),
            ("/developer", 200, "public, max-age=300"),
            ("/assets/weather.js", 200, "public, max-age=300"),
            ("/openapi.json", 200, "public, max-age=300"),
            ("/healthz", 200, "no-store"),
            ("/v1/cities?q=Seattle", 200, "public, max-age=86400"),
            ("/v1/cities", 400, "public, max-age=60"),
            ("/v1/weather?city=Springfield", 409, "public, max-age=60"),
            ("/v1/weather?lat=999&lon=0", 400, "public, max-age=60"),
            ("/v1/weather?city=London,GB", 404, "public, max-age=60"),
            (
                "/v1/alerts?city=Seattle&units=banana",
                400,
                "public, max-age=60",
            ),
            ("/not-a-route", 404, "no-store"),
        ] {
            let response = send(&app, get_with(path, None)).await;
            assert_eq!(response.status().as_u16(), status, "{path}");
            assert_eq!(header(&response, "cache-control"), cache, "{path}");
        }
        let response = send(&app, mcp_init(None)).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(header(&response, "cache-control"), "no-store");
    }
    const FRESH_REPORT: &str = "public, max-age=120, s-maxage=120";
    const NO_STORE: &str = "no-store";
    fn report_cache(complete: bool, lifetime: u64) -> String {
        let value = CachePolicy::report(complete, lifetime).header_value();
        value.to_str().expect("ascii").to_owned()
    }
    #[test]
    fn report_and_error_cache_policies() {
        use crate::model::{AlertsStatus, Warning, is_complete};
        let policy = |status, warnings: &[Warning]| {
            report_cache(is_complete(status, warnings), CACHE_SECONDS)
        };
        // Lifetime follows the oldest source and never exceeds the freshness window.
        assert_eq!(report_cache(true, 7), "public, max-age=7, s-maxage=7");
        assert_eq!(report_cache(true, 999), FRESH_REPORT);
        assert_eq!(report_cache(true, 0), NO_STORE);
        assert_eq!(report_cache(false, 100), NO_STORE);
        assert_eq!(policy(AlertsStatus::Checked, &[]), FRESH_REPORT);
        assert_eq!(policy(AlertsStatus::NotChecked, &[]), FRESH_REPORT);
        assert_eq!(policy(AlertsStatus::Unavailable, &[]), NO_STORE);
        for partial in [
            Warning::HourlyUnavailable,
            Warning::AlertsUnavailable,
            Warning::StationsUnavailable,
            Warning::NoObservation,
        ] {
            assert_eq!(policy(AlertsStatus::Checked, &[partial]), NO_STORE);
        }
        let note = Warning::AlertsNotChecked;
        assert_eq!(policy(AlertsStatus::NotChecked, &[note]), FRESH_REPORT);
        let partial = [note, Warning::HourlyUnavailable];
        assert_eq!(policy(AlertsStatus::NotChecked, &partial), NO_STORE);
        for code in ErrorCode::ALL {
            let expected = match code.http_status() {
                400 | 404 | 409 => "public, max-age=60",
                _ => NO_STORE,
            };
            assert_eq!(
                CachePolicy::for_error(code).header_value(),
                expected,
                "{code:?}"
            );
        }
    }
}
