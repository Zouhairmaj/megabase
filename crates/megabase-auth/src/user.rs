// Ported from supabase/auth internal/api/user.go, internal/api/identity.go,
// internal/api/oauthserver/handlers.go, internal/api/middleware.go,
// internal/api/auth.go, internal/api/external.go, internal/api/helpers.go,
// and internal/api/phone.go (MIT), pin v2.197.0.

//! Authenticated user routes.
//!
//! `GET` and `PUT /user` follow GoTrue for the reference-stack defaults.
//! An email change, a phone change while SMS autoconfirm is off, a password
//! change while reauthentication or the current password is required, and an
//! OAuth redirect for an enabled provider return 501. Identity linking and
//! the OAuth grant routes stay the upstream 404 while their flags are off.

use std::sync::OnceLock;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use megabase_core::{bearer_token, JwtClaims, MegabaseNotImplemented};
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::config::{AuthConfig, ExternalProviders};
use crate::error::{bad_json, unexpected, validation, GoTrueError};
use crate::jsonutil::{format_ts, user_json};
use crate::routes::{
    banned, jwt_failure, query_param, read_body, session_claim, valid_channel, validate_email,
    JsonOk, INVALID_CHANNEL,
};
use crate::state::AuthState;
use crate::store::{
    hash_new_password, password_matches, OAuthGrantView, RevokeGrant, StoreError, UnlinkError,
    UserRecord, UserUpdate, UserUpdateError,
};
use crate::COMPONENT;

const UNIT_PUT: &str = "auth:route:PUT /auth/v1/user";
const UNIT_AUTHORIZE: &str = "auth:route:GET /auth/v1/user/identities/authorize";

const AUDIENCE: &str = "Token audience doesn't match request audience";

struct Authed {
    claims: JwtClaims,
    user: UserRecord,
    session_id: Option<Uuid>,
}

// megabase:unit auth:route:GET /auth/v1/user
pub(crate) async fn user_get(State(state): State<AuthState>, headers: HeaderMap) -> Response {
    let authed = match require_user(&state, &headers).await {
        Ok(authed) => authed,
        Err(response) => return *response,
    };
    if !audiences_match(&headers, &authed.claims, &state) {
        return validation(AUDIENCE).into_response();
    }
    JsonOk(user_json(&authed.user)).into_response()
}

