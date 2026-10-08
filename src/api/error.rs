//! HTTP rendering of service errors. The code-to-status mapping itself lives in
//! [`ErrorCode::http_status`], which has no web-stack dependency.
use crate::{Error, ErrorCode};
use axum::{
    Json,
    http::{
        HeaderValue, Method, StatusCode,
        header::{ACCESS_CONTROL_ALLOW_ORIGIN, CACHE_CONTROL, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
};

/// Give REST callers the documented envelope for errors produced before the handler.
/// Only replace non-JSON transport errors; service errors and MCP retain their bodies.
/// This does not read or buffer response bodies, including streaming MCP responses.
pub(super) async fn rest_transport_errors(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let rest = request.uri().path().starts_with("/v1/");
    let public_read = matches!(*request.method(), Method::GET | Method::HEAD);
    let response = next.run(request).await;
    if !rest
        || response
            .headers()
            .get(CONTENT_TYPE)
            .is_some_and(|value| value.as_bytes().starts_with(b"application/json"))
    {
        return response;
    }
    let (code, detail) = match response.status() {
        StatusCode::BAD_REQUEST => (
            ErrorCode::InvalidLocation,
            "Request body could not be read.".to_owned(),
        ),
        StatusCode::PAYLOAD_TOO_LARGE => (
            ErrorCode::RequestTooLarge,
            format!(
                "Request body must not exceed {}.",
                super::max_request_body_text()
            ),
        ),
        StatusCode::GATEWAY_TIMEOUT => (
            ErrorCode::UpstreamTimeout,
            "The HTTP request deadline was exceeded. Try again shortly.".to_owned(),
        ),
        _ => return response,
    };
    let mut response = Error::new(code, detail).into_response();
    response.headers_mut().insert(
        CACHE_CONTROL,
        super::cache::CachePolicy::NO_STORE.header_value(),
    );
    // The outer timeout can finish before the inner CORS layer returns a response.
    // Origin rejections remain untouched above and therefore never gain CORS headers.
    if public_read {
        response
            .headers_mut()
            .insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    }
    response
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.code.http_status())
            .expect("every error code maps to a valid status");
        (status, Json(self.body())).into_response()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_code_converts_to_its_numeric_status() {
        for code in ErrorCode::ALL {
            let response = Error::new(code, "detail").into_response();
            assert_eq!(response.status().as_u16(), code.http_status(), "{code:?}");
        }
    }
}
