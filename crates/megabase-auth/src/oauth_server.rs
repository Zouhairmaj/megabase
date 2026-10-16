// Ported from supabase/auth internal/api/api.go, internal/api/middleware.go,
// internal/api/oauthserver/handlers.go and internal/models/oauth_scope.go
// (MIT), pin v2.197.0.

//! OAuth 2.1 server routes under `/auth/v1/oauth` (Supabase Auth as the
//! authorization server).
//!
//! GoTrue mounts all six routes in one chi sub-router whose first middleware
//! is `requireOAuthServerEnabled`, so every method on every route is 404
//! `feature_disabled` while `GOTRUE_OAUTH_SERVER_ENABLED` is off (the
//! reference stack). With the server on, dynamic client registration and
//! `userinfo` are served; the authorization-code flow (`authorize`,
//! `authorizations/{id}`, `consent`, `token`) is not ported and answers 501.
//! Spec: `specs/auth/oauth.md`.

use std::time::UNIX_EPOCH;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header::ALLOW, header::CONTENT_TYPE, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use megabase_core::MegabaseNotImplemented;
use serde_json::{json, Map, Value};

use crate::admin::require_oauth_server;
use crate::admin_batch2::{register_oauth_client, OAuthRegister};
use crate::http::{json_status, AuthError};
use crate::ratelimit::limit_by_header;
use crate::routes::read_body;
use crate::state::AuthState;
use crate::store::UserRecord;
use crate::user::require_user;
use crate::well_known::go_marshal;
use crate::COMPONENT;

const UNIT_AUTHORIZE: &str = "auth:route:GET /auth/v1/oauth/authorize";
const UNIT_GET_AUTHORIZATION: &str =
    "auth:route:GET /auth/v1/oauth/authorizations/{authorization_id}";
const UNIT_CONSENT: &str =
    "auth:route:POST /auth/v1/oauth/authorizations/{authorization_id}/consent";
const UNIT_TOKEN: &str = "auth:route:POST /auth/v1/oauth/token";

pub(crate) fn router(state: AuthState) -> Router {
    Router::new()
        .route("/auth/v1/oauth/clients/register", any(register))
        .route("/auth/v1/oauth/token", any(token))
        .route("/auth/v1/oauth/userinfo", any(userinfo))
        .route("/auth/v1/oauth/authorize", any(authorize))
        .route(
            "/auth/v1/oauth/authorizations/:authorization_id",
            any(get_authorization),
        )
        .route(
            "/auth/v1/oauth/authorizations/:authorization_id/consent",
            any(consent),
        )
        .with_state(state)
}

/// `requireOAuthServerEnabled`, then chi's method match. The sub-router
/// middleware runs before routing, so a wrong method on a disabled server is
/// still 404 `feature_disabled`. `Some` is the rejection to send.
fn gate(state: &AuthState, method: &Method, allowed: &Method) -> Option<Response> {
    if let Err(error) = require_oauth_server(state) {
        return Some(error.into_response());
    }
    if method == allowed {
        return None;
    }
    let mut response = StatusCode::METHOD_NOT_ALLOWED.into_response();
    if let Ok(value) = HeaderValue::from_str(allowed.as_str()) {
        response.headers_mut().insert(ALLOW, value);
    }
    Some(response)
}

// megabase:unit auth:route:POST /auth/v1/oauth/clients/register
async fn register(
    State(state): State<AuthState>,
    method: Method,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if let Some(response) = gate(&state, &method, &Method::POST) {
        return response;
    }
    // `limitHandler(limiterOpts.OAuthClientRegister)`.
    if let Err(error) = limit_by_header(
        &state.oauth_register_limiter,
        &state.rate_limit_header,
        &headers,
    ) {
        return error.into_response();
    }
    // `OAuthServerClientDynamicRegister`.
    if !state.oauth_dynamic_registration {
        return AuthError::new(
            403,
            "oauth_dynamic_client_registration_disabled",
            "Dynamic client registration is not enabled",
        )
        .into_response();
    }
    let body = match read_body(body).await {
        Ok(body) => body,
        Err(error) => return error.into_response(),
    };
    // `json.NewDecoder(r.Body).Decode` reads the first JSON value only.
    let params = match serde_json::Deserializer::from_slice(&body)
        .into_iter::<OAuthRegister>()
        .next()
    {
        Some(Ok(params)) => params,
        _ => return AuthError::new(400, "bad_json", "Invalid JSON body").into_response(),
    };
    match register_oauth_client(&state, &params, "dynamic").await {
        Ok(client) => json_status(StatusCode::CREATED, &client),
        Err(message) => AuthError::validation(400, message).into_response(),
    }
}