// megabase:unit auth:route:PUT /auth/v1/user
pub(crate) async fn user_update(
    State(state): State<AuthState>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let authed = match require_user(&state, &headers).await {
        Ok(authed) => authed,
        Err(response) => return *response,
    };
    let bytes = match read_body(body).await {
        Ok(bytes) => bytes,
        Err(error) => return error.into_response(),
    };
    let params: UpdateBody = match serde_json::from_slice(&bytes) {
        Ok(params) => params,
        Err(error) => return bad_json(error).into_response(),
    };
    let email = if params.email.is_empty() {
        String::new()
    } else {
        match validate_email(&params.email) {
            Ok(email) => email,
            Err(error) => return error.into_response(),
        }
    };
    let phone = if params.phone.is_empty() {
        None
    } else {
        match validate_phone(&params.phone) {
            Ok(phone) => Some(phone),
            Err(error) => return error.into_response(),
        }
    };
    if let Some(password) = params.password.as_deref() {
        if let Err(error) = check_password(&state.config, password) {
            return error.into_response();
        }
    }
    if phone.is_some() {
        let channel = if params.channel.is_empty() {
            "sms"
        } else {
            params.channel.as_str()
        };
        if !valid_channel(channel, &state.config.sms_provider) {
            return validation(INVALID_CHANNEL).into_response();
        }
    }
    if params.app_metadata.is_some() && !is_admin(&authed.user, &state) {
        return GoTrueError::new(
            StatusCode::FORBIDDEN,
            "not_admin",
            "Updating app_metadata requires admin privileges",
        )
        .into_response();
    }
    let password_set = params
        .password
        .as_ref()
        .is_some_and(|value| !value.is_empty());
    let email_change = !email.is_empty() && email != authed.user.email;
    let phone_change = phone
        .as_ref()
        .is_some_and(|value| value != &authed.user.phone);
    if password_set || email_change || phone_change {
        match state
            .backend
            .mfa_blocks_sensitive_update(authed.user.id, authed.session_id)
            .await
        {
            Ok(true) => {
                return GoTrueError::new(
                    StatusCode::UNAUTHORIZED,
                    "insufficient_aal",
                    "AAL2 session is required to update email or password when MFA is enabled.",
                )
                .into_response();
            }
            Ok(false) => {}
            Err(error) => return update_store_error(&error),
        }
    }
    if authed.user.is_anonymous && password_set && email.is_empty() && params.phone.is_empty() {
        return GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "Updating password of an anonymous user without an email or phone is not allowed",
        )
        .into_response();
    }
    if authed.user.is_sso_user
        && (password_set || email_change || phone_change || !params.nonce.is_empty())
    {
        return GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "user_sso_managed",
            "Updating email, phone, password of a SSO account only possible via SSO",
        )
        .into_response();
    }
    if email_change {
        match state
            .backend
            .email_owned_by_other(
                &email,
                &request_audience(&headers, &authed.claims, &state),
                authed.user.id,
            )
            .await
        {
            Ok(true) => {
                return GoTrueError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "email_exists",
                    "A user with this email address has already been registered",
                )
                .into_response();
            }
            Ok(false) => {}
            Err(error) => {
                tracing::error!(%error, "email duplicate check failed");
                return unexpected("Database error checking email").into_response();
            }
        }
    }
    if let Some(phone) = phone.as_deref().filter(|_| phone_change) {
        match state
            .backend
            .phone_taken(phone, &request_audience(&headers, &authed.claims, &state))
            .await
        {
            Ok(true) => {
                return GoTrueError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "phone_exists",
                    "A user with this phone number has already been registered",
                )
                .into_response();
            }
            Ok(false) => {}
            Err(error) => {
                tracing::error!(%error, "phone duplicate check failed");
                return unexpected("Database error checking phone").into_response();
            }
        }
    }
    if params.password.is_some()
        && (state.password_require_reauthentication || state.password_require_current)
    {
        return MegabaseNotImplemented::new(COMPONENT, UNIT_PUT).into_response();
    }
    if email_change {
        return MegabaseNotImplemented::new(COMPONENT, UNIT_PUT).into_response();
    }
    if phone_change && !state.config.phone_autoconfirm {
        return MegabaseNotImplemented::new(COMPONENT, UNIT_PUT).into_response();
    }
    let mut password_hash = None;
    if let Some(password) = params.password.as_deref().filter(|value| !value.is_empty()) {
        if !authed.user.password_hash.is_empty() {
            match password_matches(password, &authed.user.password_hash).await {
                Ok(true) => {
                    return GoTrueError::new(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "same_password",
                        "New password should be different from the old password.",
                    )
                    .into_response();
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::error!(%error, "password compare failed");
                    return unexpected("Error during password storage").into_response();
                }
            }
        }
        match hash_new_password(password.to_string()).await {
            Ok(hash) => password_hash = Some(hash),
            Err(error) => {
                tracing::error!(%error, "password hash failed");
                return unexpected("Error during password storage").into_response();
            }
        }
    }
    let updated = state
        .backend
        .update_user(UserUpdate {
            user_id: authed.user.id,
            session_id: authed.session_id,
            data: params.data,
            app_data: params.app_metadata,
            password_hash,
            phone: phone_change.then_some(phone).flatten(),
        })
        .await;
    match updated {
        Ok(user) => JsonOk(user_json(&user)).into_response(),
        Err(UserUpdateError::Missing) => GoTrueError::new(
            StatusCode::FORBIDDEN,
            "user_not_found",
            "User from sub claim in JWT does not exist",
        )
        .into_response(),
        Err(UserUpdateError::Store(error)) => {
            tracing::error!(%error, "user update failed");
            unexpected("Error updating user").into_response()
        }
    }
}

