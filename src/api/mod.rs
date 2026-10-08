//! HTTP layer: the axum app that serves pages, the REST API and MCP over the shared
//! `Weather` service. Each submodule states its own responsibility.
mod cache;
mod config;
mod cors;
mod error;
mod mcp_http;
mod openapi;
mod routes;
pub use openapi::{openapi, openapi_json};
mod security;

pub use config::{DEFAULT_DOCS_URL, DocsUrl, HttpConfig, McpAccess};
pub use security::{ORIGIN_VERIFY_HEADER, OriginVerify};

use crate::{
    diagnostics::{HttpRoute, Metrics, StatusClass},
    weather::Weather,
};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{FromRequest, MatchedPath, Request, State},
    http::{
        StatusCode,
        header::{
            CACHE_CONTROL, CONTENT_SECURITY_POLICY, REFERRER_POLICY, STRICT_TRANSPORT_SECURITY,
            X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
        },
    },
    response::{IntoResponse, Response},
};
use cache::CachePolicy;
use security::{DEFAULT_CSP, default_header};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower_http::{
    compression::CompressionLayer,
    limit::RequestBodyLimitLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, RequestId, SetRequestIdLayer},
    set_header::SetResponseHeader,
    timeout::TimeoutLayer,
};
use tracing::Instrument;

/// Largest REST or MCP request body the app reads. Larger bodies get 413
/// `REQUEST_TOO_LARGE`; the error detail and OpenAPI text derive from this value.
const MAX_REQUEST_BODY_BYTES: usize = 16 * 1024;
const _: () = assert!(
    MAX_REQUEST_BODY_BYTES.is_multiple_of(1024),
    "limit text is in whole KiB"
);

/// The request body limit as user-facing text, e.g. "16 KiB".
fn max_request_body_text() -> String {
    format!("{} KiB", MAX_REQUEST_BODY_BYTES / 1024)
}

/// Outer deadline for a whole HTTP request. It must outlast the weather service's own
/// lookup deadline so a slow lookup ends as that typed error, not as this timeout.
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(50);
const _: () = assert!(
    HTTP_REQUEST_TIMEOUT.as_secs() > crate::weather::LOOKUP_DEADLINE.as_secs(),
    "the HTTP timeout must exceed the weather lookup deadline"
);

/// Build the app.
///
/// Middleware order is security-relevant. Each `.layer` wraps everything added before
/// it, so a request passes through the layers bottom to top and a response top to
/// bottom: request ID, trace, baseline headers, compression, REST error mapping, timeout,
/// origin check, CORS, body limit, REST body consumption, then the route. Keeping the
/// origin check outside CORS means a rejected request gets no CORS headers, and keeping
/// the baseline headers outermost means every response, including 403s, 404s, 413s and
/// timeouts, carries them.
pub fn app(weather: Arc<Weather>, config: HttpConfig) -> Router {
    let HttpConfig {
        mcp_access,
        origin_verify,
        docs_url,
    } = config;
    let mcp = mcp_http::service(weather.clone(), mcp_access);
    let router = routes::router(&docs_url)
        // MCP is a per-request protocol exchange; never cache it.
        .nest_service(
            "/mcp",
            SetResponseHeader::overriding(mcp, CACHE_CONTROL, CachePolicy::NO_STORE.header_value()),
        );
    layers(router, origin_verify, weather.metrics()).with_state(weather)
}

