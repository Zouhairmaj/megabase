// Ported from supabase/auth internal/api/api.go, signup.go, logout.go,
// settings.go, password.go, pkce.go, mail.go, and auth.go (MIT), pin v2.197.0.

//! `/auth/v1` routes served by this crate.
//!
//! Health, settings, autoconfirm email signup, logout,
//! `POST /token` (password and refresh-token grants),
//! `GET`/`POST /verify` (signup, invite, recovery, email change), and the
//! user routes are implemented. Invite, recover, resend, reauthenticate,
//! phone signup, anonymous signup, the other verify types, and mailer
//! confirmation stay HTTP 501.

use std::sync::OnceLock;
use std::time::SystemTime;

use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::Router;
use megabase_core::{bearer_token, JwtClaims, JwtError, MegabaseNotImplemented};
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::config::{AuthConfig, AUTH_VERSION};
use crate::error::{
    bad_json, body_too_large, go_quote, unexpected, validation, GoTrueError, MAX_BODY_BYTES,
};
use crate::jsonutil::{access_claims, session_json, unix_secs};
use crate::state::AuthState;
use crate::store::{LogoutScope, SignupCommand, SignupResult, StoreError};
use crate::COMPONENT;

const UNIT_SIGNUP: &str = "auth:route:POST /auth/v1/signup";
const UNIT_REAUTHENTICATE: &str = "auth:route:GET /auth/v1/reauthenticate";
const UNIT_INVITE: &str = "auth:route:POST /auth/v1/invite";
const UNIT_RECOVER: &str = "auth:route:POST /auth/v1/recover";
const UNIT_RESEND: &str = "auth:route:POST /auth/v1/resend";

pub(crate) const INVALID_CHANNEL: &str = "Invalid channel, supported values are 'sms' or 'whatsapp'. 'whatsapp' is only supported if Twilio or Twilio Verify is used as the provider.";

pub(crate) fn router(state: AuthState) -> Router {
    Router::new()
        .route("/auth/v1/health", get(health))
        .route("/auth/v1/settings", get(settings))
        .route("/auth/v1/reauthenticate", get(reauthenticate))
        .route("/auth/v1/signup", post(signup))
        .route("/auth/v1/token", post(crate::token::token))
        .route("/auth/v1/invite", post(invite))
        .route("/auth/v1/logout", post(logout))
        .route("/auth/v1/recover", post(recover))
        .route("/auth/v1/resend", post(resend))
        .route(
            "/auth/v1/verify",
            get(crate::verify::verify_get).post(crate::verify::verify_post),
        )
        .route(
            "/auth/v1/user",
            get(crate::user::user_get).put(crate::user::user_update),
        )
        .route(
            "/auth/v1/user/identities/authorize",
            get(crate::user::link_identity),
        )
        .route(
            "/auth/v1/user/identities/:identity_id",
            delete(crate::user::delete_identity),
        )
        .route(
            "/auth/v1/user/oauth/grants",
            get(crate::user::list_grants).delete(crate::user::revoke_grant),
        )
        .with_state(state)
}

// megabase:unit auth:route:GET /auth/v1/health
async fn health() -> Response {
    JsonOk(json!({
        "version": AUTH_VERSION,
        "name": "GoTrue",
        "description": "GoTrue is a user registration and authentication API",
    }))
    .into_response()
}

