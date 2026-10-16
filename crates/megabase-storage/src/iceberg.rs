//! `/iceberg/**`: the Iceberg REST catalog routes, answered as storage-api
//! does when the catalog is not enabled (`specs/storage/iceberg.md`).

// Ported from supabase/storage src/http/routes/iceberg/index.ts,
// src/http/routes/iceberg/bucket.ts, src/http/routes/iceberg/catalog.ts,
// src/http/routes/iceberg/namespace.ts, src/http/routes/iceberg/table.ts and
// src/config.ts (Apache-2.0), pin v1.80.2. The 404 body reproduces the
// response of Fastify's default not-found handler; no Fastify code is ported.

use axum::{
    extract::OriginalUri,
    http::{header, HeaderMap, Method, StatusCode, Uri},
    response::Response,
};
use serde_json::{json, Value};

use super::{fastify_json, not_implemented};

/// The Kong route prefix storage-api never sees (`strip_path: true`).
const KONG_PREFIX: &str = "/storage/v1";

/// Every `/iceberg` path. `routes/iceberg/index.ts` returns before
/// registering a route when neither `ICEBERG_ENABLED=true` nor
/// `MULTI_TENANT=true` is set, and Megabase has neither setting, so each
/// request reaches Fastify's default not-found handler.
///
/// Fastify parses a request body before that handler runs, and a parse
/// failure is answered by the root error handler instead. That path is
/// not ported: a request other than `GET`/`HEAD` that carries a
/// `Content-Type` or a body is a 501.
// megabase:unit storage:route:DELETE /storage/v1/iceberg/v1/{prefix}/namespaces/{namespace}/tables/{table}
// megabase:unit storage:route:GET /storage/v1/iceberg/bucket
// megabase:unit storage:route:GET /storage/v1/iceberg/v1/config
// megabase:unit storage:route:GET /storage/v1/iceberg/v1/{prefix}/namespaces
// megabase:unit storage:route:GET /storage/v1/iceberg/v1/{prefix}/namespaces/{namespace}
// megabase:unit storage:route:GET /storage/v1/iceberg/v1/{prefix}/namespaces/{namespace}/tables
// megabase:unit storage:route:GET /storage/v1/iceberg/v1/{prefix}/namespaces/{namespace}/tables/{table}
// megabase:unit storage:route:HEAD /storage/v1/iceberg/v1/{prefix}/namespaces/{namespace}
// megabase:unit storage:route:HEAD /storage/v1/iceberg/v1/{prefix}/namespaces/{namespace}/tables/{table}
// megabase:unit storage:route:POST /storage/v1/iceberg/bucket
// megabase:unit storage:route:POST /storage/v1/iceberg/v1/{prefix}/namespaces
// megabase:unit storage:route:POST /storage/v1/iceberg/v1/{prefix}/namespaces/{namespace}/tables
// megabase:unit storage:route:POST /storage/v1/iceberg/v1/{prefix}/namespaces/{namespace}/tables/{table}
pub(super) async fn catalog_disabled(
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    if !(method == Method::GET || method == Method::HEAD) && has_body(&headers) {
        return not_implemented(&method, &uri);
    }
    fastify_json(StatusCode::NOT_FOUND, &route_not_found(&method, &uri))
}

/// Fastify's `handleRequest` skips body parsing only when there is no
/// `Content-Type`, no `Transfer-Encoding`, and `Content-Length` is absent
/// or `0`.
fn has_body(headers: &HeaderMap) -> bool {
    headers.contains_key(header::CONTENT_TYPE)
        || headers.contains_key(header::TRANSFER_ENCODING)
        || headers
            .get(header::CONTENT_LENGTH)
            .is_some_and(|length| length.as_bytes() != b"0")
}