fn layers(
    router: Router<Arc<Weather>>,
    origin_verify: Option<OriginVerify>,
    metrics: Arc<Metrics>,
) -> Router<Arc<Weather>> {
    router
        .layer(axum::middleware::from_fn(consume_rest_body))
        .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BODY_BYTES))
        .layer(axum::middleware::from_fn(cors::public_cors))
        // Runs before routing, so unknown paths and MCP are covered too.
        .layer(axum::middleware::from_fn(move |request, next| {
            security::require_origin(origin_verify.clone(), request, next)
        }))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            HTTP_REQUEST_TIMEOUT,
        ))
        .layer(axum::middleware::from_fn(error::rest_transport_errors))
        // Compress text responses (HTML, JS, JSON, MCP JSON) when the client accepts it.
        // SSE and responses under 32 bytes are left alone by the default predicate.
        .layer(CompressionLayer::new().gzip(true).br(true))
        // Baseline headers for every response, including JSON, MCP and middleware errors.
        // Routes that set their own value (HTML pages) keep it.
        // /healthz, 403s, unknown paths, 413s and timeouts: never cached.
        .layer(
            tower_http::set_header::SetResponseHeaderLayer::if_not_present(
                CACHE_CONTROL,
                CachePolicy::NO_STORE.header_value(),
            ),
        )
        .layer(default_header(CONTENT_SECURITY_POLICY, DEFAULT_CSP))
        .layer(default_header(X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(default_header(X_FRAME_OPTIONS, "DENY"))
        .layer(default_header(REFERRER_POLICY, "no-referrer"))
        .layer(default_header(
            STRICT_TRANSPORT_SECURITY,
            "max-age=31536000",
        ))
        // Correlation and tracing wrap the whole stack, including origin, limit and timeout errors.
        .layer(axum::middleware::from_fn_with_state(metrics, observe_http))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(axum::middleware::from_fn(sanitize_request_id))
}

/// Accept caller correlation IDs only when they are short, printable tokens. Invalid or
/// oversized values are replaced by the request-ID layer before anything records them.
async fn sanitize_request_id(mut request: Request, next: axum::middleware::Next) -> Response {
    const NAME: &str = "x-request-id";
    let valid = request.headers().get(NAME).is_some_and(|value| {
        let bytes = value.as_bytes();
        !bytes.is_empty()
            && bytes.len() <= 128
            && bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.'))
    });
    if !valid {
        request.headers_mut().remove(NAME);
    }
    next.run(request).await
}

async fn observe_http(
    State(metrics): State<Arc<Metrics>>,
    request: Request,
    next: axum::middleware::Next,
) -> Response {
    let method = request.method().clone();
    let route = HttpRoute::from_matched(
        request
            .extensions()
            .get::<MatchedPath>()
            .map(MatchedPath::as_str),
    );
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .and_then(|id| id.header_value().to_str().ok())
        .unwrap_or("unknown")
        .to_owned();
    let span = tracing::info_span!(
        "http_request",
        %method,
        route = route.matched_path(),
        %request_id
    );
    async move {
        let started = Instant::now();
        let response = next.run(request).await;
        let latency = started.elapsed();
        metrics.http_response(
            route,
            StatusClass::from_status(response.status().as_u16()),
            latency,
        );
        tracing::info!(
            status = response.status().as_u16(),
            latency_ms = latency.as_millis(),
            "HTTP response"
        );
        response
    }
    .instrument(span)
    .await
}

/// REST inputs are query parameters. Consume the unused body through the outer
/// [`MAX_REQUEST_BODY_BYTES`] limiter so streamed/chunked bodies obey the same cap as Content-Length requests.
/// The request timeout bounds this read. MCP continues to consume its own body.
async fn consume_rest_body(request: Request, next: axum::middleware::Next) -> Response {
    if !request.uri().path().starts_with("/v1/") {
        return next.run(request).await;
    }
    let (parts, body) = request.into_parts();
    match Bytes::from_request(Request::new(body), &()).await {
        Ok(_) => next.run(Request::from_parts(parts, Body::empty())).await,
        Err(rejection) => rejection.into_response(),
    }
}

/// Request builders and header assertions shared by the module tests.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use axum::response::Response;
    use serde_json::json;
    use tower::ServiceExt;

    pub fn test_app() -> Router {
        app(Arc::new(Weather::new().unwrap()), HttpConfig::default())
    }
    pub async fn send(app: &Router, request: axum::http::Request<axum::body::Body>) -> Response {
        app.clone().oneshot(request).await.unwrap()
    }
    pub fn header<'a>(response: &'a Response, name: &str) -> &'a str {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_else(|| panic!("missing {name}"))
    }
    pub fn assert_baseline(response: &Response, path: &str) {
        assert_eq!(
            header(response, "x-content-type-options"),
            "nosniff",
            "{path}"
        );
        assert_eq!(header(response, "referrer-policy"), "no-referrer", "{path}");
        assert_eq!(
            header(response, "strict-transport-security"),
            "max-age=31536000",
            "{path}"
        );
        let csp = header(response, "content-security-policy");
        assert!(!csp.contains("unsafe-eval"), "{path}: {csp}");
        assert!(!csp.contains("script-src *"), "{path}: {csp}");
    }
    pub fn mcp_init(origin_header: Option<&str>) -> axum::http::Request<axum::body::Body> {
        let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-11-25","capabilities":{},
            "clientInfo":{"name":"origin-test","version":"1"}}});
        let mut builder = axum::http::Request::post("/mcp")
            .header("host", "127.0.0.1:8790")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-11-25");
        if let Some(value) = origin_header {
            builder = builder.header(ORIGIN_VERIFY_HEADER, value);
        }
        builder
            .body(axum::body::Body::from(init.to_string()))
            .unwrap()
    }
    pub fn get_with(
        path: &str,
        origin_header: Option<&str>,
    ) -> axum::http::Request<axum::body::Body> {
        let mut builder = axum::http::Request::builder().uri(path);
        if let Some(value) = origin_header {
            builder = builder.header(ORIGIN_VERIFY_HEADER, value);
        }
        builder.body(axum::body::Body::empty()).unwrap()
    }
}
#[cfg(test)]
mod tests {
    use super::MAX_REQUEST_BODY_BYTES;
    use super::test_support::*;
    use axum::http::StatusCode;
    use http_body_util::BodyExt;
    use serde_json::Value;