// megabase:unit auth:route:GET /auth/v1/settings
async fn settings(State(state): State<AuthState>) -> Response {
    let config = &state.config;
    let external = &config.external;
    JsonOk(json!({
        "external": {
            "anonymous_users": config.anonymous_users,
            "apple": external.apple,
            "azure": external.azure,
            "bitbucket": external.bitbucket,
            "discord": external.discord,
            "facebook": external.facebook,
            "snapchat": external.snapchat,
            "figma": external.figma,
            "fly": external.fly,
            "github": external.github,
            "gitlab": external.gitlab,
            "google": external.google,
            "keycloak": external.keycloak,
            "kakao": external.kakao,
            "linkedin": external.linkedin,
            "linkedin_oidc": external.linkedin_oidc,
            "notion": external.notion,
            "spotify": external.spotify,
            "slack": external.slack,
            "slack_oidc": external.slack_oidc,
            "workos": external.workos,
            "twitch": external.twitch,
            "twitter": external.twitter,
            "email": config.email_enabled,
            "phone": config.phone_enabled,
            "zoom": external.zoom,
        },
        "disable_signup": config.disable_signup,
        "mailer_autoconfirm": config.mailer_autoconfirm,
        "phone_autoconfirm": config.phone_autoconfirm,
        "sms_provider": config.sms_provider,
        "saml_enabled": config.saml_enabled,
        "saml_private_key_next_configured": config.saml_private_key_next_configured,
        "passkeys_enabled": config.passkeys_enabled,
    }))
    .into_response()
}

async fn reauthenticate() -> MegabaseNotImplemented {
    MegabaseNotImplemented::new(COMPONENT, UNIT_REAUTHENTICATE)
}

async fn invite() -> MegabaseNotImplemented {
    MegabaseNotImplemented::new(COMPONENT, UNIT_INVITE)
}

async fn recover() -> MegabaseNotImplemented {
    MegabaseNotImplemented::new(COMPONENT, UNIT_RECOVER)
}

async fn resend() -> MegabaseNotImplemented {
    MegabaseNotImplemented::new(COMPONENT, UNIT_RESEND)
}

// megabase:unit auth:route:POST /auth/v1/signup
async fn signup(State(state): State<AuthState>, headers: HeaderMap, body: Body) -> Response {
    let bytes = match read_body(body).await {
        Ok(bytes) => bytes,
        Err(error) => return error.into_response(),
    };
    let params: SignupBody = match serde_json::from_slice(&bytes) {
        Ok(params) => params,
        Err(error) => return bad_json(error).into_response(),
    };
    if params.email.is_empty() && params.phone.is_empty() {
        if !state.config.anonymous_users {
            return GoTrueError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "anonymous_provider_disabled",
                "Anonymous sign-ins are disabled",
            )
            .into_response();
        }
        return MegabaseNotImplemented::new(COMPONENT, UNIT_SIGNUP).into_response();
    }
    if state.config.disable_signup {
        return GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "signup_disabled",
            "Signups not allowed for this instance",
        )
        .into_response();
    }
    if let Err(error) = validate_signup(&state.config, &params) {
        return error.into_response();
    }
    if params.email.is_empty() {
        if !state.config.phone_enabled {
            return GoTrueError::new(
                StatusCode::BAD_REQUEST,
                "phone_provider_disabled",
                "Phone signups are disabled",
            )
            .into_response();
        }
        return MegabaseNotImplemented::new(COMPONENT, UNIT_SIGNUP).into_response();
    }
    if !state.config.email_enabled {
        return GoTrueError::new(
            StatusCode::BAD_REQUEST,
            "email_provider_disabled",
            "Email signups are disabled",
        )
        .into_response();
    }
    let email = match validate_email(&params.email) {
        Ok(email) => email,
        Err(error) => return error.into_response(),
    };
    if !state.config.mailer_autoconfirm {
        return MegabaseNotImplemented::new(COMPONENT, UNIT_SIGNUP).into_response();
    }
    let Some(jwt) = state.jwt.clone() else {
        return unexpected("Server lacks JWT secret").into_response();
    };
    let aud = request_aud(&headers, &state.config);
    let data = params.data.unwrap_or_default();
    let result = state
        .backend
        .signup_email(SignupCommand {
            email,
            password: params.password,
            aud,
            role: state.config.jwt_default_group.clone(),
            data,
        })
        .await;
    let issued = match result {
        Ok(SignupResult::Created(issued)) => issued,
        Ok(SignupResult::AlreadyExists) => {
            return GoTrueError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "user_already_exists",
                "User already registered",
            )
            .into_response();
        }
        Err(StoreError::Unavailable) => {
            return unexpected("Database error saving new user").into_response();
        }
        Err(StoreError::Hash) => {
            return unexpected("Database error creating user").into_response();
        }
        Err(error) => {
            tracing::error!(%error, "signup failed");
            return unexpected("Database error saving new user").into_response();
        }
    };
    let now = unix_secs(SystemTime::now());
    let claims = access_claims(
        &issued.user,
        issued.session_id,
        issued.amr_at,
        &issued.amr_method,
        &state.config.jwt_issuer,
        now,
        state.config.jwt_exp_seconds,
    );
    let access_token = match jwt.sign(&claims) {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "signing signup access token failed");
            return unexpected("error generating jwt token").into_response();
        }
    };
    let body = session_json(&issued, &access_token, state.config.jwt_exp_seconds, now);
    let mut response = JsonOk(body).into_response();
    insert_header(
        &mut response,
        "sb-auth-user-id",
        &issued.user.id.to_string(),
    );
    insert_header(
        &mut response,
        "sb-auth-session-id",
        &issued.session_id.to_string(),
    );
    let prefix: String = issued.refresh_token.chars().take(5).collect();
    insert_header(&mut response, "sb-auth-refresh-token-prefix", &prefix);
    response
}

