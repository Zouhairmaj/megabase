//! Admin route tests that do not need PostgreSQL. Database-backed bodies are
//! judge cases.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use megabase_auth::{router_with_state, AuthState};
use serde_json::Value;
use sha2::Sha256;
use tower::ServiceExt;

/// `JWT_SECRET` from `vendor/supabase/docker/.env.example`.
/// Vendor `ANON_KEY` / `SERVICE_ROLE_KEY` expire 2027-01-10; tests sign fresh tokens.
const DEMO_SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";

fn sign_role(role: &str) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let exp = now.saturating_add(60 * 60 * 24 * 365 * 10);
    let payload = format!(r#"{{"role":"{role}","iss":"supabase-demo","iat":{now},"exp":{exp}}}"#);
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload.as_bytes());
    let signing_input = format!("{header}.{payload_b64}");
    let mut mac = Hmac::<Sha256>::new_from_slice(DEMO_SECRET.as_bytes()).expect("secret");
    mac.update(signing_input.as_bytes());
    let sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    format!("{signing_input}.{sig}")
}

fn demo_anon() -> String {
    sign_role("anon")
}

fn demo_service() -> String {
    sign_role("service_role")
}

fn state() -> AuthState {
    AuthState::from_lookup(|key| (key == "JWT_SECRET").then(|| DEMO_SECRET.into()))
}

async fn send(
    state: AuthState,
    method: &str,
    path: &str,
    authorization: Option<&str>,
) -> (StatusCode, Value, axum::http::HeaderMap) {
    send_body(state, method, path, authorization, None).await
}

async fn send_json(
    state: AuthState,
    method: &str,
    path: &str,
    authorization: Option<&str>,
    json: &str,
) -> (StatusCode, Value, axum::http::HeaderMap) {
    send_body(state, method, path, authorization, Some(json)).await
}