/// `basic404`: `request.raw.url` is the path and query string after Kong
/// strips `/storage/v1`.
fn route_not_found(method: &Method, uri: &Uri) -> Value {
    let raw = uri.path_and_query().map_or(uri.path(), |pq| pq.as_str());
    let url = raw.strip_prefix(KONG_PREFIX).unwrap_or(raw);
    json!({
        "message": format!("Route {method}:{url} not found"),
        "error": "Not Found",
        "statusCode": 404,
    })
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use tower::ServiceExt;

    use super::super::{router_with_state, StorageState};
    use super::*;

    async fn send(
        method: &str,
        path: &str,
        extra: &[(header::HeaderName, &str)],
        body: &'static str,
    ) -> (StatusCode, HeaderMap, Option<Value>) {
        let mut request = axum::http::Request::builder().method(method).uri(path);
        for (name, value) in extra {
            request = request.header(name, *value);
        }
        let response = router_with_state(StorageState::default())
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, headers, serde_json::from_slice(&bytes).ok())
    }

    const ROUTES: [(&str, &str); 13] = [
        ("GET", "/iceberg/bucket"),
        ("POST", "/iceberg/bucket"),
        ("DELETE", "/iceberg/bucket/analytics"),
        ("GET", "/iceberg/v1/config?warehouse=analytics"),
        ("GET", "/iceberg/v1/analytics/namespaces"),
        ("POST", "/iceberg/v1/analytics/namespaces"),
        ("GET", "/iceberg/v1/analytics/namespaces/ns"),
        ("HEAD", "/iceberg/v1/analytics/namespaces/ns"),
        ("GET", "/iceberg/v1/analytics/namespaces/ns/tables"),
        ("POST", "/iceberg/v1/analytics/namespaces/ns/tables"),
        ("GET", "/iceberg/v1/analytics/namespaces/ns/tables/t"),
        ("HEAD", "/iceberg/v1/analytics/namespaces/ns/tables/t"),
        ("DELETE", "/iceberg/v1/analytics/namespaces/ns/tables/t"),
    ];

    #[tokio::test]
    async fn every_route_is_fastifys_not_found() {
        for (method, path) in ROUTES {
            let (status, headers, body) =
                send(method, &format!("{KONG_PREFIX}{path}"), &[], "").await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
            assert_eq!(
                headers[header::CONTENT_TYPE],
                "application/json; charset=utf-8"
            );
            assert_eq!(
                body,
                Some(json!({
                    "message": format!("Route {method}:{path} not found"),
                    "error": "Not Found",
                    "statusCode": 404,
                })),
                "{method} {path}"
            );
        }
    }

    #[tokio::test]
    async fn the_prefix_itself_and_other_methods_are_not_found() {
        for (method, path) in [
            ("GET", "/iceberg"),
            ("GET", "/iceberg/"),
            ("PUT", "/iceberg/v1/analytics/namespaces/ns"),
            ("POST", "/iceberg/v1/analytics/namespaces/ns/tables/t"),
        ] {
            let (status, _, body) = send(method, &format!("{KONG_PREFIX}{path}"), &[], "").await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
            assert_eq!(
                body.unwrap()["message"],
                format!("Route {method}:{path} not found")
            );
        }
    }

    #[tokio::test]
    async fn auth_headers_do_not_matter() {
        let (status, _, _) = send(
            "GET",
            "/storage/v1/iceberg/bucket",
            &[(header::AUTHORIZATION, "Bearer not-a-jwt")],
            "",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn requests_with_a_body_stay_501() {
        for (extra, body) in [
            (vec![(header::CONTENT_TYPE, "application/json")], "{}"),
            (vec![(header::CONTENT_TYPE, "application/json")], ""),
            (vec![(header::CONTENT_LENGTH, "2")], "{}"),
        ] {
            let (status, _, _) = send("POST", "/storage/v1/iceberg/bucket", &extra, body).await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        }
        // `GET` and `HEAD` are never parsed.
        let (status, _, _) = send(
            "GET",
            "/storage/v1/iceberg/bucket",
            &[(header::CONTENT_TYPE, "application/json")],
            "",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
