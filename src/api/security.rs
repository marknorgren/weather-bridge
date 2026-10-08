//! Security headers (CSP and baseline headers) and the CloudFront origin check.
use super::cache::CachePolicy;
use crate::{Error, ErrorCode};
use axum::{
    http::{
        HeaderName, HeaderValue, Method,
        header::{CACHE_CONTROL, HOST},
    },
    response::IntoResponse,
};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tower_http::set_header::SetResponseHeaderLayer;

/// HTML pages: same-origin scripts only. Inline styles remain for the embedded stylesheets.
pub(super) const PAGE_CSP: &str = "default-src 'self'; script-src 'self'; connect-src 'self'; \
    img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self'; \
    object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";
/// Non-HTML responses (JSON, MCP, JavaScript) never render documents.
pub(super) const DEFAULT_CSP: &str =
    "default-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
/// Set `name` on every response that does not already carry it.
pub(super) fn default_header(
    name: HeaderName,
    value: &'static str,
) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(value))
}

/// Header CloudFront adds to every origin request in the public deployment.
pub const ORIGIN_VERIFY_HEADER: &str = "x-weather-bridge-origin-verify";
/// Shared value that proves a request came through CloudFront, parsed once by the
/// entry point. It is a deterrent against direct Function URL use, not a credential.
/// `Debug` never prints it, and nothing logs it.
#[derive(Clone)]
pub struct OriginVerify(Arc<[u8]>);
impl OriginVerify {
    pub const MIN_LEN: usize = 32;
    /// Accept at least 32 visible ASCII characters (valid as an HTTP header value).
    /// The error never echoes the value.
    pub fn new(value: &str) -> Result<Self, String> {
        if !value.bytes().all(|b| b.is_ascii_graphic()) {
            Err("must contain only visible ASCII characters".into())
        } else if value.len() < Self::MIN_LEN {
            Err(format!("must be at least {} characters", Self::MIN_LEN))
        } else {
            Ok(Self(value.as_bytes().into()))
        }
    }
    /// Constant-time comparison; only the length difference is observable.
    fn matches(&self, presented: &[u8]) -> bool {
        bool::from(self.0.as_ref().ct_eq(presented))
    }
}
impl std::fmt::Debug for OriginVerify {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OriginVerify(<redacted>)")
    }
}
/// Host values the Lambda Web Adapter readiness check sends to the app.
const READINESS_HOSTS: [&str; 3] = ["127.0.0.1:8080", "localhost:8080", "[::1]:8080"];
/// Reject requests that did not come through CloudFront when a verify value is
/// configured. Only the Lambda Web Adapter readiness check (`GET`/`HEAD /healthz`
/// with no query and a loopback Host) skips the check; public `/healthz` does not.
pub(super) async fn require_origin(
    expected: Option<OriginVerify>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let Some(expected) = expected else {
        return next.run(request).await;
    };
    let readiness = request.uri().path() == "/healthz"
        && request.uri().query().is_none()
        && matches!(*request.method(), Method::GET | Method::HEAD)
        && request
            .headers()
            .get(HOST)
            .and_then(|host| host.to_str().ok())
            .is_some_and(|host| READINESS_HOSTS.contains(&host));
    // Exactly one header with the expected value; a repeated header is rejected.
    let mut presented = request.headers().get_all(ORIGIN_VERIFY_HEADER).iter();
    let verified = presented
        .next()
        .is_some_and(|value| expected.matches(value.as_bytes()))
        && presented.next().is_none();
    if readiness || verified {
        return next.run(request).await;
    }
    let mut response = Error::new(
        ErrorCode::Forbidden,
        "This endpoint accepts requests only through the public Weather Bridge address.",
    )
    .into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, CachePolicy::NO_STORE.header_value());
    response
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{HttpConfig, app, test_support::*};
    use crate::weather::Weather;
    use axum::http::StatusCode;
    use http_body_util::BodyExt;
    use serde_json::{Value, json};

    #[tokio::test]
    async fn security_headers_on_pages_api_and_mcp() {
        let app = test_app();
        let get = |path: &str| {
            axum::http::Request::builder()
                .uri(path)
                .body(axum::body::Body::empty())
                .unwrap()
        };
        for (path, frame_ancestors, frame_option) in [
            ("/", "frame-ancestors 'none'", "DENY"),
            ("/developer", "frame-ancestors 'none'", "DENY"),
        ] {
            let response = send(&app, get(path)).await;
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_baseline(&response, path);
            let csp = header(&response, "content-security-policy");
            for directive in [
                "default-src 'self'",
                "script-src 'self';",
                "connect-src 'self'",
                "object-src 'none'",
                "base-uri 'none'",
                frame_ancestors,
            ] {
                assert!(csp.contains(directive), "{path}: {directive} in {csp}");
            }
            assert!(!csp.contains("script-src 'self' 'unsafe-inline'"), "{csp}");
            assert_eq!(header(&response, "x-frame-options"), frame_option, "{path}");
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let html = std::str::from_utf8(&body).unwrap();
            assert!(!html.contains("<script>"), "{path} has an inline script");
        }
        let response = send(&app, get("/v1/cities?q=Seattle")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_baseline(&response, "/v1/cities");
        assert_eq!(header(&response, "x-frame-options"), "DENY");
        assert!(header(&response, "content-security-policy").starts_with("default-src 'none'"));

        let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-11-25","capabilities":{},
            "clientInfo":{"name":"headers-test","version":"1"}}});
        let response = send(
            &app,
            axum::http::Request::post("/mcp")
                .header("host", "127.0.0.1:8790")
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("accept-encoding", "gzip")
                .header("mcp-protocol-version", "2025-11-25")
                .body(axum::body::Body::from(init.to_string()))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_baseline(&response, "/mcp");
    }

    const SECRET: &str = "0123456789abcdefghijklmnopqrstuvwxyzABCD";
    #[tokio::test]
    async fn origin_verify_rejects_requests_without_the_cloudfront_header() {
        let app = app(
            Arc::new(Weather::new().unwrap()),
            HttpConfig {
                origin_verify: Some(OriginVerify::new(SECRET).unwrap()),
                ..HttpConfig::default()
            },
        );
        let wrong = &SECRET.replace('0', "1");
        for header_value in [None, Some(wrong.as_str()), Some("short")] {
            for request in [
                get_with("/", header_value),
                get_with("/v1/cities?q=Seattle", header_value),
                get_with("/nope", header_value),
                mcp_init(header_value),
            ] {
                let path = request.uri().path().to_owned();
                let response = send(&app, request).await;
                assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
                assert_eq!(header(&response, "cache-control"), "no-store", "{path}");
                assert_baseline(&response, &path);
                let body = response.into_body().collect().await.unwrap().to_bytes();
                let text = std::str::from_utf8(&body).unwrap();
                let parsed: Value = serde_json::from_str(text).unwrap();
                assert_eq!(parsed["errors"][0]["status"], "403", "{path}");
                assert!(
                    !text.to_ascii_lowercase().contains("origin-verify"),
                    "{text}"
                );
                assert!(!text.contains(SECRET), "{text}");
            }
        }
        for request in [
            get_with("/", Some(SECRET)),
            get_with("/v1/cities?q=Seattle", Some(SECRET)),
            mcp_init(Some(SECRET)),
        ] {
            let path = request.uri().path().to_owned();
            assert_eq!(send(&app, request).await.status(), StatusCode::OK, "{path}");
        }
        let health = |method: &str, path: &str, host: Option<&str>| {
            let mut builder = axum::http::Request::builder().method(method).uri(path);
            if let Some(host) = host {
                builder = builder.header("host", host);
            }
            builder.body(axum::body::Body::empty()).unwrap()
        };
        for host in ["127.0.0.1:8080", "localhost:8080", "[::1]:8080"] {
            for method in ["GET", "HEAD"] {
                let response = send(&app, health(method, "/healthz", Some(host))).await;
                assert_eq!(response.status(), StatusCode::OK, "{method} {host}");
            }
            for (method, path) in [
                ("POST", "/healthz"),
                ("GET", "/healthz?x=1"),
                ("GET", "/v1/cities?q=Seattle"),
            ] {
                let response = send(&app, health(method, path, Some(host))).await;
                assert_eq!(
                    response.status(),
                    StatusCode::FORBIDDEN,
                    "{method} {path} {host}"
                );
            }
        }
        for host in [None, Some("example.com"), Some("localhost:9000")] {
            let response = send(&app, health("GET", "/healthz", host)).await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{host:?}");
        }
        let repeated = axum::http::Request::builder()
            .uri("/v1/cities?q=Seattle")
            .header(ORIGIN_VERIFY_HEADER, SECRET)
            .header(ORIGIN_VERIFY_HEADER, SECRET)
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(send(&app, repeated).await.status(), StatusCode::FORBIDDEN);
    }
    #[tokio::test]
    async fn without_origin_verify_local_requests_are_unchanged() {
        let app = test_app();
        for request in [
            get_with("/", None),
            get_with("/v1/cities?q=Seattle", None),
            mcp_init(None),
        ] {
            let path = request.uri().path().to_owned();
            assert_eq!(send(&app, request).await.status(), StatusCode::OK, "{path}");
        }
    }
    #[test]
    fn origin_verify_rejects_short_or_invisible_values_without_echoing_them() {
        assert!(OriginVerify::new(SECRET).is_ok());
        for bad in ["short", &SECRET[..31], &format!("{SECRET} with space")] {
            let error = OriginVerify::new(bad).unwrap_err();
            assert!(!error.contains(bad), "{error}");
        }
        assert_eq!(
            format!("{:?}", OriginVerify::new(SECRET).unwrap()),
            "OriginVerify(<redacted>)"
        );
    }
}