/// The token endpoint: `requireOAuthClientAuth` and the authorization-code /
/// refresh-token grants are not ported.
async fn token(State(state): State<AuthState>, method: Method) -> Response {
    if let Some(response) = gate(&state, &method, &Method::POST) {
        return response;
    }
    MegabaseNotImplemented::new(COMPONENT, UNIT_TOKEN).into_response()
}

// megabase:unit auth:route:GET /auth/v1/oauth/userinfo
async fn userinfo(State(state): State<AuthState>, method: Method, headers: HeaderMap) -> Response {
    if let Some(response) = gate(&state, &method, &Method::GET) {
        return response;
    }
    // `requireAuthentication`.
    let authed = match require_user(&state, &headers).await {
        Ok(authed) => authed,
        Err(response) => return *response,
    };
    // `OAuthUserInfo`: no session in context means only `sub`.
    let scopes = match authed.session_id {
        None => None,
        Some(session_id) => match state.backend.session_scopes(session_id).await {
            Ok(scopes) => Some(scopes.unwrap_or_default()),
            Err(error) => {
                tracing::error!(%error, "session scope lookup failed");
                return AuthError::internal("Database error finding user").into_response();
            }
        },
    };
    let claims = userinfo_claims(&authed.user, scopes.as_deref());
    match go_marshal(&claims) {
        Ok(text) => (
            StatusCode::OK,
            [(CONTENT_TYPE, HeaderValue::from_static("application/json"))],
            text,
        )
            .into_response(),
        Err(_) => AuthError::internal("Error encoding json response").into_response(),
    }
}

/// `OAuthServerAuthorize` is not ported.
async fn authorize(State(state): State<AuthState>, method: Method) -> Response {
    if let Some(response) = gate(&state, &method, &Method::GET) {
        return response;
    }
    MegabaseNotImplemented::new(COMPONENT, UNIT_AUTHORIZE).into_response()
}

/// `OAuthServerGetAuthorization` is not ported.
async fn get_authorization(State(state): State<AuthState>, method: Method) -> Response {
    if let Some(response) = gate(&state, &method, &Method::GET) {
        return response;
    }
    MegabaseNotImplemented::new(COMPONENT, UNIT_GET_AUTHORIZATION).into_response()
}

/// `OAuthServerConsent` is not ported.
async fn consent(State(state): State<AuthState>, method: Method) -> Response {
    if let Some(response) = gate(&state, &method, &Method::POST) {
        return response;
    }
    MegabaseNotImplemented::new(COMPONENT, UNIT_CONSENT).into_response()
}

/// `models.ParseScopeString`: space-separated, blanks dropped.
fn parse_scopes(scopes: &str) -> Vec<&str> {
    scopes
        .trim()
        .split(' ')
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .collect()
}

