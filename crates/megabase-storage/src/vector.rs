//! `/vector/**`: the S3 Vectors routes, answered as storage-api does when
//! `VECTOR_ENABLED` is not set (`specs/storage/vector.md`).

// Ported from supabase/storage src/http/routes/vector/index.ts,
// src/http/not-found-handler.ts, src/http/schemas/error.ts and src/config.ts
// (Apache-2.0), pin v1.80.2. The 404 body reproduces the not-found handler
// the vector plugin installs; no Fastify code is ported.

use axum::{
    body::Bytes,
    extract::OriginalUri,
    http::{header, HeaderMap, Method, StatusCode, Uri},
    response::Response,
};
use serde_json::{json, Value};

use super::iceberg::{has_body, has_framed_content_type};
use super::{fastify_json, not_implemented};

/// The Kong route prefix storage-api never sees (`strip_path: true`).
const KONG_PREFIX: &str = "/storage/v1";

/// Every `/vector` path. `routes/vector/index.ts` installs
/// `setRestNotFoundHandler` and returns before registering a route when
/// neither `VECTOR_ENABLED=true` nor `MULTI_TENANT=true` is set. Megabase has
/// neither setting, so each request reaches that handler.
///
/// Fastify parses a body before the handler runs. Storage-api's catch-all
/// parser accepts any media type except `application/json`, whose parser
/// rejects an empty or malformed body (and `__proto__` / `constructor`
/// keys) through the root error handler. That path is not ported: such a
/// request is a 501.
// megabase:unit storage:route:POST /storage/v1/vector/CreateIndex
// megabase:unit storage:route:POST /storage/v1/vector/CreateVectorBucket
// megabase:unit storage:route:POST /storage/v1/vector/DeleteIndex
// megabase:unit storage:route:POST /storage/v1/vector/DeleteVectorBucket
// megabase:unit storage:route:POST /storage/v1/vector/DeleteVectors
// megabase:unit storage:route:POST /storage/v1/vector/GetIndex
// megabase:unit storage:route:POST /storage/v1/vector/GetVectorBucket
// megabase:unit storage:route:POST /storage/v1/vector/GetVectors
// megabase:unit storage:route:POST /storage/v1/vector/ListIndexes
// megabase:unit storage:route:POST /storage/v1/vector/ListVectorBuckets
pub(super) async fn disabled(
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Result<Bytes, axum::extract::rejection::BytesRejection>,
) -> Response {
    let parses_body = match method {
        Method::POST | Method::PUT | Method::PATCH => has_body(&headers),
        Method::DELETE | Method::OPTIONS => has_framed_content_type(&headers),
        _ => false,
    };
    if parses_body && is_json(&headers) {
        // Over the 1 MiB default body limit the root handler answers 413.
        let Ok(body) = body else {
            return not_implemented(&method, &uri);
        };
        if !parses_as_json(&body) {
            return not_implemented(&method, &uri);
        }
    }
    fastify_json(StatusCode::NOT_FOUND, &route_not_found(&method, &uri))
}

/// Fastify picks the JSON parser for the exact media type `application/json`.
fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
}

/// A non-empty, well-formed JSON body without a poisoned key.
fn parses_as_json(body: &[u8]) -> bool {
    if body.len() > 1_048_576 {
        return false;
    }
    let Ok(text) = std::str::from_utf8(body) else {
        return false;
    };
    !text.trim().is_empty()
        && !text.contains("__proto__")
        && !text.contains("constructor")
        && serde_json::from_str::<Value>(text).is_ok()
}

/// `setRestNotFoundHandler` body, serialized with `errorSchema`: every field
/// is a string and `statusCode` is `"404"`.
fn route_not_found(method: &Method, uri: &Uri) -> Value {
    let raw = uri.path_and_query().map_or(uri.path(), |pq| pq.as_str());
    let url = raw.strip_prefix(KONG_PREFIX).unwrap_or(raw);
    json!({
        "statusCode": "404",
        "error": "Not Found",
        "message": format!("Route {method}:{url} not found"),
        "code": "InvalidRequest",
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

    const ACTIONS: [&str; 13] = [
        "CreateIndex",
        "CreateVectorBucket",
        "DeleteIndex",
        "DeleteVectorBucket",
        "DeleteVectors",
        "GetIndex",
        "GetVectorBucket",
        "GetVectors",
        "ListIndexes",
        "ListVectorBuckets",
        "ListVectors",
        "PutVectors",
        "QueryVectors",
    ];

    fn json_type() -> [(header::HeaderName, &'static str); 1] {
        [(header::CONTENT_TYPE, "application/json")]
    }

    #[tokio::test]
    async fn every_action_is_the_rest_not_found() {
        for action in ACTIONS {
            let path = format!("/vector/{action}");
            let (status, headers, body) = send(
                "POST",
                &format!("{KONG_PREFIX}{path}"),
                &json_type(),
                r#"{"vectorBucketName":"b"}"#,
            )
            .await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{action}");
            assert_eq!(
                headers[header::CONTENT_TYPE],
                "application/json; charset=utf-8"
            );
            assert_eq!(
                body,
                Some(json!({
                    "statusCode": "404",
                    "error": "Not Found",
                    "message": format!("Route POST:{path} not found"),
                    "code": "InvalidRequest",
                })),
                "{action}"
            );
        }
    }

    #[tokio::test]
    async fn the_prefix_other_methods_and_queries_are_not_found() {
        for (method, path) in [
            ("GET", "/vector"),
            ("GET", "/vector/"),
            ("GET", "/vector/ListVectorBuckets?x=1"),
            ("PUT", "/vector/CreateIndex"),
            ("POST", "/vector/Unknown"),
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
    async fn auth_headers_and_other_media_types_do_not_matter() {
        let extra = [
            (header::AUTHORIZATION, "Bearer not-a-jwt"),
            (header::CONTENT_TYPE, "text/plain"),
        ];
        let (status, _, _) = send("POST", "/storage/v1/vector/GetIndex", &extra, "x").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn json_bodies_the_parser_rejects_stay_501() {
        for body in [
            "",
            "{",
            "   ",
            r#"{"__proto__":{}}"#,
            r#"{"constructor":1}"#,
        ] {
            let (status, _, _) =
                send("POST", "/storage/v1/vector/GetIndex", &json_type(), body).await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{body:?}");
        }
        let ct = [(header::CONTENT_TYPE, "application/json; charset=utf-8")];
        let (status, _, _) = send("POST", "/storage/v1/vector/GetIndex", &ct, "{}").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
