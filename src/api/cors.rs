//! CORS for the public, read-only REST API.
use axum::{
    http::{
        HeaderValue, Method, StatusCode,
        header::{
            ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
            ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_MAX_AGE,
        },
    },
    response::{IntoResponse, Response},
};

/// CORS for the static documentation (GitHub Pages) and any other browser client.
///
/// A wildcard origin is safe here: `/v1/*` and `/openapi.json` are public, read-only
/// and cookie-less, so no credentials are allowed and a cross-origin page can read
/// nothing it could not fetch directly. Only GET/HEAD responses get the header, and
/// preflight allows only GET/HEAD with `Accept`. `/mcp` keeps its own Origin
/// allow-list and gets no CORS headers; pages and `/healthz` get none either.
pub(super) async fn public_cors(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let path = request.uri().path();
    let api = path.starts_with("/v1/");
    if api && request.method() == Method::OPTIONS {
        return (
            StatusCode::NO_CONTENT,
            [
                (ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                (ACCESS_CONTROL_ALLOW_METHODS, "GET, HEAD"),
                (ACCESS_CONTROL_ALLOW_HEADERS, "Accept"),
                (ACCESS_CONTROL_MAX_AGE, "86400"),
            ],
        )
            .into_response();
    }
    let public =
        (api || path == "/openapi.json") && matches!(*request.method(), Method::GET | Method::HEAD);
    let mut response = next.run(request).await;
    if public {
        response
            .headers_mut()
            .insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    }
    response
}
#[cfg(test)]
mod tests {
    use crate::api::test_support::*;
    use axum::http::{HeaderValue, StatusCode};

    fn request(method: &str, path: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("origin", "https://marknorgren.github.io")
            .body(axum::body::Body::empty())
            .unwrap()
    }
    #[tokio::test]
    async fn public_read_only_api_allows_any_origin_but_pages_and_mcp_do_not() {
        let app = test_app();
        for (method, path) in [
            ("GET", "/v1/cities?q=Seattle"),
            ("HEAD", "/v1/cities?q=Seattle"),
            ("GET", "/v1/weather?city=Springfield"),
            ("GET", "/openapi.json"),
        ] {
            let response = send(&app, request(method, path)).await;
            assert_eq!(
                header(&response, "access-control-allow-origin"),
                "*",
                "{method} {path}"
            );
            assert!(
                response
                    .headers()
                    .get("access-control-allow-credentials")
                    .is_none()
            );
        }
        for (method, path) in [
            ("GET", "/"),
            ("GET", "/developer"),
            ("GET", "/healthz"),
            ("POST", "/v1/cities?q=Seattle"),
        ] {
            let response = send(&app, request(method, path)).await;
            assert!(
                response
                    .headers()
                    .get("access-control-allow-origin")
                    .is_none(),
                "{method} {path}"
            );
        }
        let mut mcp = mcp_init(None);
        mcp.headers_mut()
            .insert("origin", HeaderValue::from_static("http://127.0.0.1:8790"));
        let response = send(&app, mcp).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response
                .headers()
                .get("access-control-allow-origin")
                .is_none()
        );
        // A foreign origin is still refused by the MCP allow-list.
        let mut mcp = mcp_init(None);
        mcp.headers_mut().insert(
            "origin",
            HeaderValue::from_static("https://marknorgren.github.io"),
        );
        let response = send(&app, mcp).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(
            response
                .headers()
                .get("access-control-allow-origin")
                .is_none()
        );
    }
    #[tokio::test]
    async fn api_preflight_allows_only_read_methods() {
        let app = test_app();
        let mut preflight = request("OPTIONS", "/v1/weather?city=Seattle");
        preflight.headers_mut().insert(
            "access-control-request-method",
            HeaderValue::from_static("GET"),
        );
        let response = send(&app, preflight).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(header(&response, "access-control-allow-origin"), "*");
        assert_eq!(
            header(&response, "access-control-allow-methods"),
            "GET, HEAD"
        );
        assert_eq!(header(&response, "access-control-allow-headers"), "Accept");
        assert_eq!(header(&response, "access-control-max-age"), "86400");
        assert!(
            response
                .headers()
                .get("access-control-allow-credentials")
                .is_none()
        );
        let response = send(&app, request("OPTIONS", "/mcp")).await;
        assert!(
            response
                .headers()
                .get("access-control-allow-origin")
                .is_none()
        );
    }
}
