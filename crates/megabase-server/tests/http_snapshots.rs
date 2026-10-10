//! Snapshot the stable HTTP status and JSON body of endpoints that exist today.
//!
//! `x-request-id` is generated per call, so these snapshots omit it. A fixed
//! incoming id is checked separately.
//!
//! `GET /_megabase/health` reports the crate version from `CARGO_PKG_VERSION`.
//! The test asserts that field, then stores `[version]` in the snapshot, so
//! a release-please bump does not rewrite the file. Insta's `redactions`
//! feature is not used: it depends on `pest` and `pest_derive`, which are
//! not on the `deny.toml` allow list. The other snapshots do not include
//! the crate version. `/auth/v1/health` reports the pinned GoTrue version.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use megabase_server::create_router;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn observe(method: &str, path: &str, body: Option<&str>) -> Value {
    let mut builder = Request::builder().method(method).uri(path);
    builder = builder.header("x-request-id", "snapshot");
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let request = builder
        .body(match body {
            Some(body) => Body::from(body.to_string()),
            None => Body::empty(),
        })
        .unwrap();
    let response = create_router().oneshot(request).await.unwrap();
    let status = response.status();
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&bytes) }))
    };
    assert_eq!(request_id, "snapshot", "{method} {path}");
    json!({
        "status": status.as_u16(),
        "content_type": content_type,
        "body": parsed,
    })
}

#[tokio::test]
async fn health_and_not_implemented_bodies() {
    let mut health = observe("GET", "/_megabase/health", None).await;
    assert_eq!(
        health["body"]["version"].as_str(),
        Some(env!("CARGO_PKG_VERSION")),
        "GET /_megabase/health version is the crate version"
    );
    health["body"]["version"] = json!("[version]");
    insta::assert_json_snapshot!("megabase_health", health);
    insta::assert_json_snapshot!("auth_health", observe("GET", "/auth/v1/health", None).await);
    insta::assert_json_snapshot!(
        "auth_settings",
        observe("GET", "/auth/v1/settings", None).await
    );
    insta::assert_json_snapshot!(
        "rest_not_implemented",
        observe("GET", "/rest/v1/todos", None).await
    );
    insta::assert_json_snapshot!(
        "auth_reauthenticate_not_implemented",
        observe("GET", "/auth/v1/reauthenticate", None).await
    );
    insta::assert_json_snapshot!("studio_fallback", observe("GET", "/", None).await);
}

#[tokio::test]
async fn auth_error_bodies() {
    let weak = observe(
        "POST",
        "/auth/v1/signup",
        Some(r#"{"email":"a@example.com","password":"123"}"#),
    )
    .await;
    assert_eq!(weak["status"], StatusCode::UNPROCESSABLE_ENTITY.as_u16());
    insta::assert_json_snapshot!("signup_weak_password", weak);

    let missing = observe("GET", "/auth/v1/admin/audit", None).await;
    assert_eq!(missing["status"], StatusCode::UNAUTHORIZED.as_u16());
    insta::assert_json_snapshot!("admin_missing_bearer", missing);

    let bad_json = observe("POST", "/auth/v1/signup", Some("not-json")).await;
    insta::assert_json_snapshot!("signup_bad_json", bad_json);
}