/// `OAuthUserInfo` claims. `scopes` is `None` without a session, and the
/// session's `scopes` column (empty when NULL) otherwise. A Go map marshals
/// with sorted keys, as `serde_json::Map` does.
fn userinfo_claims(user: &UserRecord, scopes: Option<&str>) -> Value {
    let mut info = Map::new();
    info.insert("sub".into(), json!(user.id.to_string()));
    let Some(scopes) = scopes else {
        return Value::Object(info);
    };
    let scopes = parse_scopes(scopes);
    let has = |scope: &str| scopes.contains(&scope);
    let metadata_str = |key: &str| {
        user.user_metadata
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
    };

    if has("email") {
        if !user.email.is_empty() {
            info.insert("email".into(), json!(user.email));
        }
        if user.email_confirmed_at.is_some() {
            info.insert("email_verified".into(), json!(true));
        }
    }

    if has("profile") {
        if let Some(name) = metadata_str("name") {
            info.insert("name".into(), json!(name));
        } else if !user.email.is_empty() {
            info.insert("name".into(), json!(user.email));
        }
        if let Some(picture) = metadata_str("picture").or_else(|| metadata_str("avatar_url")) {
            info.insert("picture".into(), json!(picture));
        }
        if let Some(username) =
            metadata_str("preferred_username").or_else(|| metadata_str("username"))
        {
            info.insert("preferred_username".into(), json!(username));
        }
        if let Ok(since) = user.updated_at.duration_since(UNIX_EPOCH) {
            if since.as_secs() > 0 {
                info.insert("updated_at".into(), json!(since.as_secs()));
            }
        }
        if !user.user_metadata.is_null() {
            info.insert("user_metadata".into(), user.user_metadata.clone());
        }
    }

    if has("phone") {
        if !user.phone.is_empty() {
            info.insert("phone".into(), json!(user.phone));
        }
        if user.phone_confirmed_at.is_some() {
            info.insert("phone_verified".into(), json!(true));
        }
    }

    Value::Object(info)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::body::to_bytes;
    use axum::http::Request;
    use megabase_core::Hs256;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::config::AuthConfig;
    use crate::store::Backend;

    const SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";

    fn app(configure: impl FnOnce(&mut AuthState)) -> Router {
        let jwt = Hs256::new(SECRET.as_bytes()).unwrap();
        let mut state = AuthState::new(
            AuthConfig::reference_defaults(),
            Some(jwt),
            Backend::memory(),
        );
        configure(&mut state);
        crate::router_with_state(state)
    }

    async fn call(
        app: Router,
        method: &str,
        path: &str,
        body: &str,
        headers: &[(&str, &str)],
    ) -> (StatusCode, HeaderMap, Value) {
        let mut builder = Request::builder().method(method).uri(path);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let request = builder.body(Body::from(body.to_string())).unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, headers, json)
    }

    const ROUTES: &[(&str, &str)] = &[
        ("POST", "/auth/v1/oauth/clients/register"),
        ("POST", "/auth/v1/oauth/token"),
        ("GET", "/auth/v1/oauth/userinfo"),
        ("GET", "/auth/v1/oauth/authorize"),
        (
            "GET",
            "/auth/v1/oauth/authorizations/2b8d6f0e-8f4d-4c1a-9a43-1d1f3c1f0a11",
        ),
        (
            "POST",
            "/auth/v1/oauth/authorizations/2b8d6f0e-8f4d-4c1a-9a43-1d1f3c1f0a11/consent",
        ),
    ];

    #[tokio::test]
    async fn every_method_is_feature_disabled_while_the_server_is_off() {
        for (_, path) in ROUTES {
            for method in ["GET", "POST", "PUT", "DELETE"] {
                let (status, headers, body) = call(app(|_| {}), method, path, "", &[]).await;
                assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
                assert_eq!(body["error_code"], "feature_disabled");
                assert_eq!(body["msg"], "OAuth server is disabled");
                assert_eq!(body["code"], 404);
                assert_eq!(headers["x-sb-error-code"], "feature_disabled");
            }
        }
    }

    #[tokio::test]
    async fn wrong_methods_are_405_once_enabled() {
        for (method, path) in ROUTES {
            let wrong = if *method == "GET" { "POST" } else { "GET" };
            let (status, headers, _) =
                call(app(|s| s.oauth_server_enabled = true), wrong, path, "", &[]).await;
            assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{wrong} {path}");
            assert_eq!(headers[ALLOW], *method);
        }
    }

    #[tokio::test]
    async fn unported_flows_are_501_once_enabled() {
        for (method, path, unit) in [
            ("POST", ROUTES[1].1, UNIT_TOKEN),
            ("GET", ROUTES[3].1, UNIT_AUTHORIZE),
            ("GET", ROUTES[4].1, UNIT_GET_AUTHORIZATION),
            ("POST", ROUTES[5].1, UNIT_CONSENT),
        ] {
            let (status, _, body) = call(
                app(|s| s.oauth_server_enabled = true),
                method,
                path,
                "",
                &[],
            )
            .await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{path}");
            assert_eq!(body["unit"], unit);
        }
    }

    #[tokio::test]
    async fn register_needs_dynamic_registration_then_json() {
        let path = ROUTES[0].1;
        let (status, _, body) = call(
            app(|s| s.oauth_server_enabled = true),
            "POST",
            path,
            "{}",
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            body["error_code"],
            "oauth_dynamic_client_registration_disabled"
        );
        assert_eq!(body["msg"], "Dynamic client registration is not enabled");

        let enabled = |s: &mut AuthState| {
            s.oauth_server_enabled = true;
            s.oauth_dynamic_registration = true;
        };
        let (status, _, body) = call(app(enabled), "POST", path, "not json", &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "bad_json");
        assert_eq!(body["msg"], "Invalid JSON body");

        let (status, _, body) = call(app(enabled), "POST", path, "{}", &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "validation_failed");
        assert_eq!(body["msg"], "400: redirect_uris is required");
    }

    #[tokio::test]
    async fn register_is_rate_limited_per_header_value() {
        let path = ROUTES[0].1;
        let router = app(|s| {
            s.oauth_server_enabled = true;
            s.oauth_register_limiter =
                std::sync::Arc::new(crate::ratelimit::KeyedLimiter::new(0.0, 1.0));
        });
        let header = [("x-megabase-judge-rate-key", "203.0.113.9")];
        let (status, _, _) = call(router.clone(), "POST", path, "{}", &header).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _, body) = call(router, "POST", path, "{}", &header).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body["error_code"], "over_request_rate_limit");
    }

    #[tokio::test]
    async fn userinfo_requires_a_bearer_token() {
        let (status, _, body) = call(
            app(|s| s.oauth_server_enabled = true),
            "GET",
            ROUTES[2].1,
            "",
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error_code"], "no_authorization");
    }

    #[tokio::test]
    async fn userinfo_without_scopes_is_only_sub() {
        let router = app(|s| s.oauth_server_enabled = true);
        let (status, _, created) = call(
            router.clone(),
            "POST",
            "/auth/v1/signup",
            r#"{"email":"info@example.com","password":"secret-pass"}"#,
            &[("content-type", "application/json")],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        let auth = format!("Bearer {}", created["access_token"].as_str().unwrap());
        let (status, _, body) =
            call(router, "GET", ROUTES[2].1, "", &[("authorization", &auth)]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body, json!({ "sub": created["user"]["id"] }));
    }

    fn user() -> UserRecord {
        let at = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        UserRecord {
            id: Uuid::nil(),
            aud: "authenticated".into(),
            role: "authenticated".into(),
            email: "ada@example.com".into(),
            phone: "15550001111".into(),
            email_confirmed_at: Some(at),
            last_sign_in_at: None,
            app_metadata: json!({}),
            user_metadata: json!({ "avatar_url": "https://x/a.png", "username": "ada" }),
            identities: Vec::new(),
            created_at: at,
            updated_at: at,
            is_anonymous: false,
            banned_until: None,
            is_sso_user: false,
            password_hash: String::new(),
            phone_confirmed_at: None,
            confirmed_at: Some(at),
        }
    }

    #[test]
    fn claims_follow_the_session_scopes() {
        let user = user();
        let sub = Uuid::nil().to_string();
        assert_eq!(userinfo_claims(&user, None), json!({ "sub": sub }));
        assert_eq!(userinfo_claims(&user, Some("")), json!({ "sub": sub }));
        assert_eq!(
            userinfo_claims(&user, Some(" openid  email phone ")),
            json!({
                "sub": sub,
                "email": "ada@example.com",
                "email_verified": true,
                "phone": "15550001111",
            })
        );
        assert_eq!(
            userinfo_claims(&user, Some("profile")),
            json!({
                "sub": sub,
                "name": "ada@example.com",
                "picture": "https://x/a.png",
                "preferred_username": "ada",
                "updated_at": 1_700_000_000,
                "user_metadata": { "avatar_url": "https://x/a.png", "username": "ada" },
            })
        );
    }

    #[test]
    fn profile_prefers_name_picture_and_preferred_username() {
        let mut user = user();
        user.user_metadata = json!({
            "name": "Ada",
            "picture": "https://x/p.png",
            "avatar_url": "https://x/a.png",
            "preferred_username": "lovelace",
            "username": "ada",
        });
        user.updated_at = UNIX_EPOCH;
        let claims = userinfo_claims(&user, Some("profile"));
        assert_eq!(claims["name"], "Ada");
        assert_eq!(claims["picture"], "https://x/p.png");
        assert_eq!(claims["preferred_username"], "lovelace");
        assert!(claims.get("updated_at").is_none());
    }
}