// megabase:unit auth:route:GET /auth/v1/user/identities/authorize
pub(crate) async fn link_identity(
    State(state): State<AuthState>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    if let Err(response) = require_user(&state, &headers).await {
        return *response;
    }
    if !state.manual_linking_enabled {
        return feature_off("manual_linking_disabled", "Manual linking is disabled");
    }
    let provider = query_param(uri.query(), "provider").unwrap_or_default();
    match provider_decision(&state, &provider) {
        ProviderDecision::Reject(detail) => {
            validation(format!("Unsupported provider: {detail}")).into_response()
        }
        ProviderDecision::RedirectNotPorted => {
            MegabaseNotImplemented::new(COMPONENT, UNIT_AUTHORIZE).into_response()
        }
    }
}

// megabase:unit auth:route:DELETE /auth/v1/user/identities/{identity_id}
pub(crate) async fn delete_identity(
    State(state): State<AuthState>,
    Path(identity_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let authed = match require_user(&state, &headers).await {
        Ok(authed) => authed,
        Err(response) => return *response,
    };
    if !state.manual_linking_enabled {
        return feature_off("manual_linking_disabled", "Manual linking is disabled");
    }
    let Ok(identity_id) = Uuid::parse_str(&identity_id) else {
        return GoTrueError::new(
            StatusCode::NOT_FOUND,
            "validation_failed",
            "identity_id must be an UUID",
        )
        .into_response();
    };
    if !audiences_match(&headers, &authed.claims, &state) {
        return GoTrueError::new(StatusCode::FORBIDDEN, "unexpected_audience", AUDIENCE)
            .into_response();
    }
    if authed.user.identities.len() <= 1 {
        return GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "single_identity_not_deletable",
            "User must have at least 1 identity after unlinking",
        )
        .into_response();
    }
    if !authed
        .user
        .identities
        .iter()
        .any(|identity| identity.id == identity_id)
    {
        return GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "identity_not_found",
            "Identity doesn't exist",
        )
        .into_response();
    }
    match state
        .backend
        .unlink_identity(authed.user.id, identity_id, state.config.mailer_autoconfirm)
        .await
    {
        Ok(_) => JsonOk(json!({})).into_response(),
        Err(UnlinkError::EmailConflict) => GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "email_conflict_identity_not_deletable",
            "Unable to unlink identity due to email conflict",
        )
        .into_response(),
        Err(UnlinkError::NotFound) => GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "identity_not_found",
            "Identity doesn't exist",
        )
        .into_response(),
        Err(UnlinkError::Single) => GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "single_identity_not_deletable",
            "User must have at least 1 identity after unlinking",
        )
        .into_response(),
        Err(UnlinkError::Missing) => GoTrueError::new(
            StatusCode::FORBIDDEN,
            "user_not_found",
            "User from sub claim in JWT does not exist",
        )
        .into_response(),
        Err(UnlinkError::Store(error)) => {
            tracing::error!(%error, "identity unlink failed");
            unexpected("Database error deleting identity").into_response()
        }
    }
}

// megabase:unit auth:route:GET /auth/v1/user/oauth/grants
pub(crate) async fn list_grants(State(state): State<AuthState>, headers: HeaderMap) -> Response {
    let authed = match require_user(&state, &headers).await {
        Ok(authed) => authed,
        Err(response) => return *response,
    };
    if !state.oauth_server_enabled {
        return feature_off("feature_disabled", "OAuth server is disabled");
    }
    match state.backend.list_oauth_grants(authed.user.id).await {
        Ok(grants) => {
            let body = Value::Array(grants.iter().map(grant_json).collect());
            JsonOk(body).into_response()
        }
        Err(error) => {
            tracing::error!(%error, "oauth grant list failed");
            unexpected("Error fetching OAuth grants").into_response()
        }
    }
}

