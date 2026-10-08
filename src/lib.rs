pub mod api;
pub mod cities;
pub(crate) mod diagnostics;
pub mod mcp;
pub mod model;
pub mod weather;

use cities::City;
use rmcp::schemars;
use serde::Serialize;
use serde_json::Value;

/// Source revision embedded by `build.rs`, or `unknown` for an ordinary local build.
pub const BUILD_REVISION: &str = env!("WEATHER_BRIDGE_BUILD_REVISION_EMBEDDED");
/// User-facing build identity used by the CLI and runtime diagnostics.
pub const BUILD_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (revision ",
    env!("WEATHER_BRIDGE_BUILD_REVISION_EMBEDDED"),
    ")"
);

/// Stable machine-readable error codes. Each code has exactly one HTTP status, defined in
/// [`ErrorCode::http_status`]. The HTTP layer (`api`) builds responses from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[schemars(
    description = "Each code has one HTTP status: INVALID_LOCATION 400, FORBIDDEN 403, CITY_NOT_FOUND 404, AMBIGUOUS_CITY 409, REQUEST_TOO_LARGE 413, OUTSIDE_COVERAGE 422, UPSTREAM_UNAVAILABLE 502, BUSY 503, UPSTREAM_TIMEOUT 504."
)]
pub enum ErrorCode {
    /// Any invalid request input: unknown or malformed query parameters, an invalid location or an unreadable request body.
    InvalidLocation,
    /// No exact city match; choices hold suggestions when available.
    CityNotFound,
    /// Several cities share the name; choices hold the candidates.
    AmbiguousCity,
    /// NWS does not cover the location.
    OutsideCoverage,
    /// NWS failed or returned an unusable response.
    UpstreamUnavailable,
    /// The weather lookup or HTTP request deadline passed.
    UpstreamTimeout,
    /// Too many weather lookups are in flight.
    Busy,
    /// The request did not come through the public address.
    Forbidden,
    /// The HTTP request body exceeded the size limit.
    RequestTooLarge,
}
impl ErrorCode {
    /// Every code, for exhaustive contract checks.
    pub const ALL: [Self; 9] = [
        Self::InvalidLocation,
        Self::CityNotFound,
        Self::AmbiguousCity,
        Self::OutsideCoverage,
        Self::UpstreamUnavailable,
        Self::UpstreamTimeout,
        Self::Busy,
        Self::Forbidden,
        Self::RequestTooLarge,
    ];
    /// The single code-to-HTTP-status mapping, as a numeric status code.
    pub fn http_status(self) -> u16 {
        match self {
            Self::InvalidLocation => 400,
            Self::CityNotFound => 404,
            Self::AmbiguousCity => 409,
            Self::OutsideCoverage => 422,
            Self::UpstreamUnavailable => 502,
            Self::UpstreamTimeout => 504,
            Self::Busy => 503,
            Self::Forbidden => 403,
            Self::RequestTooLarge => 413,
        }
    }
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("{detail}")]
pub struct Error {
    pub code: ErrorCode,
    pub detail: String,
    /// City candidates for `CITY_NOT_FOUND` suggestions and `AMBIGUOUS_CITY`.
    pub choices: Option<Vec<City>>,
}
/// Wire shape of an error response: `{"errors": [ErrorObject]}`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ErrorBody {
    #[schemars(extend("minItems" = 1))]
    pub errors: Vec<ErrorObject>,
}
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ErrorObject {
    /// HTTP status as a string, e.g. "409".
    #[schemars(extend("description" = "HTTP status as a string, e.g. \"409\"."))]
    pub status: String,
    pub code: ErrorCode,
    pub detail: String,
    #[schemars(extend("description" = "Candidates for AMBIGUOUS_CITY and suggestions for CITY_NOT_FOUND; otherwise null."))]
    pub choices: Option<Vec<City>>,
}
impl Error {
    pub fn new(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
            choices: None,
        }
    }
    pub fn invalid(detail: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidLocation, detail)
    }
    pub fn with_choices(mut self, choices: Vec<City>) -> Self {
        self.choices = Some(choices);
        self
    }
    pub fn to_body(&self) -> ErrorBody {
        ErrorBody {
            errors: vec![ErrorObject {
                status: self.code.http_status().to_string(),
                code: self.code,
                detail: self.detail.clone(),
                choices: self.choices.clone(),
            }],
        }
    }
    /// The error body as JSON, with keys in the same order as every other response.
    pub fn body(&self) -> Value {
        to_json(&self.to_body())
    }
}
/// Serialize a response type to a JSON value. All response types are plain structs with
/// string keys, so this cannot fail.
pub(crate) fn to_json(value: &impl Serialize) -> Value {
    serde_json::to_value(value).expect("response types serialize to JSON")
}
/// Wrap response data with attribution metadata, as JSON.
pub fn envelope(data: impl Serialize) -> Value {
    to_json(&model::Envelope {
        data,
        meta: model::Meta::default(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn error_codes_keep_their_wire_spelling_and_status() {
        let expected = [
            ("INVALID_LOCATION", 400),
            ("CITY_NOT_FOUND", 404),
            ("AMBIGUOUS_CITY", 409),
            ("OUTSIDE_COVERAGE", 422),
            ("UPSTREAM_UNAVAILABLE", 502),
            ("UPSTREAM_TIMEOUT", 504),
            ("BUSY", 503),
            ("FORBIDDEN", 403),
            ("REQUEST_TOO_LARGE", 413),
        ];
        for (code, (name, status)) in ErrorCode::ALL.into_iter().zip(expected) {
            assert_eq!(to_json(&code), name);
            assert_eq!(code.http_status(), status, "{name}");
            assert_eq!(
                Error::new(code, "detail").body(),
                json!({"errors":[{"status":status.to_string(),"code":name,"detail":"detail","choices":null}]})
            );
        }
    }
}