    /// Assert the shared transport-error envelope and return its detail text.
    async fn assert_transport_error(
        response: axum::response::Response,
        status: u16,
        code: &str,
    ) -> String {
        assert_eq!(response.status().as_u16(), status);
        assert_baseline(&response, "/v1/cities");
        assert!(response.headers().contains_key("x-request-id"));
        assert_eq!(header(&response, "cache-control"), "no-store");
        assert_eq!(header(&response, "access-control-allow-origin"), "*");
        assert_eq!(header(&response, "content-type"), "application/json");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["errors"][0]["status"], status.to_string());
        assert_eq!(body["errors"][0]["code"], code);
        assert!(body["errors"][0]["choices"].is_null());
        let detail = body["errors"][0]["detail"].as_str().unwrap_or_default();
        assert!(!detail.is_empty());
        detail.to_owned()
    }

    #[tokio::test]
    async fn oversized_rest_requests_return_the_error_envelope() {
        let size = MAX_REQUEST_BODY_BYTES + 1;
        let request = axum::http::Request::get("/v1/cities?q=Seattle")
            .header("content-length", size.to_string())
            .body(axum::body::Body::from(vec![b'x'; size]))
            .unwrap();
        let detail =
            assert_transport_error(send(&test_app(), request).await, 413, "REQUEST_TOO_LARGE")
                .await;
        // The documented limit, stated in the unit callers read in docs and OpenAPI.
        assert_eq!(MAX_REQUEST_BODY_BYTES, 16384);
        assert_eq!(detail, "Request body must not exceed 16 KiB.");
    }

    #[tokio::test]
    async fn rest_body_limit_also_applies_without_content_length() {
        let app = test_app();
        for size in [0, 7, MAX_REQUEST_BODY_BYTES, MAX_REQUEST_BODY_BYTES + 1] {
            let request = axum::http::Request::get("/v1/cities?q=Seattle")
                .body(axum::body::Body::from(vec![b'x'; size]))
                .unwrap();
            let response = send(&app, request).await;
            if size > MAX_REQUEST_BODY_BYTES {
                assert_transport_error(response, 413, "REQUEST_TOO_LARGE").await;
            } else {
                assert_eq!(response.status(), StatusCode::OK, "size {size}");
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn rest_body_reads_are_bounded_and_fail_as_typed_errors() {
        let app = test_app();
        for (fail, status, code) in [
            (false, 504, "UPSTREAM_TIMEOUT"),
            (true, 400, "INVALID_LOCATION"),
        ] {
            let body = axum::body::Body::empty().with_trailers(async move {
                if fail {
                    Some(Err(axum::Error::new(std::io::Error::from(
                        std::io::ErrorKind::UnexpectedEof,
                    ))))
                } else {
                    std::future::pending().await
                }
            });
            let request = axum::http::Request::get("/v1/cities?q=Seattle")
                .body(axum::body::Body::new(body))
                .unwrap();
            assert_transport_error(send(&app, request).await, status, code).await;
        }
    }

    #[tokio::test]
    async fn chunked_rest_bodies_cannot_bypass_the_limit() {
        use std::io::{Read, Write};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, test_app()).await.unwrap() });
        let response = tokio::task::spawn_blocking(move || {
            let mut stream = std::net::TcpStream::connect(address).unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut request = b"GET /v1/cities?q=Seattle HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
            for size in [8192, 8192, 1] {
                write!(request, "{size:x}\r\n").unwrap();
                request.extend(vec![b'x'; size]);
                request.extend(b"\r\n");
            }
            request.extend(b"0\r\n\r\n");
            stream.write_all(&request).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        }).await.unwrap();
        server.abort();
        assert!(response.starts_with("HTTP/1.1 413"), "{response}");
        let (_, body) = response.split_once("\r\n\r\n").unwrap();
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["errors"][0]["code"], "REQUEST_TOO_LARGE");
    }

    #[tokio::test(start_paused = true)]
    async fn mapped_errors_keep_get_content_length_on_head() {
        let router = axum::Router::new().route(
            "/v1/slow",
            axum::routing::get(std::future::pending::<&'static str>),
        );
        let slow = super::layers(
            router,
            None,
            std::sync::Arc::new(crate::diagnostics::Metrics::default()),
        )
        .with_state(std::sync::Arc::new(crate::weather::Weather::new().unwrap()));
        for (app, path, size, status) in [
            (test_app(), "/v1/cities?q=Seattle", 16385, 413),
            (slow, "/v1/slow", 0, 504),
        ] {
            let request = |method| {
                axum::http::Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-length", size)
                    .body(axum::body::Body::from(vec![b'x'; size]))
                    .unwrap()
            };
            let get = send(&app, request("GET")).await;
            let head = send(&app, request("HEAD")).await;
            assert_eq!(get.status().as_u16(), status);
            assert_eq!(head.status().as_u16(), status);
            assert_eq!(
                header(&get, "content-length"),
                header(&head, "content-length")
            );
            assert!(
                head.into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .is_empty()
            );
        }
    }

    #[tokio::test]
    async fn transport_errors_preserve_origin_precedence_mcp_and_head() {
        use super::{HttpConfig, OriginVerify, app};
        use std::sync::Arc;
        let app = app(
            Arc::new(crate::weather::Weather::new().unwrap()),
            HttpConfig {
                origin_verify: Some(
                    OriginVerify::new("0123456789abcdefghijklmnopqrstuvwxyzABCD").unwrap(),
                ),
                ..Default::default()
            },
        );
        let oversized = |method: &str, path: &str| {
            axum::http::Request::builder()
                .method(method)
                .uri(path)
                .header("content-length", "16385")
                .body(axum::body::Body::from(vec![b'x'; 16385]))
                .unwrap()
        };
        let response = send(&app, oversized("GET", "/v1/cities?q=Seattle")).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(
            response
                .headers()
                .get("access-control-allow-origin")
                .is_none()
        );
        let app = test_app();
        let response = send(&app, oversized("POST", "/mcp")).await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(header(&response, "cache-control"), "no-store");
        assert_eq!(
            header(&response, "content-type"),
            "text/plain; charset=utf-8"
        );
        assert!(
            response
                .headers()
                .get("access-control-allow-origin")
                .is_none()
        );
        let response = send(&app, oversized("HEAD", "/v1/cities?q=Seattle")).await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(header(&response, "content-type"), "application/json");
        assert_eq!(header(&response, "access-control-allow-origin"), "*");
        assert!(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .is_empty()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn outer_rest_timeouts_return_the_error_envelope() {
        let router = axum::Router::new().route(
            "/v1/cities",
            axum::routing::get(std::future::pending::<&'static str>),
        );
        let app = super::layers(
            router,
            None,
            std::sync::Arc::new(crate::diagnostics::Metrics::default()),
        )
        .with_state(std::sync::Arc::new(crate::weather::Weather::new().unwrap()));
        assert_transport_error(
            send(&app, get_with("/v1/cities?q=Seattle", None)).await,
            504,
            "UPSTREAM_TIMEOUT",
        )
        .await;
    }

    #[tokio::test]
    async fn transport_mapping_preserves_typed_service_errors() {
        let router = axum::Router::new().route(
            "/v1/weather",
            axum::routing::get(|| async {
                crate::Error::new(
                    crate::ErrorCode::UpstreamTimeout,
                    "Service deadline detail.",
                )
            }),
        );
        let app = super::layers(
            router,
            None,
            std::sync::Arc::new(crate::diagnostics::Metrics::default()),
        )
        .with_state(std::sync::Arc::new(crate::weather::Weather::new().unwrap()));
        let response = send(&app, get_with("/v1/weather", None)).await;
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["errors"][0]["detail"], "Service deadline detail.");
    }

    #[tokio::test]
    async fn text_responses_are_compressed_when_accepted() {
        let app = test_app();
        let response = send(&app, get_with("/openapi.json", None)).await;
        assert!(response.headers().get("content-encoding").is_none());
        let original = response.into_body().collect().await.unwrap().to_bytes();
        assert!(serde_json::from_slice::<Value>(&original).is_ok());
        for encoding in ["gzip", "br"] {
            let response = send(
                &app,
                axum::http::Request::builder()
                    .uri("/openapi.json")
                    .header("accept-encoding", encoding)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(header(&response, "content-encoding"), encoding);
            assert!(header(&response, "content-type").starts_with("application/json"));
            assert_baseline(&response, "/openapi.json");
            let body = response.into_body().collect().await.unwrap().to_bytes();
            assert!(
                body.len() < original.len() / 3,
                "{encoding}: {} of {} bytes",
                body.len(),
                original.len()
            );
        }
    }
}