async fn send_body(
    state: AuthState,
    method: &str,
    path: &str,
    authorization: Option<&str>,
    json: Option<&str>,
) -> (StatusCode, Value, axum::http::HeaderMap) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(token) = authorization {
        builder = builder.header("Authorization", format!("Bearer {token}"));
    }
    let body = match json {
        Some(json) => {
            builder = builder.header("content-type", "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let response = router_with_state(state)
        .oneshot(builder.body(body).unwrap())
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
    ("GET", "/auth/v1/admin/users"),
    (
        "GET",
        "/auth/v1/admin/users/11111111-1111-1111-1111-111111111111",
    ),
    (
        "GET",
        "/auth/v1/admin/users/11111111-1111-1111-1111-111111111111/factors",
    ),
    (
        "GET",
        "/auth/v1/admin/users/11111111-1111-1111-1111-111111111111/passkeys",
    ),
    ("GET", "/auth/v1/admin/sso/providers"),
    (
        "GET",
        "/auth/v1/admin/sso/providers/11111111-1111-1111-1111-111111111111",
    ),
    (
        "GET",
        "/auth/v1/admin/oauth/clients/11111111-1111-1111-1111-111111111111",
    ),
    ("POST", "/auth/v1/admin/generate_link"),
    ("POST", "/auth/v1/admin/custom-providers"),
    ("POST", "/auth/v1/admin/oauth/clients"),
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
        "/auth/v1/admin/users",
        "/auth/v1/admin/users/11111111-1111-1111-1111-111111111111",
        "/auth/v1/admin/users/11111111-1111-1111-1111-111111111111/factors",
        "/auth/v1/admin/users/11111111-1111-1111-1111-111111111111/passkeys",
        "/auth/v1/admin/sso/providers",
        "/auth/v1/admin/sso/providers/11111111-1111-1111-1111-111111111111",
    ] {
        let (status, body, _) = send(state(), "GET", path, Some(&demo_anon())).await;
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
        ("GET", "/auth/v1/admin/oauth/clients/not-a-uuid"),
        ("POST", "/auth/v1/admin/oauth/clients"),
    ] {
        let (status, body, _) = send(state(), method, path, Some(&demo_service())).await;
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
        "POST",
        "/auth/v1/admin/custom-providers",
        Some(&demo_service()),
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
    let (status, body, _) = send(
        state(),
        "POST",
        "/auth/v1/admin/unimplemented",
        Some(&demo_service()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(body["code"], "MEGABASE_NOT_IMPLEMENTED");
    assert_eq!(body["unit"], "POST /auth/v1/admin/unimplemented");
}

#[tokio::test]
async fn unimplemented_methods_on_registered_admin_paths_are_501() {
    for (method, path, unit) in [
        (
            "PUT",
            "/auth/v1/admin/custom-providers",
            "PUT /auth/v1/admin/custom-providers",
        ),
        (
            "POST",
            "/auth/v1/admin/users/11111111-1111-1111-1111-111111111111/factors",
            "POST /auth/v1/admin/users/11111111-1111-1111-1111-111111111111/factors",
        ),
    ] {
        let (status, body, _) = send(state(), method, path, Some(&demo_service())).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{method} {path}");
        assert_eq!(body["code"], "MEGABASE_NOT_IMPLEMENTED", "{body}");
        assert_eq!(body["component"], "auth", "{body}");
        assert_eq!(body["unit"], unit, "{body}");
    }
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
        Some(&demo_service()),
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
        Some(&demo_service()),
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
        Some(&demo_service()),
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
        Some(&demo_service()),
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
        Some(&demo_service()),
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
        Some(&demo_service()),
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

#[tokio::test]
async fn custom_provider_type_must_be_oauth2_or_oidc() {
    let (status, body, _) = send(
        state(),
        "GET",
        "/auth/v1/admin/custom-providers?type=saml",
        Some(&demo_service()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_gotrue_error(
        &body,
        400,
        "validation_failed",
        "type must be either 'oauth2' or 'oidc'",
    );
}

#[tokio::test]
async fn oauth_clients_enabled_without_database_is_500() {
    let state = AuthState::from_lookup(|key| match key {
        "JWT_SECRET" => Some(DEMO_SECRET.into()),
        "GOTRUE_OAUTH_SERVER_ENABLED" => Some("true".into()),
        _ => None,
    });
    let (status, body, _) = send(
        state,
        "GET",
        "/auth/v1/admin/oauth/clients",
        Some(&demo_service()),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_gotrue_error(&body, 500, "unexpected_failure", "Database error");
}

#[tokio::test]
async fn admin_users_reject_bad_page_and_sort_before_db() {
    let (status, body, _) = send(
        state(),
        "GET",
        "/auth/v1/admin/users?page=nope",
        Some(&demo_service()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_gotrue_error(
        &body,
        400,
        "validation_failed",
        "Bad Pagination Parameters: strconv.ParseUint: parsing \"nope\": invalid syntax",
    );

    let (status, body, _) = send(
        state(),
        "GET",
        "/auth/v1/admin/users?sort=email%20desc",
        Some(&demo_service()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_gotrue_error(
        &body,
        400,
        "validation_failed",
        "Bad Sort Parameters: bad field for sort 'email'",
    );
}

#[tokio::test]
async fn admin_user_reads_reject_non_uuid_before_db() {
    for path in [
        "/auth/v1/admin/users/not-a-uuid",
        "/auth/v1/admin/users/not-a-uuid/factors",
        "/auth/v1/admin/users/not-a-uuid/passkeys",
    ] {
        let (status, body, _) = send(state(), "GET", path, Some(&demo_service())).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_gotrue_error(&body, 404, "validation_failed", "user_id must be an UUID");
    }
}

#[tokio::test]
async fn get_sso_provider_rejects_non_uuid_before_db() {
    let (status, body, _) = send(
        state(),
        "GET",
        "/auth/v1/admin/sso/providers/not-a-uuid",
        Some(&demo_service()),
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

#[tokio::test]
async fn generate_link_validates_email_and_json_before_db() {
    let (status, body, _) = send_json(
        state(),
        "POST",
        "/auth/v1/admin/generate_link",
        Some(&demo_service()),
        "{}",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_gotrue_error(
        &body,
        400,
        "validation_failed",
        "An email address is required",
    );

    let (status, body, _) = send_json(
        state(),
        "POST",
        "/auth/v1/admin/generate_link",
        Some(&demo_service()),
        r#"{"type":"magiclink","email":"not-an-email"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_gotrue_error(
        &body,
        400,
        "validation_failed",
        "Unable to validate email address: invalid format",
    );

    let (status, body, _) = send_json(
        state(),
        "POST",
        "/auth/v1/admin/generate_link",
        Some(&demo_service()),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_gotrue_error(
        &body,
        400,
        "bad_json",
        "Could not parse request body as JSON: unexpected end of JSON input",
    );
}

#[tokio::test]
async fn post_oauth_client_validation_when_enabled() {
    let disabled = send_json(
        state(),
        "POST",
        "/auth/v1/admin/oauth/clients",
        Some(&demo_service()),
        "not-json",
    )
    .await;
    assert_eq!(disabled.0, StatusCode::NOT_FOUND);
    assert_gotrue_error(
        &disabled.1,
        404,
        "feature_disabled",
        "OAuth server is disabled",
    );

    let enabled = AuthState::from_lookup(|key| match key {
        "JWT_SECRET" => Some(DEMO_SECRET.into()),
        "GOTRUE_OAUTH_SERVER_ENABLED" => Some("true".into()),
        _ => None,
    });
    let (status, body, _) = send_json(
        enabled,
        "POST",
        "/auth/v1/admin/oauth/clients",
        Some(&demo_service()),
        "not-json",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_gotrue_error(&body, 400, "bad_json", "Invalid JSON body");

    let enabled = AuthState::from_lookup(|key| match key {
        "JWT_SECRET" => Some(DEMO_SECRET.into()),
        "GOTRUE_OAUTH_SERVER_ENABLED" => Some("true".into()),
        _ => None,
    });
    let (status, body, _) = send_json(
        enabled,
        "POST",
        "/auth/v1/admin/oauth/clients",
        Some(&demo_service()),
        "{}",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_gotrue_error(
        &body,
        400,
        "validation_failed",
        "400: redirect_uris is required",
    );
}

#[tokio::test]
async fn post_custom_provider_validates_before_db() {
    let (status, body, _) = send_json(
        state(),
        "POST",
        "/auth/v1/admin/custom-providers",
        Some(&demo_service()),
        r#"{"provider_type":"saml"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_gotrue_error(
        &body,
        400,
        "validation_failed",
        "provider_type must be either 'oauth2' or 'oidc'",
    );

    let (status, body, _) = send_json(
        state(),
        "POST",
        "/auth/v1/admin/custom-providers",
        Some(&demo_service()),
        r#"{"provider_type":"oauth2","identifier":"example","name":"Example","client_id":"id","client_secret":"secret","authorization_url":"https://example.com","token_url":"https://example.com","userinfo_url":"https://example.com"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_gotrue_error(
        &body,
        400,
        "validation_failed",
        "identifier must start with 'custom:' prefix, e.g. 'custom:example'",
    );

    let (status, body, _) = send_json(
        state(),
        "POST",
        "/auth/v1/admin/custom-providers",
        Some(&demo_service()),
        r#"{"provider_type":"oauth2","identifier":"custom:example","name":"Example","client_id":"id","client_secret":"secret","authorization_url":"http://example.com/auth","token_url":"https://example.com/token","userinfo_url":"https://example.com/user"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_gotrue_error(&body, 400, "validation_failed", "URL must use HTTPS");
}
