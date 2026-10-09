//! Admin route tests that do not need PostgreSQL. Database-backed bodies are
//! judge cases.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use megabase_auth::{router_with_state, AuthState};
use serde_json::Value;
use tower::ServiceExt;

/// `JWT_SECRET` from `vendor/supabase/docker/.env.example`.
const DEMO_SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";
const DEMO_ANON: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyAgCiAgICAicm9sZSI6ICJhbm9uIiwKICAgICJpc3MiOiAic3VwYWJhc2UtZGVtbyIsCiAgICAiaWF0IjogMTY0MTc2OTIwMCwKICAgICJleHAiOiAxNzk5NTM1NjAwCn0.dc_X5iR_VP_qT0zsiyj_I_OZ2T9FtRU2BBNWN8Bu4GE";
const DEMO_SERVICE: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyAgCiAgICAicm9sZSI6ICJzZXJ2aWNlX3JvbGUiLAogICAgImlzcyI6ICJzdXBhYmFzZS1kZW1vIiwKICAgICJpYXQiOiAxNjQxNzY5MjAwLAogICAgImV4cCI6IDE3OTk1MzU2MDAKfQ.DaYlNEoUrrEn2Ig7tqibS-PHK5vgusbcbo7X36XVt4Q";

fn state() -> AuthState {
    AuthState::from_lookup(|key| (key == "JWT_SECRET").then(|| DEMO_SECRET.into()))
}

async fn send(
    state: AuthState,
    method: &str,
    path: &str,
    authorization: Option<&str>,
) -> (StatusCode, Value, axum::http::HeaderMap) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(token) = authorization {
        builder = builder.header("Authorization", format!("Bearer {token}"));
    }
    let response = router_with_state(state)
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    (status, body, headers)
}

fn assert_gotrue_error(body: &Value, status: u16, error_code: &str, msg: &str) {
    assert_eq!(body["code"], status, "{body}");
    assert_eq!(body["error_code"], error_code, "{body}");
    assert_eq!(body["msg"], msg, "{body}");
}

const ADMIN_ROUTES: &[(&str, &str)] = &[
    ("GET", "/auth/v1/admin/audit"),
    ("GET", "/auth/v1/admin/custom-providers"),
    ("GET", "/auth/v1/admin/custom-providers/custom:example"),
    ("DELETE", "/auth/v1/admin/custom-providers/custom:example"),
    ("GET", "/auth/v1/admin/oauth/clients"),
    (
        "DELETE",
        "/auth/v1/admin/oauth/clients/11111111-1111-1111-1111-111111111111",
    ),
    (
        "DELETE",
        "/auth/v1/admin/sso/providers/11111111-1111-1111-1111-111111111111",
    ),
    (
        "DELETE",
        "/auth/v1/admin/users/11111111-1111-1111-1111-111111111111",
    ),
    (
        "DELETE",
        "/auth/v1/admin/users/11111111-1111-1111-1111-111111111111/factors/22222222-2222-2222-2222-222222222222",
    ),
    (
        "DELETE",
        "/auth/v1/admin/users/11111111-1111-1111-1111-111111111111/passkeys/22222222-2222-2222-2222-222222222222",
    ),
];

