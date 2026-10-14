// Ported from postgrest src/library/PostgREST/Cors.hs (MIT), pin v16.4.

//! PostgREST's CORS middleware as it is reached behind the gateway.
//!
//! `Cors.hs` builds a `wai-cors` policy only when the request has an `Origin`
//! header. With `PGRST_SERVER_CORS_ALLOWED_ORIGINS` unset (the self-hosted
//! default) every origin is allowed, so a request with `Origin` gets
//! `Access-Control-Allow-Origin: *` and the fixed `Access-Control-Expose-Headers`
//! list; a request without `Origin` is untouched. `corsVaryOrigin` is off, so
//! no `Vary` is added.
//!
//! Preflights never reach PostgREST in the pinned stack: Kong's `cors` plugin
//! answers them first (`megabase-server`), so the `wai-cors` preflight branch
//! (`Access-Control-Allow-Methods`, `Access-Control-Max-Age: 86400`) is not
//! served here. Specified in `specs/rest/cors.md`.

use axum::{
    extract::Request,
    http::{header, HeaderValue},
    middleware::Next,
    response::Response,
};

/// `corsExposedHeaders` from `Cors.hs`, joined the way `wai-cors` renders them.
pub(crate) const EXPOSED_HEADERS: &str = "Content-Encoding, Content-Location, Content-Range, \
     Content-Type, Date, Location, Server, Transfer-Encoding, Range-Unit";

/// Add PostgREST's CORS response headers when the request names an `Origin`.
pub(crate) async fn postgrest_cors(request: Request, next: Next) -> Response {
    let has_origin = request.headers().contains_key(header::ORIGIN);
    let mut response = next.run(request).await;
    if has_origin {
        let headers = response.headers_mut();
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        );
        headers.insert(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            HeaderValue::from_static(EXPOSED_HEADERS),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, routing::get, Router};
    use tower::ServiceExt;

    fn app() -> Router {
        Router::new()
            .route("/items", get(|| async { "[]" }))
            .layer(axum::middleware::from_fn(postgrest_cors))
    }

    #[tokio::test]
    async fn origin_gets_allow_origin_and_exposed_headers() {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/items")
                    .header(header::ORIGIN, "http://example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_EXPOSE_HEADERS],
            "Content-Encoding, Content-Location, Content-Range, Content-Type, Date, \
             Location, Server, Transfer-Encoding, Range-Unit"
        );
        assert!(response.headers().get(header::VARY).is_none());
    }

    #[tokio::test]
    async fn no_origin_adds_nothing() {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/items")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());
        assert!(response
            .headers()
            .get(header::ACCESS_CONTROL_EXPOSE_HEADERS)
            .is_none());
    }
}
