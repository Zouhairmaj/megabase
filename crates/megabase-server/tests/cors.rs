//! Kong's `cors` plugin at the gateway (`specs/server/cors.md`, issue 317).
//!
//! The pinned `kong.yml` attaches the plugin with its defaults to every
//! service except Postgres Meta, the blocked Realtime admin routes, and MCP.
//! A browser preflight on `/auth/v1/*` therefore gets Kong's `200`, not the
//! upstream service and not a 501.

use axum::{
    body::{to_bytes, Body},
    http::{header, HeaderMap, Request, StatusCode},
};
use megabase_server::{create_router, CORS_ALLOW_METHODS, HEALTH_PATH, ROUTES_WITHOUT_CORS};
use tower::ServiceExt;

async fn send(
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, HeaderMap, Vec<u8>) {
    // A client key, so Kong's `key-auth` (which runs after `cors`) lets the
    // request through; it is not a service-role key.
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("apikey", "anon-key");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let response = create_router()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, headers, bytes.to_vec())
}

const BROWSER_PREFLIGHT: &[(&str, &str)] = &[
    ("origin", "http://localhost:3000"),
    ("access-control-request-method", "POST"),
    (
        "access-control-request-headers",
        "apikey,authorization,content-type,x-client-info,x-supabase-api-version",
    ),
];

#[tokio::test]
async fn auth_preflight_is_kong_200_with_default_plugin_headers() {
    for path in [
        "/auth/v1/signup",
        "/auth/v1/token?grant_type=password",
        "/auth/v1/user",
        "/auth/v1/logout",
        "/auth/v1/verify",
        "/auth/v1/some/unported/path",
        "/.well-known/oauth-authorization-server",
    ] {
        let (status, headers, body) = send("OPTIONS", path, BROWSER_PREFLIGHT).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(
            body.is_empty(),
            "{path}: {}",
            String::from_utf8_lossy(&body)
        );
        assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*", "{path}");
        assert_eq!(
            headers[header::ACCESS_CONTROL_ALLOW_HEADERS],
            "apikey,authorization,content-type,x-client-info,x-supabase-api-version",
            "{path}"
        );
        assert_eq!(
            headers[header::ACCESS_CONTROL_ALLOW_METHODS],
            CORS_ALLOW_METHODS,
            "{path}"
        );
        assert_eq!(
            headers[header::ACCESS_CONTROL_ALLOW_METHODS],
            "GET,HEAD,PUT,PATCH,POST,DELETE,OPTIONS,TRACE,CONNECT",
            "{path}"
        );
        // Defaults: `credentials: false`, no `max_age`, no `Vary`, no upstream.
        for absent in [
            header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
            header::ACCESS_CONTROL_MAX_AGE,
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            header::VARY,
            header::ALLOW,
            header::CONTENT_TYPE,
        ] {
            assert!(headers.get(&absent).is_none(), "{path}: {absent}");
        }
        assert!(headers.get("x-request-id").is_some(), "{path}");
    }
}