// megabase:unit auth:route:DELETE /auth/v1/user/oauth/grants
pub(crate) async fn revoke_grant(
    State(state): State<AuthState>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let authed = match require_user(&state, &headers).await {
        Ok(authed) => authed,
        Err(response) => return *response,
    };
    if !state.oauth_server_enabled {
        return feature_off("feature_disabled", "OAuth server is disabled");
    }
    let Some(client_id) = query_param(uri.query(), "client_id").filter(|value| !value.is_empty())
    else {
        return validation("client_id query parameter is required").into_response();
    };
    let Ok(client_id) = Uuid::parse_str(&client_id) else {
        return validation("invalid client_id format").into_response();
    };
    match state
        .backend
        .revoke_oauth_grant(authed.user.id, client_id)
        .await
    {
        Ok(RevokeGrant::Revoked) => StatusCode::NO_CONTENT.into_response(),
        Ok(RevokeGrant::Missing) => GoTrueError::new(
            StatusCode::NOT_FOUND,
            "oauth_consent_not_found",
            "No active grant found for this client",
        )
        .into_response(),
        Err(error) => {
            tracing::error!(%error, "oauth grant revoke failed");
            unexpected("Error revoking grant").into_response()
        }
    }
}

#[derive(Deserialize)]
struct UpdateBody {
    #[serde(default)]
    email: String,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    nonce: String,
    #[serde(default)]
    data: Option<Map<String, Value>>,
    #[serde(default)]
    app_metadata: Option<Map<String, Value>>,
    #[serde(default)]
    phone: String,
    #[serde(default)]
    channel: String,
}

fn reject(response: Response) -> Result<Authed, Box<Response>> {
    Err(Box::new(response))
}

async fn require_user(state: &AuthState, headers: &HeaderMap) -> Result<Authed, Box<Response>> {
    let header = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let Some(token) = bearer_token(header) else {
        return reject(
            GoTrueError::new(
                StatusCode::UNAUTHORIZED,
                "no_authorization",
                "This endpoint requires a valid Bearer token",
            )
            .into_response(),
        );
    };
    let Some(jwt) = state.jwt.as_ref() else {
        return reject(unexpected("Server lacks JWT secret").into_response());
    };
    let claims = match jwt.verify(token) {
        Ok(claims) => claims,
        Err(error) => return reject(jwt_failure(&error)),
    };
    let Some(sub) = claims.sub.as_deref() else {
        return reject(
            GoTrueError::new(
                StatusCode::FORBIDDEN,
                "bad_jwt",
                "invalid claim: missing sub claim",
            )
            .into_response(),
        );
    };
    let user_id = match Uuid::parse_str(sub) {
        Ok(id) => id,
        Err(_) => {
            return reject(
                GoTrueError::new(
                    StatusCode::BAD_REQUEST,
                    "bad_jwt",
                    "invalid claim: sub claim must be a UUID",
                )
                .into_response(),
            );
        }
    };
    let session_id = match session_claim(&claims) {
        Ok(id) => id,
        Err(error) => return reject(error.into_response()),
    };
    let user = match state.backend.load_user(user_id).await {
        Ok(Some(user)) => user,
        Ok(None) => {
            return reject(
                GoTrueError::new(
                    StatusCode::FORBIDDEN,
                    "user_not_found",
                    "User from sub claim in JWT does not exist",
                )
                .into_response(),
            );
        }
        Err(error) => {
            tracing::error!(%error, "user lookup failed");
            return reject(unexpected("Database error finding user").into_response());
        }
    };
    if let Some(session_id) = session_id {
        match state.backend.session_exists(session_id).await {
            Ok(true) => {}
            Ok(false) => {
                return reject(
                    GoTrueError::new(
                        StatusCode::FORBIDDEN,
                        "session_not_found",
                        "Session from session_id claim in JWT does not exist",
                    )
                    .into_response(),
                );
            }
            Err(error) => {
                tracing::error!(%error, "session lookup failed");
                return reject(unexpected("Database error finding user").into_response());
            }
        }
    }
    if banned(user.banned_until) {
        return reject(
            GoTrueError::new(StatusCode::FORBIDDEN, "user_banned", "User is banned")
                .into_response(),
        );
    }
    Ok(Authed {
        claims,
        user,
        session_id,
    })
}