#[tokio::test]
async fn admin_without_bearer_is_401() {
    for (method, path) in ADMIN_ROUTES {
        let (status, body, headers) = send(state(), method, path, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
        assert_gotrue_error(
            &body,
            401,
            "no_authorization",
            "This endpoint requires a valid Bearer token",
        );
        assert_eq!(
            headers.get("x-sb-error-code").and_then(|v| v.to_str().ok()),
            Some("no_authorization")
        );
    }
}

#[tokio::test]
async fn admin_anon_is_403_not_admin() {
    for path in [
        "/auth/v1/admin/audit",
        "/auth/v1/admin/custom-providers",
        "/auth/v1/admin/oauth/clients",
    ] {
        let (status, body, _) = send(state(), "GET", path, Some(DEMO_ANON)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
        assert_gotrue_error(&body, 403, "not_admin", "User not allowed");
    }
}

#[tokio::test]
async fn oauth_clients_service_role_is_feature_disabled() {
    for (method, path) in [
        ("GET", "/auth/v1/admin/oauth/clients"),
        (
            "DELETE",
            "/auth/v1/admin/oauth/clients/11111111-1111-1111-1111-111111111111",
        ),
    ] {
        let (status, body, _) = send(state(), method, path, Some(DEMO_SERVICE)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
        assert_gotrue_error(&body, 404, "feature_disabled", "OAuth server is disabled");
    }
}

#[tokio::test]
async fn custom_oauth_can_be_disabled() {
    let state = AuthState::from_lookup(|key| match key {
        "JWT_SECRET" => Some(DEMO_SECRET.into()),
        "GOTRUE_CUSTOM_OAUTH_ENABLED" => Some("false".into()),
        _ => None,
    });
    let (status, body, _) = send(
        state,
        "GET",
        "/auth/v1/admin/custom-providers",
        Some(DEMO_SERVICE),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_gotrue_error(
        &body,
        404,
        "feature_disabled",
        "Custom OAuth providers are disabled",
    );
}

#[tokio::test]
async fn unimplemented_auth_paths_stay_501() {
    let (status, body, _) = send(state(), "POST", "/auth/v1/admin/users", Some(DEMO_SERVICE)).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(body["code"], "MEGABASE_NOT_IMPLEMENTED");
    assert_eq!(body["unit"], "POST /auth/v1/admin/users");
}

#[tokio::test]
async fn default_router_reads_empty_env() {
    let (status, body, _) = send(
        AuthState::from_lookup(|_| None),
        "GET",
        "/auth/v1/admin/audit",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_gotrue_error(
        &body,
        401,
        "no_authorization",
        "This endpoint requires a valid Bearer token",
    );
}

#[tokio::test]
async fn audit_rejects_non_integer_page_before_db() {
    let (status, body, _) = send(
        state(),
        "GET",
        "/auth/v1/admin/audit?page=nope",
        Some(DEMO_SERVICE),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_gotrue_error(
        &body,
        400,
        "validation_failed",
        "Bad Pagination Parameters: strconv.ParseUint: parsing \"nope\": invalid syntax",
    );
}

#[tokio::test]
async fn audit_rejects_unknown_query_scope() {
    let (status, body, _) = send(
        state(),
        "GET",
        "/auth/v1/admin/audit?query=bogus:x",
        Some(DEMO_SERVICE),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_gotrue_error(
        &body,
        400,
        "validation_failed",
        "Invalid query scope: bogus:x",
    );
}

#[tokio::test]
async fn custom_provider_identifier_must_use_prefix() {
    let (status, body, _) = send(
        state(),
        "GET",
        "/auth/v1/admin/custom-providers/example",
        Some(DEMO_SERVICE),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_gotrue_error(
        &body,
        400,
        "validation_failed",
        "identifier must start with 'custom:' prefix, e.g. 'custom:example'",
    );
}

#[tokio::test]
async fn delete_user_rejects_non_uuid_before_db() {
    let (status, body, _) = send(
        state(),
        "DELETE",
        "/auth/v1/admin/users/not-a-uuid",
        Some(DEMO_SERVICE),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_gotrue_error(&body, 404, "validation_failed", "user_id must be an UUID");
}

#[tokio::test]
async fn delete_oauth_client_rejects_non_uuid_when_enabled() {
    let state = AuthState::from_lookup(|key| match key {
        "JWT_SECRET" => Some(DEMO_SECRET.into()),
        "GOTRUE_OAUTH_SERVER_ENABLED" => Some("true".into()),
        _ => None,
    });
    let (status, body, _) = send(
        state,
        "DELETE",
        "/auth/v1/admin/oauth/clients/not-a-uuid",
        Some(DEMO_SERVICE),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_gotrue_error(&body, 400, "validation_failed", "invalid client_id format");
}

#[tokio::test]
async fn delete_sso_rejects_non_uuid_before_db() {
    let (status, body, _) = send(
        state(),
        "DELETE",
        "/auth/v1/admin/sso/providers/not-a-uuid",
        Some(DEMO_SERVICE),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_gotrue_error(
        &body,
        404,
        "sso_provider_not_found",
        "SSO Identity Provider not found",
    );
}