#[tokio::test]
async fn preflight_without_requested_headers_sets_no_allow_headers() {
    let (status, headers, _) = send(
        "OPTIONS",
        "/auth/v1/signup",
        &[
            ("origin", "http://localhost:3000"),
            ("access-control-request-method", "POST"),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.get(header::ACCESS_CONTROL_ALLOW_HEADERS).is_none());
    assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert_eq!(
        headers[header::ACCESS_CONTROL_ALLOW_METHODS],
        CORS_ALLOW_METHODS
    );
}

#[tokio::test]
async fn preflight_needs_no_apikey_on_any_cors_service() {
    // Kong runs `cors` (priority 2000) before `key-auth` and `basic-auth`,
    // so the Studio catch-all and REST answer the preflight too.
    for path in [
        "/rest/v1/",
        "/rest/v1/todos",
        "/rest/v1/rpc/add",
        "/graphql/v1",
        "/realtime/v1/api/broadcast",
        "/storage/v1/object/bucket/key",
        "/functions/v1/hello",
        "/",
        "/auth/v1",
        "/api/platform/profile",
        HEALTH_PATH,
    ] {
        let (status, headers, body) = send("OPTIONS", path, BROWSER_PREFLIGHT).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(body.is_empty(), "{path}");
        assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*", "{path}");
        assert_eq!(
            headers[header::ACCESS_CONTROL_ALLOW_METHODS],
            CORS_ALLOW_METHODS,
            "{path}"
        );
    }
}

#[tokio::test]
async fn options_without_a_full_preflight_reaches_the_component() {
    // Kong only intercepts when both `Origin` and
    // `Access-Control-Request-Method` are present.
    for headers in [
        &[][..],
        &[("origin", "http://localhost:3000")][..],
        &[("access-control-request-method", "POST")][..],
    ] {
        let (status, response_headers, body) = send("OPTIONS", "/auth/v1/signup", headers).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{headers:?}");
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["code"], "MEGABASE_NOT_IMPLEMENTED", "{headers:?}");
        // The header filter still stamps the upstream answer.
        assert_eq!(
            response_headers[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            "*",
            "{headers:?}"
        );
        assert!(
            response_headers
                .get(header::ACCESS_CONTROL_ALLOW_METHODS)
                .is_none(),
            "{headers:?}"
        );
    }
}

#[tokio::test]
async fn every_non_preflight_response_carries_allow_origin_star() {
    // Kong sets the header with or without `Origin` on the request, on
    // served routes and on 501s alike.
    for (method, path, expected) in [
        ("GET", "/auth/v1/health", StatusCode::OK),
        ("GET", "/auth/v1/settings", StatusCode::OK),
        ("GET", "/auth/v1/reauthenticate", StatusCode::UNAUTHORIZED),
        // An empty JSON body is GoTrue's 400 bad request.
        ("POST", "/auth/v1/signup", StatusCode::BAD_REQUEST),
        ("GET", "/storage/v1/object/b/k", StatusCode::NOT_IMPLEMENTED),
        ("GET", "/", StatusCode::NOT_IMPLEMENTED),
        ("GET", HEALTH_PATH, StatusCode::OK),
    ] {
        for origin in [None, Some("http://localhost:3000")] {
            let request_headers: Vec<(&str, &str)> = origin
                .map(|origin| ("origin", origin))
                .into_iter()
                .chain(std::iter::once(("content-type", "application/json")))
                .collect();
            let (status, headers, _) = send(method, path, &request_headers).await;
            assert_eq!(status, expected, "{method} {path} origin={origin:?}");
            assert_eq!(
                headers[header::ACCESS_CONTROL_ALLOW_ORIGIN],
                "*",
                "{method} {path} origin={origin:?}"
            );
            assert_eq!(
                headers
                    .get_all(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .iter()
                    .count(),
                1,
                "{method} {path} origin={origin:?}"
            );
        }
    }
}

#[tokio::test]
async fn kong_services_without_the_plugin_get_no_cors_answer() {
    assert_eq!(
        ROUTES_WITHOUT_CORS,
        [
            "/pg/",
            "/realtime/v1/api/openapi",
            "/realtime/v1/api/tenants",
            "/api/mcp",
            "/mcp",
        ]
    );
    for path in [
        "/pg/tables",
        "/realtime/v1/api/openapi",
        "/realtime/v1/api/tenants/realtime-dev",
        "/api/mcp",
        "/mcp",
    ] {
        let (status, headers, _) = send("OPTIONS", path, BROWSER_PREFLIGHT).await;
        // `/pg/` admits only the admin ACL group, so a client key is Kong's 403.
        let expected = if path.starts_with("/pg/") {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::NOT_IMPLEMENTED
        };
        assert_eq!(status, expected, "{path}");
        assert!(
            headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
            "{path}"
        );
        let (_, headers, _) = send("GET", path, &[("origin", "http://localhost:3000")]).await;
        assert!(
            headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
            "{path}"
        );
    }
}
