//! CORS through the gateway: Kong's `cors` plugin answers preflights and adds
//! `Access-Control-Allow-Origin: *` on every route that lists the plugin;
//! PostgREST adds `Access-Control-Expose-Headers` when `Origin` is present.
//! `specs/rest/cors.md`.

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use megabase_server::{create_router, CORS_ALLOW_METHODS, CORS_ROUTES, GATEWAY_ROUTES};
use tower::ServiceExt;

const ORIGIN: &str = "http://localhost:3000";
const REQUESTED_HEADERS: &str =
    "apikey,authorization,content-type,prefer,x-client-info,accept-profile,content-profile";

fn preflight(path: &str, requested_method: &str, requested_headers: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("OPTIONS")
        .uri(path)
        .header(header::ORIGIN, ORIGIN)
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, requested_method);
    if let Some(requested) = requested_headers {
        builder = builder.header(header::ACCESS_CONTROL_REQUEST_HEADERS, requested);
    }
    builder.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn rest_preflight_allows_methods_and_echoes_headers() {
    let response = create_router()
        .oneshot(preflight(
            "/rest/v1/todos?select=*",
            "PATCH",
            Some(REQUESTED_HEADERS),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert_eq!(
        headers[header::ACCESS_CONTROL_ALLOW_METHODS],
        CORS_ALLOW_METHODS
    );
    assert_eq!(
        headers[header::ACCESS_CONTROL_ALLOW_HEADERS],
        REQUESTED_HEADERS
    );
    // Kong's plugin defaults leave credentials and max_age off, and the
    // preflight is answered before PostgREST, so there is no `Allow`.
    assert!(headers
        .get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
        .is_none());
    assert!(headers.get(header::ACCESS_CONTROL_MAX_AGE).is_none());
    assert!(headers.get(header::ALLOW).is_none());
    assert!(headers.get(header::ACCESS_CONTROL_EXPOSE_HEADERS).is_none());
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(bytes.is_empty());
}

#[tokio::test]
async fn preflight_is_answered_before_authentication_on_every_cors_route() {
    for prefix in CORS_ROUTES {
        for suffix in ["", "todos", "rpc/add"] {
            let path = format!("{prefix}{suffix}");
            let response = create_router()
                .oneshot(preflight(&path, "DELETE", None))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(
                response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
                "*",
                "{path}"
            );
            assert_eq!(
                response.headers()[header::ACCESS_CONTROL_ALLOW_METHODS],
                CORS_ALLOW_METHODS,
                "{path}"
            );
            assert!(
                response
                    .headers()
                    .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
                    .is_none(),
                "{path}: no Access-Control-Request-Headers to echo"
            );
        }
    }
}

#[tokio::test]
async fn options_without_requested_method_is_not_a_preflight() {
    // Kong forwards it upstream; the route is still 501 here.
    let request = Request::builder()
        .method("OPTIONS")
        .uri("/auth/v1/token")
        .header(header::ORIGIN, ORIGIN)
        .body(Body::empty())
        .unwrap();
    let response = create_router().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert!(response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_METHODS)
        .is_none());
}

#[tokio::test]
async fn rest_simple_request_with_origin_exposes_postgrest_headers() {
    let request = Request::builder()
        .uri("/rest/v1/todos")
        .header(header::ORIGIN, ORIGIN)
        .body(Body::empty())
        .unwrap();
    let response = create_router().oneshot(request).await.unwrap();
    // No pool in this router, so PostgREST's PGRST000; the CORS headers still apply.
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let allow_origin: Vec<_> = response
        .headers()
        .get_all(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .iter()
        .collect();
    assert_eq!(allow_origin, ["*"], "Kong replaces PostgREST's value");
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_EXPOSE_HEADERS],
        "Content-Encoding, Content-Location, Content-Range, Content-Type, Date, \
         Location, Server, Transfer-Encoding, Range-Unit"
    );
}

#[tokio::test]
async fn every_cors_route_response_allows_any_origin_without_origin_header() {
    for prefix in CORS_ROUTES {
        let response = create_router()
            .oneshot(Request::builder().uri(*prefix).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            "*",
            "{prefix}"
        );
        assert!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_EXPOSE_HEADERS)
                .is_none(),
            "{prefix}: PostgREST exposes headers only for a request with Origin"
        );
    }
}

#[tokio::test]
async fn routes_without_the_plugin_get_no_cors_headers() {
    for path in ["/pg/x", "/", "/_megabase/health", "/rest/v1", "/auth/v1"] {
        let response = create_router()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header(header::ORIGIN, ORIGIN)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none(),
            "{path}"
        );
    }
    // A preflight on `/pg/` reaches Postgres Meta instead of Kong's plugin.
    let response = create_router()
        .oneshot(preflight("/pg/tables", "GET", Some("apikey")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    assert!(response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_METHODS)
        .is_none());
}

#[test]
fn cors_routes_are_gateway_routes() {
    for prefix in CORS_ROUTES {
        assert!(
            GATEWAY_ROUTES.iter().any(|(route, _)| route == prefix),
            "{prefix}"
        );
    }
}