// megabase:unit auth:route:POST /auth/v1/logout
async fn logout(State(state): State<AuthState>, headers: HeaderMap, uri: Uri) -> Response {
    let header = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let Some(token) = bearer_token(header) else {
        return GoTrueError::new(
            StatusCode::UNAUTHORIZED,
            "no_authorization",
            "This endpoint requires a valid Bearer token",
        )
        .into_response();
    };
    let Some(jwt) = state.jwt.as_ref() else {
        return unexpected("Server lacks JWT secret").into_response();
    };
    let claims = match jwt.verify(token) {
        Ok(claims) => claims,
        Err(error) => return jwt_failure(&error),
    };
    let Some(sub) = claims.sub.as_deref() else {
        return GoTrueError::new(
            StatusCode::FORBIDDEN,
            "bad_jwt",
            "invalid claim: missing sub claim",
        )
        .into_response();
    };
    let user_id = match Uuid::parse_str(sub) {
        Ok(id) => id,
        Err(_) => {
            return GoTrueError::new(
                StatusCode::BAD_REQUEST,
                "bad_jwt",
                "invalid claim: sub claim must be a UUID",
            )
            .into_response();
        }
    };
    let session_id = match session_claim(&claims) {
        Ok(id) => id,
        Err(error) => return error.into_response(),
    };
    let scope = match parse_scope(query_param(uri.query(), "scope").as_deref()) {
        Ok(scope) => scope,
        Err(error) => return error.into_response(),
    };
    let subject = match state.backend.load_subject(user_id).await {
        Ok(Some(subject)) => subject,
        Ok(None) => {
            return GoTrueError::new(
                StatusCode::FORBIDDEN,
                "user_not_found",
                "User from sub claim in JWT does not exist",
            )
            .into_response();
        }
        Err(error) => {
            tracing::error!(%error, "logout user lookup failed");
            return unexpected("Database error finding user").into_response();
        }
    };
    if banned(subject.banned_until) {
        return GoTrueError::new(StatusCode::FORBIDDEN, "user_banned", "User is banned")
            .into_response();
    }
    if let Some(session_id) = session_id {
        match state.backend.session_exists(session_id).await {
            Ok(true) => {}
            Ok(false) => {
                return GoTrueError::new(
                    StatusCode::FORBIDDEN,
                    "session_not_found",
                    "Session from session_id claim in JWT does not exist",
                )
                .into_response();
            }
            Err(error) => {
                tracing::error!(%error, "logout session lookup failed");
                return unexpected("Database error finding user").into_response();
            }
        }
    }
    if let Err(error) = state.backend.logout(user_id, session_id, scope).await {
        tracing::error!(%error, "logout failed");
        return unexpected("Error logging out user").into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
struct SignupBody {
    #[serde(default)]
    email: String,
    #[serde(default)]
    phone: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    data: Option<Map<String, Value>>,
    #[serde(default)]
    channel: String,
    #[serde(default)]
    code_challenge: String,
    #[serde(default)]
    code_challenge_method: String,
}

fn validate_signup(config: &AuthConfig, params: &SignupBody) -> Result<(), GoTrueError> {
    if params.password.is_empty() {
        return Err(validation("Signup requires a valid password"));
    }
    if params.password.len() > 72 {
        return Err(validation("Password cannot be longer than 72 characters"));
    }
    if params.password.len() < config.password_min_length {
        return Err(GoTrueError::weak_password(
            format!(
                "Password should be at least {} characters.",
                config.password_min_length
            ),
            vec!["length"],
        ));
    }
    if !params.email.is_empty() && !params.phone.is_empty() {
        return Err(validation(
            "Only an email address or phone number should be provided on signup.",
        ));
    }
    let phone_signup = params.email.is_empty();
    let channel = if phone_signup && params.channel.is_empty() {
        "sms"
    } else {
        params.channel.as_str()
    };
    if phone_signup && !valid_channel(channel, &config.sms_provider) {
        return Err(validation(INVALID_CHANNEL));
    }
    if phone_signup && !params.code_challenge.is_empty() {
        return Err(validation("PKCE not supported for phone signups"));
    }
    validate_pkce(&params.code_challenge_method, &params.code_challenge)
}

pub(crate) fn valid_channel(channel: &str, sms_provider: &str) -> bool {
    match channel {
        "sms" => true,
        "whatsapp" => sms_provider == "twilio" || sms_provider == "twilio_verify",
        _ => false,
    }
}

fn validate_pkce(method: &str, challenge: &str) -> Result<(), GoTrueError> {
    if challenge.is_empty() != method.is_empty() {
        return Err(validation(
            "PKCE flow requires code_challenge_method and code_challenge",
        ));
    }
    if challenge.is_empty() {
        return Ok(());
    }
    if !(43..=128).contains(&challenge.len()) {
        return Err(validation(
            "code challenge has to be between 43 and 128 characters",
        ));
    }
    if !challenge
        .chars()
        .all(|ch| matches!(ch, 'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '.' | '_' | '~'))
    {
        return Err(validation(
            "code challenge can only contain alphanumeric characters, hyphens, periods, underscores and tildes",
        ));
    }
    Ok(())
}

pub(crate) fn validate_email(email: &str) -> Result<String, GoTrueError> {
    if email.is_empty() {
        return Err(validation("An email address is required"));
    }
    if email.len() > 255 {
        return Err(validation("An email address is too long"));
    }
    if !email_format().is_match(email) {
        return Err(validation(
            "Unable to validate email address: invalid format",
        ));
    }
    Ok(email.to_lowercase())
}

fn email_format() -> &'static Regex {
    static EMAIL: OnceLock<Regex> = OnceLock::new();
    EMAIL.get_or_init(|| {
        Regex::new(
            r"^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$",
        )
        .expect("email format regex")
    })
}

pub(crate) fn request_aud(headers: &HeaderMap, config: &AuthConfig) -> String {
    headers
        .get("x-jwt-aud")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .unwrap_or(&config.jwt_aud)
        .to_string()
}

pub(crate) fn session_claim(claims: &JwtClaims) -> Result<Option<Uuid>, GoTrueError> {
    let Some(raw) = claims.raw.get("session_id") else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    let Some(text) = raw.as_str() else {
        return Err(bad_session_id());
    };
    if text.is_empty() {
        return Ok(None);
    }
    match Uuid::parse_str(text) {
        Ok(id) if id.is_nil() => Ok(None),
        Ok(id) => Ok(Some(id)),
        Err(_) => Err(bad_session_id()),
    }
}

fn bad_session_id() -> GoTrueError {
    GoTrueError::new(
        StatusCode::FORBIDDEN,
        "bad_jwt",
        "invalid claim: session_id claim must be a UUID",
    )
}

fn parse_scope(scope: Option<&str>) -> Result<LogoutScope, GoTrueError> {
    match scope.unwrap_or("") {
        "" | "global" => Ok(LogoutScope::Global),
        "local" => Ok(LogoutScope::Local),
        "others" => Ok(LogoutScope::Others),
        other => Err(validation(format!(
            "Unsupported logout scope {}",
            go_quote(other)
        ))),
    }
}

pub(crate) fn query_param(query: Option<&str>, name: &str) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if percent_decode(key) == name {
            return Some(percent_decode(value));
        }
    }
    None
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &input[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    index += 3;
                } else {
                    out.push(b'%');
                    index += 1;
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(crate) fn banned(until: Option<SystemTime>) -> bool {
    until.is_some_and(|until| SystemTime::now() < until)
}

pub(crate) fn jwt_failure(error: &JwtError) -> Response {
    GoTrueError::new(
        StatusCode::FORBIDDEN,
        "bad_jwt",
        format!("invalid JWT: unable to parse or verify signature, {error}"),
    )
    .into_response()
}

pub(crate) struct JsonOk(pub(crate) Value);

impl IntoResponse for JsonOk {
    fn into_response(self) -> Response {
        (StatusCode::OK, axum::Json(self.0)).into_response()
    }
}

pub(crate) fn insert_header(response: &mut Response, name: &'static str, value: &str) {
    let Ok(value) = axum::http::HeaderValue::from_str(value) else {
        return;
    };
    response
        .headers_mut()
        .insert(axum::http::HeaderName::from_static(name), value);
}

pub(crate) async fn read_body(body: Body) -> Result<axum::body::Bytes, GoTrueError> {
    match to_bytes(body, MAX_BODY_BYTES).await {
        Ok(bytes) => Ok(bytes),
        Err(error) => {
            let text = error.to_string().to_ascii_lowercase();
            if text.contains("length") {
                Err(body_too_large())
            } else {
                Err(unexpected("Could not read body into byte slice"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonutil::format_ts;
    use crate::store::Backend;
    use axum::body::Body;
    use axum::http::Request;
    use megabase_core::Hs256;
    use tower::ServiceExt;

    const SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";

    fn state(autoconfirm: bool, jwt: bool, backend: Backend) -> AuthState {
        let mut config = AuthConfig::reference_defaults();
        config.mailer_autoconfirm = autoconfirm;
        let jwt = jwt.then(|| Hs256::new(SECRET.as_bytes()).unwrap());
        AuthState::new(config, jwt, backend)
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

    fn memory_app() -> (Router, Backend) {
        let backend = Backend::memory();
        (router(state(true, true, backend.clone())), backend)
    }

    #[tokio::test]
    async fn health_and_settings_match_the_reference_stack() {
        let app = router(AuthState::reference());
        let (status, body) = call(app.clone(), "GET", "/auth/v1/health", None, &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["name"], "GoTrue");
        assert_eq!(body["version"], "v2.197.0");
        assert_eq!(
            body["description"],
            "GoTrue is a user registration and authentication API"
        );
        let (status, body) = call(app, "GET", "/auth/v1/settings", None, &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["external"]["email"], true);
        assert_eq!(body["external"]["phone"], true);
        assert_eq!(body["external"]["anonymous_users"], false);
        assert_eq!(body["external"]["github"], false);
        assert_eq!(body["disable_signup"], false);
        assert_eq!(body["mailer_autoconfirm"], true);
        assert_eq!(body["phone_autoconfirm"], true);
        assert_eq!(body["sms_provider"], "");
        assert_eq!(body["saml_enabled"], false);
        assert_eq!(body["passkeys_enabled"], false);
    }

    #[tokio::test]
    async fn scoped_routes_without_behavior_are_501() {
        let app = router(AuthState::reference());
        for (method, path, unit) in [
            ("GET", "/auth/v1/reauthenticate", UNIT_REAUTHENTICATE),
            ("POST", "/auth/v1/invite", UNIT_INVITE),
            ("POST", "/auth/v1/recover", UNIT_RECOVER),
            ("POST", "/auth/v1/resend", UNIT_RESEND),
        ] {
            let (status, body) = call(app.clone(), method, path, Some("{}"), &[]).await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{path}");
            assert_eq!(body["code"], "MEGABASE_NOT_IMPLEMENTED");
            assert_eq!(body["component"], "auth");
            assert_eq!(body["unit"], unit);
        }
    }

    #[tokio::test]
    async fn signup_validation_matches_gotrue() {
        let app = router(AuthState::reference());
        let (status, body) = call(app.clone(), "POST", "/auth/v1/signup", Some("{}"), &[]).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error_code"], "anonymous_provider_disabled");
        assert_eq!(body["msg"], "Anonymous sign-ins are disabled");
        assert_eq!(body["code"], 422);

        let (status, body) = call(
            app.clone(),
            "POST",
            "/auth/v1/signup",
            Some(r#"{"email":"a@example.com","password":"123"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error_code"], "weak_password");
        assert_eq!(body["msg"], "Password should be at least 6 characters.");
        assert_eq!(body["weak_password"]["reasons"][0], "length");

        let (status, body) = call(
            app.clone(),
            "POST",
            "/auth/v1/signup",
            Some(r#"{"email":"a@example.com","password":""}"#),
            &[],
        )
        .await;
        assert_eq!(body["msg"], "Signup requires a valid password");
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let long = "x".repeat(73);
        let (status, body) = call(
            app.clone(),
            "POST",
            "/auth/v1/signup",
            Some(&format!(
                r#"{{"email":"a@example.com","password":"{long}"}}"#
            )),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["msg"], "Password cannot be longer than 72 characters");

        let (status, body) = call(
            app.clone(),
            "POST",
            "/auth/v1/signup",
            Some(r#"{"email":"a@example.com","phone":"+1555","password":"secret1"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["msg"],
            "Only an email address or phone number should be provided on signup."
        );

        let (status, body) = call(
            app.clone(),
            "POST",
            "/auth/v1/signup",
            Some(r#"{"email":"not-an-email","password":"secret1"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["msg"],
            "Unable to validate email address: invalid format"
        );

        let too_long = format!("{}@x.co", "a".repeat(251));
        let (status, body) = call(
            app.clone(),
            "POST",
            "/auth/v1/signup",
            Some(&format!(r#"{{"email":"{too_long}","password":"secret1"}}"#)),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["msg"], "An email address is too long");

        let (status, body) = call(app.clone(), "POST", "/auth/v1/signup", Some("{"), &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "bad_json");

        let (status, body) = call(
            app.clone(),
            "POST",
            "/auth/v1/signup",
            Some(r#"{"email":"a@example.com","password":"secret1","code_challenge":"abc"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["msg"],
            "PKCE flow requires code_challenge_method and code_challenge"
        );

        let (status, body) = call(
            app,
            "POST",
            "/auth/v1/signup",
            Some(r#"{"phone":"+15551212","password":"secret1","channel":"fax"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["msg"], INVALID_CHANNEL);
    }

    #[tokio::test]
    async fn disabled_signup_and_providers() {
        let mut config = AuthConfig::reference_defaults();
        config.disable_signup = true;
        let app = router(AuthState::new(config, None, Backend::none()));
        let (status, body) = call(
            app,
            "POST",
            "/auth/v1/signup",
            Some(r#"{"email":"a@example.com","password":"secret1"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error_code"], "signup_disabled");

        let mut config = AuthConfig::reference_defaults();
        config.phone_enabled = false;
        let app = router(AuthState::new(config, None, Backend::none()));
        let (status, body) = call(
            app,
            "POST",
            "/auth/v1/signup",
            Some(r#"{"phone":"+15551212","password":"secret1"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "phone_provider_disabled");
    }

    #[tokio::test]
    async fn mailer_off_phone_and_missing_deps_do_not_invent_users() {
        let backend = Backend::memory();
        let off = router(state(false, true, backend.clone()));
        let (status, body) = call(
            off,
            "POST",
            "/auth/v1/signup",
            Some(r#"{"email":"a@example.com","password":"secret1"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], UNIT_SIGNUP);
        assert!(backend
            .password_hash_for_test("a@example.com")
            .await
            .is_none());

        let phone = router(state(true, true, backend.clone()));
        let (status, body) = call(
            phone,
            "POST",
            "/auth/v1/signup",
            Some(r#"{"phone":"+15551212","password":"secret1"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], UNIT_SIGNUP);

        let no_jwt = router(state(true, false, backend.clone()));
        let (status, body) = call(
            no_jwt,
            "POST",
            "/auth/v1/signup",
            Some(r#"{"email":"a@example.com","password":"secret1"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["msg"], "Server lacks JWT secret");
        assert!(backend
            .password_hash_for_test("a@example.com")
            .await
            .is_none());

        let no_db = router(state(true, true, Backend::none()));
        let (status, body) = call(
            no_db,
            "POST",
            "/auth/v1/signup",
            Some(r#"{"email":"a@example.com","password":"secret1"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["msg"], "Database error saving new user");
    }

    #[tokio::test]
    async fn autoconfirm_signup_returns_a_session_and_logout_revokes_it() {
        let (app, backend) = memory_app();
        let (status, body) = call(
            app.clone(),
            "POST",
            "/auth/v1/signup",
            Some(
                r#"{"email":"Judge-User@Example.COM","password":"secret1","data":{"name":"Ada"}}"#,
            ),
            &[("x-jwt-aud", "authenticated")],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["token_type"], "bearer");
        assert_eq!(body["expires_in"], 3600);
        assert_eq!(body["user"]["email"], "judge-user@example.com");
        assert_eq!(body["user"]["phone"], "");
        assert_eq!(body["user"]["aud"], "authenticated");
        assert_eq!(body["user"]["role"], "authenticated");
        assert_eq!(body["user"]["is_anonymous"], false);
        assert_eq!(body["user"]["app_metadata"]["provider"], "email");
        assert_eq!(body["user"]["app_metadata"]["providers"][0], "email");
        assert_eq!(body["user"]["user_metadata"]["email_verified"], true);
        assert_eq!(body["user"]["user_metadata"]["phone_verified"], false);
        assert_eq!(
            body["user"]["user_metadata"]["email"],
            "judge-user@example.com"
        );
        assert_eq!(body["user"]["user_metadata"]["sub"], body["user"]["id"]);
        assert_eq!(body["user"]["user_metadata"]["name"], "Ada");
        assert!(body["user"]["email_confirmed_at"].is_string());
        assert!(body["user"]["confirmed_at"].is_null());
        assert_eq!(body["user"]["identities"][0]["provider"], "email");
        assert_eq!(body["user"]["identities"][0]["id"], body["user"]["id"]);
        assert_eq!(
            body["user"]["identities"][0]["identity_data"]["email_verified"],
            true
        );
        assert_eq!(
            body["user"]["identities"][0]["identity_data"]["phone_verified"],
            false
        );
        assert_eq!(
            body["user"]["identities"][0]["identity_data"]["name"],
            "Ada"
        );
        let refresh = body["refresh_token"].as_str().unwrap();
        assert_eq!(refresh.len(), 12);
        let access = body["access_token"].as_str().unwrap();
        let claims = Hs256::new(SECRET.as_bytes())
            .unwrap()
            .verify(access)
            .unwrap();
        assert_eq!(claims.role.as_deref(), Some("authenticated"));
        assert_eq!(claims.sub.as_deref(), body["user"]["id"].as_str());
        assert_eq!(claims.raw["aud"], "authenticated");
        assert_eq!(claims.raw["aal"], "aal1");
        assert_eq!(claims.raw["amr"][0]["method"], "password");
        assert!(claims.raw["session_id"].is_string());
        assert_eq!(claims.raw["email"], "judge-user@example.com");
        assert_eq!(
            claims.exp.unwrap() - claims.raw["iat"].as_i64().unwrap(),
            3600
        );

        let (status, again) = call(
            app.clone(),
            "POST",
            "/auth/v1/signup",
            Some(r#"{"email":"judge-user@example.com","password":"secret1"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(again["error_code"], "user_already_exists");
        assert_eq!(again["msg"], "User already registered");

        let (status, body) = call(
            app.clone(),
            "POST",
            "/auth/v1/logout?scope=nope",
            None,
            &[("authorization", &format!("Bearer {access}"))],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["msg"], "Unsupported logout scope \"nope\"");

        let (status, body) = call(
            app.clone(),
            "POST",
            "/auth/v1/logout",
            None,
            &[("authorization", &format!("Bearer {access}"))],
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(body, Value::Null);

        let (status, body) = call(
            app.clone(),
            "POST",
            "/auth/v1/logout",
            None,
            &[("authorization", &format!("Bearer {access}"))],
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error_code"], "session_not_found");

        let (status, _) = call(app.clone(), "POST", "/auth/v1/logout", None, &[]).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let anon = Hs256::new(SECRET.as_bytes())
            .unwrap()
            .sign(&json!({"role": "anon", "exp": unix_secs(SystemTime::now()) + 60}))
            .unwrap();
        let (status, body) = call(
            app,
            "POST",
            "/auth/v1/logout",
            None,
            &[("authorization", &format!("Bearer {anon}"))],
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["msg"], "invalid claim: missing sub claim");
        assert!(backend
            .password_hash_for_test("judge-user@example.com")
            .await
            .is_some());
    }

    #[tokio::test]
    async fn logout_scopes_and_ban() {
        let (app, backend) = memory_app();
        let (status, body) = call(
            app.clone(),
            "POST",
            "/auth/v1/signup",
            Some(r#"{"email":"local@example.com","password":"secret1"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let access = body["access_token"].as_str().unwrap().to_string();
        let (status, _) = call(
            app.clone(),
            "POST",
            "/auth/v1/logout?scope=others",
            None,
            &[("authorization", &format!("Bearer {access}"))],
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = call(
            app.clone(),
            "POST",
            "/auth/v1/logout?scope=local",
            None,
            &[("authorization", &format!("Bearer {access}"))],
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, body) = call(
            app.clone(),
            "POST",
            "/auth/v1/signup",
            Some(r#"{"email":"banned@example.com","password":"secret1"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let access = body["access_token"].as_str().unwrap().to_string();
        backend.ban_for_test("banned@example.com").await;
        let (status, body) = call(
            app,
            "POST",
            "/auth/v1/logout",
            None,
            &[("authorization", &format!("Bearer {access}"))],
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error_code"], "user_banned");
        assert_eq!(body["msg"], "User is banned");
    }

    #[tokio::test]
    async fn unconfirmed_signup_keeps_the_stored_password() {
        let (app, backend) = memory_app();
        backend
            .insert_unconfirmed_for_test(
                "old@example.com",
                "authenticated",
                "authenticated",
                "stored-hash",
            )
            .await;
        let (status, body) = call(
            app,
            "POST",
            "/auth/v1/signup",
            Some(r#"{"email":"old@example.com","password":"secret1"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["user"]["user_metadata"]["email_verified"], true);
        assert!(body["user"]["email_confirmed_at"].is_string());
        assert_eq!(
            backend
                .password_hash_for_test("old@example.com")
                .await
                .as_deref(),
            Some("stored-hash")
        );
    }

    #[tokio::test]
    async fn audience_header_separates_users() {
        let (app, _) = memory_app();
        let (status, body) = call(
            app.clone(),
            "POST",
            "/auth/v1/signup",
            Some(r#"{"email":"aud@example.com","password":"secret1"}"#),
            &[("x-jwt-aud", "custom-aud")],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["user"]["aud"], "custom-aud");
        // `users_email_partial_key` is email-only for non-SSO users, so a
        // second audience does not create another account.
        let (status, body) = call(
            app,
            "POST",
            "/auth/v1/signup",
            Some(r#"{"email":"aud@example.com","password":"secret1"}"#),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error_code"], "user_already_exists");
        assert_eq!(body["msg"], "User already registered");
    }

    #[test]
    fn timestamps_are_rfc3339() {
        let text = format_ts(SystemTime::UNIX_EPOCH);
        assert!(text.ends_with('Z'), "{text}");
        assert!(text.contains('T'), "{text}");
    }
}