fn audiences_match(headers: &HeaderMap, claims: &JwtClaims, state: &AuthState) -> bool {
    claim_audience(claims).is_some_and(|aud| aud == request_audience(headers, claims, state))
}

fn claim_audience(claims: &JwtClaims) -> Option<String> {
    match claims.raw.get("aud")? {
        Value::String(value) if !value.is_empty() => Some(value.clone()),
        Value::Array(values) => values
            .first()
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

/// GoTrue `requestAud`: header, then the token audience unless the role is
/// an API admin role, then the configured audience.
fn request_audience(headers: &HeaderMap, claims: &JwtClaims, state: &AuthState) -> String {
    if let Some(aud) = headers
        .get("x-jwt-aud")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
    {
        return aud.to_string();
    }
    let role = claims.role.as_deref().unwrap_or("");
    if !state.admin_roles.iter().any(|allowed| allowed == role) {
        if let Some(aud) = claim_audience(claims) {
            return aud;
        }
    }
    state.config.jwt_aud.clone()
}

fn is_admin(user: &UserRecord, state: &AuthState) -> bool {
    user.aud == state.config.jwt_aud && user.role == state.admin_group_name
}

fn check_password(config: &AuthConfig, password: &str) -> Result<(), GoTrueError> {
    if password.len() > 72 {
        return Err(validation("Password cannot be longer than 72 characters"));
    }
    if password.len() < config.password_min_length {
        return Err(GoTrueError::weak_password(
            format!(
                "Password should be at least {} characters.",
                config.password_min_length
            ),
            vec!["length"],
        ));
    }
    Ok(())
}

fn validate_phone(phone: &str) -> Result<String, GoTrueError> {
    let phone = phone.strip_prefix('+').unwrap_or(phone).replace(' ', "");
    if !phone_format().is_match(&phone) {
        return Err(validation("Invalid phone number format (E.164 required)"));
    }
    Ok(phone)
}

fn phone_format() -> &'static Regex {
    static PHONE: OnceLock<Regex> = OnceLock::new();
    PHONE.get_or_init(|| Regex::new(r"^[1-9][0-9]{1,14}$").expect("phone format regex"))
}

fn feature_off(error_code: &'static str, message: &'static str) -> Response {
    GoTrueError::new(StatusCode::NOT_FOUND, error_code, message).into_response()
}

fn update_store_error(error: &StoreError) -> Response {
    tracing::error!(%error, "user update lookup failed");
    unexpected("Error updating user").into_response()
}

enum ProviderDecision {
    Reject(String),
    RedirectNotPorted,
}

fn provider_decision(state: &AuthState, name: &str) -> ProviderDecision {
    let name = name.to_ascii_lowercase();
    if name.starts_with("custom:") {
        if !state.custom_oauth_enabled {
            return ProviderDecision::Reject("custom OAuth providers are disabled".into());
        }
        return ProviderDecision::RedirectNotPorted;
    }
    match known_provider(&state.config.external, &name) {
        None => ProviderDecision::Reject(format!("Provider {name} could not be found")),
        Some(false) => ProviderDecision::Reject("provider is not enabled".into()),
        Some(true) => ProviderDecision::RedirectNotPorted,
    }
}

fn known_provider(external: &ExternalProviders, name: &str) -> Option<bool> {
    Some(match name {
        "apple" => external.apple,
        "azure" => external.azure,
        "bitbucket" => external.bitbucket,
        "discord" => external.discord,
        "facebook" => external.facebook,
        "figma" => external.figma,
        "fly" => external.fly,
        "github" => external.github,
        "gitlab" => external.gitlab,
        "google" => external.google,
        "kakao" => external.kakao,
        "keycloak" => external.keycloak,
        "linkedin" => external.linkedin,
        "linkedin_oidc" => external.linkedin_oidc,
        "notion" => external.notion,
        "snapchat" => external.snapchat,
        "spotify" => external.spotify,
        "slack" => external.slack,
        "slack_oidc" => external.slack_oidc,
        "twitch" => external.twitch,
        "twitter" => external.twitter,
        "workos" => external.workos,
        "zoom" => external.zoom,
        "x" | "vercel_marketplace" => false,
        _ => return None,
    })
}

fn grant_json(grant: &OAuthGrantView) -> Value {
    let mut client = Map::new();
    client.insert("id".into(), json!(grant.client_id.to_string()));
    if !grant.name.is_empty() {
        client.insert("name".into(), json!(grant.name));
    }
    if !grant.uri.is_empty() {
        client.insert("uri".into(), json!(grant.uri));
    }
    if !grant.logo_uri.is_empty() {
        client.insert("logo_uri".into(), json!(grant.logo_uri));
    }
    json!({
        "client": client,
        "scopes": grant.scopes,
        "granted_at": format_ts(grant.granted_at),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;
    use crate::store::Backend;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use axum::Router;
    use megabase_core::Hs256;
    use tower::ServiceExt;

    const SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";

    fn app(configure: impl FnOnce(&mut AuthState)) -> (Router, Backend) {
        let backend = Backend::memory();
        let jwt = Hs256::new(SECRET.as_bytes()).unwrap();
        let mut state =
            AuthState::new(AuthConfig::reference_defaults(), Some(jwt), backend.clone());
        configure(&mut state);
        (crate::router_with_state(state), backend)
    }

    async fn call(
        app: Router,
        method: &str,
        path: &str,
        body: Option<&str>,
        headers: &[(&str, &str)],
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(path);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let request = builder
            .body(Body::from(body.unwrap_or("").to_string()))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, json)
    }

    async fn signup(app: &Router, email: &str) -> Value {
        let body = format!(
            r#"{{"email":"{email}","password":"secret-pass","data":{{"name":"Ada","drop":"me"}}}}"#
        );
        let (status, body) = call(app.clone(), "POST", "/auth/v1/signup", Some(&body), &[]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    fn bearer(token: &str) -> String {
        format!("Bearer {token}")
    }

    #[tokio::test]
    async fn get_user_returns_the_reloaded_row() {
        let (app, backend) = app(|_| {});
        let created = signup(&app, "get-user@example.com").await;
        let token = created["access_token"].as_str().unwrap();
        let auth = bearer(token);
        let (status, body) = call(
            app.clone(),
            "GET",
            "/auth/v1/user",
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["id"], created["user"]["id"]);
        assert_eq!(body["email"], "get-user@example.com");
        assert_eq!(body["user_metadata"]["email_verified"], true);
        assert_eq!(
            body["identities"][0]["identity_data"]["email_verified"],
            false
        );
        assert!(body["confirmed_at"].is_string());
        assert!(body["email_confirmed_at"].is_string());
        assert!(body.get("factors").is_none());

        let (status, body) = call(app.clone(), "GET", "/auth/v1/user", None, &[]).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error_code"], "no_authorization");

        let (status, body) = call(
            app.clone(),
            "GET",
            "/auth/v1/user",
            None,
            &[("authorization", &auth), ("x-jwt-aud", "other")],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["msg"], AUDIENCE);

        backend.ban_for_test("get-user@example.com").await;
        let (status, body) = call(
            app,
            "GET",
            "/auth/v1/user",
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error_code"], "user_banned");
    }

    #[tokio::test]
    async fn put_updates_metadata_password_and_phone() {
        let (app, _) = app(|_| {});
        let created = signup(&app, "put-user@example.com").await;
        let refresh = created["refresh_token"].as_str().unwrap().to_string();
        let (status, login) = call(
            app.clone(),
            "POST",
            "/auth/v1/token?grant_type=password",
            Some(r#"{"email":"put-user@example.com","password":"secret-pass"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{login}");
        let second = login["access_token"].as_str().unwrap().to_string();
        let second_auth = bearer(&second);

        let (status, body) = call(
            app.clone(),
            "PUT",
            "/auth/v1/user",
            Some(r#"{"data":{"name":"Grace","drop":null}}"#),
            &[("authorization", &second_auth)],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["user_metadata"]["name"], "Grace");
        assert!(body["user_metadata"].get("drop").is_none());
        assert!(body["confirmed_at"].is_string());

        let (status, body) = call(
            app.clone(),
            "PUT",
            "/auth/v1/user",
            Some(r#"{"password":"secret-pass"}"#),
            &[("authorization", &second_auth)],
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error_code"], "same_password");

        let (status, body) = call(
            app.clone(),
            "PUT",
            "/auth/v1/user",
            Some(r#"{"password":"next-secret","phone":"+1 415 555 0100"}"#),
            &[("authorization", &second_auth)],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["phone"], "14155550100");
        assert!(body["phone_confirmed_at"].is_string());
        assert!(body["identities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|identity| identity["provider"] == "phone"));
        assert_eq!(body["app_metadata"]["provider"], "email");

        let (status, body) = call(
            app.clone(),
            "GET",
            "/auth/v1/user",
            None,
            &[("authorization", &second_auth)],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, body) = call(
            app.clone(),
            "POST",
            "/auth/v1/token?grant_type=refresh_token",
            Some(&format!(r#"{{"refresh_token":"{refresh}"}}"#)),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error_code"], "refresh_token_not_found");
        let (status, body) = call(
            app,
            "POST",
            "/auth/v1/token?grant_type=password",
            Some(r#"{"email":"put-user@example.com","password":"next-secret"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    #[tokio::test]
    async fn put_rejects_conflicts_and_leaves_email_unported() {
        let (app, backend) = app(|_| {});
        let first = signup(&app, "first@example.com").await;
        let _ = signup(&app, "second@example.com").await;
        let sso_created = signup(&app, "sso@example.com").await;
        let anon_created = signup(&app, "anon@example.com").await;
        let auth = bearer(first["access_token"].as_str().unwrap());
        let sso_auth = bearer(sso_created["access_token"].as_str().unwrap());
        let anon_auth = bearer(anon_created["access_token"].as_str().unwrap());

        let (status, body) = call(
            app.clone(),
            "PUT",
            "/auth/v1/user",
            Some(r#"{"app_metadata":{}}"#),
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error_code"], "not_admin");

        let (status, body) = call(
            app.clone(),
            "PUT",
            "/auth/v1/user",
            Some(r#"{"email":"second@example.com"}"#),
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error_code"], "email_exists");

        let (status, body) = call(
            app.clone(),
            "PUT",
            "/auth/v1/user",
            Some(r#"{"email":"fresh@example.com"}"#),
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], UNIT_PUT);
        let (status, body) = call(
            app.clone(),
            "GET",
            "/auth/v1/user",
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(body["email"], "first@example.com");
        assert_eq!(status, StatusCode::OK);

        let (status, body) = call(
            app.clone(),
            "PUT",
            "/auth/v1/user",
            Some(r#"{"phone":"0123"}"#),
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["msg"], "Invalid phone number format (E.164 required)");

        let (status, body) = call(
            app.clone(),
            "PUT",
            "/auth/v1/user",
            Some(r#"{"password":"short"}"#),
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error_code"], "weak_password");

        backend.set_sso_for_test("sso@example.com").await;
        let (status, body) = call(
            app.clone(),
            "PUT",
            "/auth/v1/user",
            Some(r#"{"password":"another-secret"}"#),
            &[("authorization", &sso_auth)],
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error_code"], "user_sso_managed");

        backend.set_anonymous_for_test("anon@example.com").await;
        let (status, body) = call(
            app,
            "PUT",
            "/auth/v1/user",
            Some(r#"{"password":"another-secret"}"#),
            &[("authorization", &anon_auth)],
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            body["msg"],
            "Updating password of an anonymous user without an email or phone is not allowed"
        );
    }

    #[tokio::test]
    async fn identity_routes_follow_the_linking_flag() {
        let (off, _) = app(|_| {});
        let created = signup(&off, "link@example.com").await;
        let auth = bearer(created["access_token"].as_str().unwrap());
        let (status, body) = call(
            off.clone(),
            "DELETE",
            "/auth/v1/user/identities/not-a-uuid",
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error_code"], "manual_linking_disabled");

        let (router, backend) = app(|state| state.manual_linking_enabled = true);
        let created = signup(&router, "unlink@example.com").await;
        let auth = bearer(created["access_token"].as_str().unwrap());
        let (status, body) = call(
            router.clone(),
            "DELETE",
            "/auth/v1/user/identities/not-a-uuid",
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["msg"], "identity_id must be an UUID");

        let sole = created["user"]["identities"][0]["identity_id"]
            .as_str()
            .unwrap()
            .to_string();
        let (status, body) = call(
            router.clone(),
            "DELETE",
            &format!("/auth/v1/user/identities/{sole}"),
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error_code"], "single_identity_not_deletable");

        let extra = backend
            .add_identity_for_test("unlink@example.com", "github", "gh@example.com", true)
            .await;
        let (status, body) = call(
            router.clone(),
            "DELETE",
            &format!("/auth/v1/user/identities/{extra}"),
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body, json!({}));
        let (status, body) = call(
            router.clone(),
            "GET",
            "/auth/v1/user",
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["email"], "unlink@example.com");
        assert_eq!(body["identities"].as_array().unwrap().len(), 1);

        let (status, body) = call(
            router.clone(),
            "GET",
            "/auth/v1/user/identities/authorize?provider=nope",
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["msg"],
            "Unsupported provider: Provider nope could not be found"
        );
        let (status, body) = call(
            router.clone(),
            "GET",
            "/auth/v1/user/identities/authorize?provider=GitHub",
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["msg"], "Unsupported provider: provider is not enabled");

        let (enabled, _) = app(|state| {
            state.manual_linking_enabled = true;
            state.config.external.github = true;
        });
        let created = signup(&enabled, "redirect@example.com").await;
        let auth = bearer(created["access_token"].as_str().unwrap());
        let (status, body) = call(
            enabled,
            "GET",
            "/auth/v1/user/identities/authorize?provider=github",
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], UNIT_AUTHORIZE);
    }

    #[tokio::test]
    async fn oauth_grants_follow_the_server_flag() {
        let (off, _) = app(|_| {});
        let created = signup(&off, "grants-off@example.com").await;
        let auth = bearer(created["access_token"].as_str().unwrap());
        let (status, body) = call(
            off,
            "GET",
            "/auth/v1/user/oauth/grants",
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["msg"], "OAuth server is disabled");

        let (app, backend) = app(|state| state.oauth_server_enabled = true);
        let created = signup(&app, "grants@example.com").await;
        let auth = bearer(created["access_token"].as_str().unwrap());
        let (status, body) = call(
            app.clone(),
            "GET",
            "/auth/v1/user/oauth/grants",
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!([]));

        let live = backend
            .insert_oauth_grant_for_test("grants@example.com", "Example", "openid email", false)
            .await;
        backend
            .insert_oauth_grant_for_test("grants@example.com", "Gone", "openid", true)
            .await;
        let (status, body) = call(
            app.clone(),
            "GET",
            "/auth/v1/user/oauth/grants",
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body.as_array().unwrap().len(), 1);
        assert_eq!(body[0]["client"]["name"], "Example");
        assert_eq!(body[0]["scopes"][0], "openid");
        assert_eq!(body[0]["scopes"][1], "email");
        assert!(body[0]["client"].get("logo_uri").is_none());

        let (status, _) = call(
            app.clone(),
            "DELETE",
            "/auth/v1/user/oauth/grants",
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, body) = call(
            app.clone(),
            "DELETE",
            "/auth/v1/user/oauth/grants?client_id=nope",
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["msg"], "invalid client_id format");

        let (status, body) = call(
            app.clone(),
            "DELETE",
            &format!("/auth/v1/user/oauth/grants?client_id={live}"),
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
        let (status, body) = call(
            app.clone(),
            "GET",
            "/auth/v1/user/oauth/grants",
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!([]));
        let (status, body) = call(
            app,
            "DELETE",
            &format!("/auth/v1/user/oauth/grants?client_id={live}"),
            None,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error_code"], "oauth_consent_not_found");
    }
}
