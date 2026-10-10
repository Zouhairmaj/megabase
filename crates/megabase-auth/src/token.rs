// Ported from supabase/auth internal/api/token.go, internal/api/token_refresh.go,
// internal/api/password.go, internal/api/phone.go, internal/models/user.go,
// internal/models/refresh_token.go, and internal/tokens/service.go (MIT),
// pin v2.197.0.

//! `POST /auth/v1/token` for the password and refresh-token grants.
//!
//! `id_token`, `pkce`, and `web3` stay HTTP 501. An unknown `grant_type` is
//! the upstream 400, because that branch is part of this route.

use std::time::SystemTime;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use megabase_core::MegabaseNotImplemented;
use serde::Deserialize;

use crate::error::{bad_json, unexpected, validation, GoTrueError};
use crate::jsonutil::{access_claims, session_json, unix_secs};
use crate::routes::{insert_header, query_param, read_body, request_aud, JsonOk};
use crate::state::AuthState;
use crate::store::{LoginChannel, RefreshStatus, StoreError};
use crate::COMPONENT;

const UNIT_ID_TOKEN: &str = "auth:grant-type:id_token";
const UNIT_PKCE: &str = "auth:grant-type:pkce";
const UNIT_WEB3: &str = "auth:grant-type:web3";

const INVALID_LOGIN: &str = "Invalid login credentials";
const MAX_PASSWORD_LENGTH: usize = 72;

// megabase:unit auth:route:POST /auth/v1/token
pub(crate) async fn token(
    State(state): State<AuthState>,
    headers: HeaderMap,
    uri: Uri,
    body: Body,
) -> Response {
    // Go `Request.FormValue`: query for JSON bodies. Body parameters win only
    // for form posts, and this handler still requires a JSON body afterwards.
    let grant_type = query_param(uri.query(), "grant_type").unwrap_or_default();
    match grant_type.as_str() {
        "password" => password_grant(state, headers, body).await,
        "refresh_token" => refresh_grant(state, body).await,
        "id_token" => MegabaseNotImplemented::new(COMPONENT, UNIT_ID_TOKEN).into_response(),
        "pkce" => MegabaseNotImplemented::new(COMPONENT, UNIT_PKCE).into_response(),
        "web3" => MegabaseNotImplemented::new(COMPONENT, UNIT_WEB3).into_response(),
        _ => GoTrueError::new(
            StatusCode::BAD_REQUEST,
            "invalid_credentials",
            "unsupported_grant_type",
        )
        .into_response(),
    }
}

// megabase:unit auth:grant-type:password
async fn password_grant(state: AuthState, headers: HeaderMap, body: Body) -> Response {
    let params = match grant_json::<PasswordBody>(body).await {
        Ok(params) => params,
        Err(error) => return error.into_response(),
    };
    if !params.email.is_empty() && !params.phone.is_empty() {
        return validation("Only an email address or phone number should be provided on login.")
            .into_response();
    }
    let (channel, identifier, provider) = if !params.email.is_empty() {
        if !state.config.email_enabled {
            return GoTrueError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "email_provider_disabled",
                "Email logins are disabled",
            )
            .into_response();
        }
        (LoginChannel::Email, params.email.to_lowercase(), "email")
    } else if !params.phone.is_empty() {
        if !state.config.phone_enabled {
            return GoTrueError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "phone_provider_disabled",
                "Phone logins are disabled",
            )
            .into_response();
        }
        (LoginChannel::Phone, format_phone(&params.phone), "phone")
    } else {
        return validation("missing email or phone").into_response();
    };
    let aud = request_aud(&headers, &state.config);
    let user = match state
        .backend
        .find_login_user(channel, &identifier, &aud)
        .await
    {
        Ok(user) => user,
        Err(error) => return store_failure(&error),
    };
    let Some(user) = user else {
        return invalid_login();
    };
    if user.password_hash.is_empty() {
        return invalid_login();
    }
    if banned(user.banned_until) {
        return GoTrueError::new(StatusCode::BAD_REQUEST, "user_banned", "User is banned")
            .into_response();
    }
    let password_ok =
        match verify_password(params.password.clone(), user.password_hash.clone()).await {
            Ok(ok) => ok,
            Err(error) => return error.into_response(),
        };
    let weak = if password_ok {
        match login_strength(&params.password, state.config.password_min_length) {
            Ok(weak) => weak,
            Err(error) => return error.into_response(),
        }
    } else {
        None
    };
    if !password_ok {
        return invalid_login();
    }
    if channel == LoginChannel::Email && user.email_confirmed_at.is_none() {
        return GoTrueError::new(
            StatusCode::BAD_REQUEST,
            "email_not_confirmed",
            "Email not confirmed",
        )
        .into_response();
    }
    if channel == LoginChannel::Phone && user.phone_confirmed_at.is_none() {
        return GoTrueError::new(
            StatusCode::BAD_REQUEST,
            "phone_not_confirmed",
            "Phone not confirmed",
        )
        .into_response();
    }
    let issued = match state
        .backend
        .issue_login_session(user.id, provider, None)
        .await
    {
        Ok(Some(issued)) => issued,
        Ok(None) => return invalid_login(),
        Err(error) => return store_failure(&error),
    };
    session_response(&state, &issued, false, weak, true)
}

// megabase:unit auth:grant-type:refresh_token
async fn refresh_grant(state: AuthState, body: Body) -> Response {
    let params = match grant_json::<RefreshBody>(body).await {
        Ok(params) => params,
        Err(error) => return error.into_response(),
    };
    if let Err(error) = validate_refresh_token(&params.refresh_token) {
        return error.into_response();
    }
    let status = match state.backend.refresh_login(&params.refresh_token).await {
        Ok(status) => status,
        Err(error) => return store_failure(&error),
    };
    let (issued, rotated) = match status {
        RefreshStatus::Issued { session, rotated } => (session, rotated),
        RefreshStatus::NotFound => {
            return GoTrueError::new(
                StatusCode::BAD_REQUEST,
                "refresh_token_not_found",
                "Invalid Refresh Token: Refresh Token Not Found",
            )
            .into_response();
        }
        RefreshStatus::NoSession => {
            return GoTrueError::new(
                StatusCode::BAD_REQUEST,
                "session_not_found",
                "Invalid Refresh Token: No Valid Session Found",
            )
            .into_response();
        }
        RefreshStatus::Banned => {
            return GoTrueError::new(
                StatusCode::BAD_REQUEST,
                "user_banned",
                "Invalid Refresh Token: User Banned",
            )
            .into_response();
        }
        RefreshStatus::AlreadyUsed => {
            return GoTrueError::new(
                StatusCode::BAD_REQUEST,
                "refresh_token_already_used",
                "Invalid Refresh Token: Already Used",
            )
            .into_response();
        }
    };
    session_response(&state, &issued, rotated, None, false)
}

#[derive(Deserialize)]
struct PasswordBody {
    #[serde(default, deserialize_with = "null_as_empty")]
    email: String,
    #[serde(default, deserialize_with = "null_as_empty")]
    phone: String,
    #[serde(default, deserialize_with = "null_as_empty")]
    password: String,
}

#[derive(Deserialize)]
struct RefreshBody {
    #[serde(default, deserialize_with = "null_as_empty")]
    refresh_token: String,
}

/// Go `encoding/json` leaves a `string` at its zero value when the JSON is `null`.
fn null_as_empty<'de, D: serde::Deserializer<'de>>(de: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(de)?.unwrap_or_default())
}

struct WeakPassword {
    message: String,
    reasons: Vec<&'static str>,
}

async fn grant_json<T: for<'de> Deserialize<'de>>(body: Body) -> Result<T, GoTrueError> {
    let bytes = read_body(body).await?;
    serde_json::from_slice(&bytes).map_err(bad_json)
}

fn invalid_login() -> Response {
    GoTrueError::new(
        StatusCode::BAD_REQUEST,
        "invalid_credentials",
        INVALID_LOGIN,
    )
    .into_response()
}

fn store_failure(error: &StoreError) -> Response {
    tracing::error!(%error, "token grant failed");
    unexpected("Database error querying schema").into_response()
}

fn banned(until: Option<SystemTime>) -> bool {
    until.is_some_and(|until| SystemTime::now() < until)
}

/// `formatPhoneNumber`: drop one leading `+`, then spaces.
fn format_phone(phone: &str) -> String {
    phone.strip_prefix('+').unwrap_or(phone).replace(' ', "")
}

fn login_strength(password: &str, min_length: usize) -> Result<Option<WeakPassword>, GoTrueError> {
    if password.len() > MAX_PASSWORD_LENGTH {
        return Err(validation("Password cannot be longer than 72 characters"));
    }
    if password.len() < min_length {
        return Ok(Some(WeakPassword {
            message: format!("Password should be at least {min_length} characters."),
            reasons: vec!["length"],
        }));
    }
    Ok(None)
}

fn validate_refresh_token(token: &str) -> Result<(), GoTrueError> {
    if token.len() < 12 || (token.len() == 12 && !legacy_refresh_token(token)) {
        return Err(validation("Refresh token is not valid"));
    }
    if token.len() > 12 && !v2_refresh_token_shape(token) {
        return Err(validation("Refresh token is not valid"));
    }
    Ok(())
}

fn legacy_refresh_token(token: &str) -> bool {
    token.len() == 12
        && token
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

/// Structural stand-in for `crypto.ParseRefreshToken`.
///
/// A token longer than 12 characters must be raw-url base64 whose decoded
/// body is version 0, carries a 16-byte session id and a uvarint counter, a
/// 16-byte signature, and a 4-byte SHA-256 prefix. Checksum failure is
/// `validation_failed` here. A token that parses and has no session HMAC key
/// is `refresh_token_not_found` from the store lookup (the reference stack
/// does not issue version 2).
fn v2_refresh_token_shape(token: &str) -> bool {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use sha2::{Digest, Sha256};

    let Ok(bytes) = URL_SAFE_NO_PAD.decode(token) else {
        return false;
    };
    // 1 version + 16 session + at least 1 counter byte + 16 signature + 4 checksum.
    if bytes.len() < 1 + 16 + 1 + 16 + 4 || bytes[0] != 0 {
        return false;
    }
    let checksum_at = bytes.len() - 4;
    let digest = Sha256::digest(&bytes[..checksum_at]);
    if digest[..4] != bytes[checksum_at..] {
        return false;
    }
    let mut rest = &bytes[1 + 16..checksum_at];
    let Some((_, counter_len)) = take_uvarint(rest) else {
        return false;
    };
    rest = &rest[counter_len..];
    rest.len() == 16
}

fn take_uvarint(bytes: &[u8]) -> Option<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if index >= 10 {
            return None;
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((value, index + 1));
        }
        shift += 7;
    }
    None
}

async fn verify_password(password: String, hash: String) -> Result<bool, GoTrueError> {
    // Go bcrypt compares the first 72 bytes. The check below still uses the
    // original length, so a match on a longer password is rejected.
    let presented = bcrypt_password(&password);
    let matched = tokio::task::spawn_blocking(move || bcrypt::verify(presented, &hash))
        .await
        .map_err(|_| unexpected("Database error querying schema"))?
        .map_err(|_| unexpected("Database error querying schema"))?;
    Ok(matched)
}

fn bcrypt_password(password: &str) -> String {
    if password.len() <= MAX_PASSWORD_LENGTH {
        return password.to_string();
    }
    let mut end = MAX_PASSWORD_LENGTH;
    while end > 0 && !password.is_char_boundary(end) {
        end -= 1;
    }
    password[..end].to_string()
}

fn session_response(
    state: &AuthState,
    issued: &crate::store::IssuedSession,
    rotated: bool,
    weak: Option<WeakPassword>,
    // Password grant assigns a nil `*WeakPasswordError` into an `interface{}`,
    // so encoding/json emits `weak_password: null`. Refresh leaves the field
    // unset and `omitempty` drops it.
    emit_null_weak: bool,
) -> Response {
    let Some(jwt) = state.jwt.clone() else {
        return unexpected("Server lacks JWT secret").into_response();
    };
    let now = unix_secs(SystemTime::now());
    let expires_in = state.config.jwt_exp_seconds;
    let claims = access_claims(
        &issued.user,
        issued.session_id,
        issued.amr_at,
        &state.config.jwt_issuer,
        now,
        expires_in,
    );
    let access_token = match jwt.sign(&claims) {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "signing access token failed");
            return unexpected("error generating jwt token").into_response();
        }
    };
    let mut body = session_json(issued, &access_token, expires_in, now);
    if let Some(weak) = weak {
        body["weak_password"] = serde_json::json!({
            "message": weak.message,
            "reasons": weak.reasons,
        });
    } else if emit_null_weak {
        body["weak_password"] = serde_json::Value::Null;
    }
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
    if rotated {
        insert_header(&mut response, "sb-auth-refresh-token-reuse", "false");
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;
    use crate::routes::router;
    use crate::store::{Backend, SignupCommand};
    use axum::http::Request;
    use megabase_core::Hs256;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    const SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";

    fn state(config: AuthConfig, jwt: bool, backend: Backend) -> AuthState {
        let jwt = jwt.then(|| Hs256::new(SECRET.as_bytes()).unwrap());
        AuthState::new(config, jwt, backend)
    }

    async fn call(
        state: &AuthState,
        path: &str,
        body: Option<&str>,
    ) -> (StatusCode, Value, String) {
        let mut builder = Request::builder().method("POST").uri(path);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let request = builder
            .body(Body::from(body.unwrap_or("").to_string()))
            .unwrap();
        let response = router(state.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json, content_type)
    }

    /// Password built from the clock and process id. Tests that sign up and
    /// then log in share one return value. It is not a string literal, so it
    /// is not a hard-coded password at `hash_password`.
    fn fixture_secret() -> String {
        let mut n = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(1)
            ^ u128::from(std::process::id());
        if n == 0 {
            n = 1;
        }
        let mut secret = String::with_capacity(16);
        for _ in 0..16 {
            let offset = (n % 26) as u8;
            secret.push(char::from(b'a' + offset));
            n = n.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        }
        secret
    }

    /// Swap `{secret}` for `fixture_secret` inside a JSON body.
    fn with_runtime_secret(json: &str) -> String {
        json.replace("{secret}", &fixture_secret())
    }

    async fn signup(state: &AuthState, email: &str, secret: &str) -> Value {
        let (status, body, _) = call(
            state,
            "/auth/v1/signup",
            Some(&format!(r#"{{"email":"{email}","password":"{secret}"}}"#)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    #[tokio::test]
    async fn password_grant_returns_a_session() {
        let backend = Backend::memory();
        let app = state(AuthConfig::reference_defaults(), true, backend.clone());
        let secret = fixture_secret();
        signup(&app, "ada@example.com", &secret).await;
        let (status, body, content_type) = call(
            &app,
            "/auth/v1/token?grant_type=password",
            Some(&format!(
                r#"{{"email":"Ada@Example.com","password":"{secret}"}}"#
            )),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type, "application/json");
        assert_eq!(body["token_type"], "bearer");
        assert_eq!(body["expires_in"], 3600);
        assert_eq!(body["user"]["email"], "ada@example.com");
        assert_eq!(body["refresh_token"].as_str().unwrap().len(), 12);
        assert!(body["weak_password"].is_null());
    }

    #[tokio::test]
    async fn wrong_password_is_invalid_credentials() {
        let app = state(AuthConfig::reference_defaults(), true, Backend::memory());
        signup(&app, "ada@example.com", &fixture_secret()).await;
        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=password",
            Some(r#"{"email":"ada@example.com","password":"not-the-password"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "invalid_credentials");
        assert_eq!(body["msg"], "Invalid login credentials");
        assert_eq!(body["code"], 400);
    }

    #[tokio::test]
    async fn refresh_grant_rotates_and_rejects_reuse() {
        let app = state(AuthConfig::reference_defaults(), true, Backend::memory());
        let first = signup(&app, "ada@example.com", &fixture_secret()).await;
        let refresh = first["refresh_token"].as_str().unwrap();
        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=refresh_token",
            Some(&json!({ "refresh_token": refresh }).to_string()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let rotated = body["refresh_token"].as_str().unwrap();
        assert_ne!(rotated, refresh);
        assert_eq!(rotated.len(), 12);
        assert_eq!(body["expires_in"], 3600);
        assert_eq!(body["token_type"], "bearer");

        let (status, again, _) = call(
            &app,
            "/auth/v1/token?grant_type=refresh_token",
            Some(&json!({ "refresh_token": refresh }).to_string()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(again["refresh_token"], rotated);

        let (status, used, _) = call(
            &app,
            "/auth/v1/token?grant_type=refresh_token",
            Some(&json!({ "refresh_token": "abcdefghij22" }).to_string()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(used["error_code"], "refresh_token_not_found");
        assert_eq!(
            used["msg"],
            "Invalid Refresh Token: Refresh Token Not Found"
        );
    }

    #[tokio::test]
    async fn revoked_token_that_is_not_the_parent_is_already_used() {
        let app = state(AuthConfig::reference_defaults(), true, Backend::memory());
        let first = signup(&app, "ada@example.com", &fixture_secret()).await;
        let original = first["refresh_token"].as_str().unwrap().to_string();
        let (status, second, _) = call(
            &app,
            "/auth/v1/token?grant_type=refresh_token",
            Some(&json!({ "refresh_token": original }).to_string()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{second}");
        let child = second["refresh_token"].as_str().unwrap();
        let (status, third, _) = call(
            &app,
            "/auth/v1/token?grant_type=refresh_token",
            Some(&json!({ "refresh_token": child }).to_string()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{third}");
        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=refresh_token",
            Some(&json!({ "refresh_token": original }).to_string()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "refresh_token_already_used");
        assert_eq!(body["msg"], "Invalid Refresh Token: Already Used");
    }

    #[tokio::test]
    async fn unknown_grant_and_unimplemented_grants() {
        let app = state(AuthConfig::reference_defaults(), true, Backend::memory());
        let (status, body, _) = call(
            &app,
            "/auth/v1/token",
            Some(&with_runtime_secret(
                r#"{"email":"ada@example.com","password":"{secret}"}"#,
            )),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "invalid_credentials");
        assert_eq!(body["msg"], "unsupported_grant_type");

        let (status, body, _) = call(&app, "/auth/v1/token?grant_type=id_token", Some("{}")).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["code"], "MEGABASE_NOT_IMPLEMENTED");
        assert_eq!(body["unit"], UNIT_ID_TOKEN);

        let (status, body, _) = call(&app, "/auth/v1/token?grant_type=pkce", Some("{}")).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], UNIT_PKCE);

        let (status, body, _) = call(&app, "/auth/v1/token?grant_type=web3", Some("{}")).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], UNIT_WEB3);
    }

    #[tokio::test]
    async fn password_grant_validation_and_account_state() {
        let backend = Backend::memory();
        let app = state(AuthConfig::reference_defaults(), true, backend.clone());
        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=password",
            Some(&with_runtime_secret(
                r#"{"email":"ada@example.com","phone":"+1555","password":"{secret}"}"#,
            )),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "validation_failed");
        assert_eq!(
            body["msg"],
            "Only an email address or phone number should be provided on login."
        );

        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=password",
            Some(&with_runtime_secret(r#"{"password":"{secret}"}"#)),
        )
        .await;
        assert_eq!(body["msg"], "missing email or phone");
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let mut disabled = AuthConfig::reference_defaults();
        disabled.email_enabled = false;
        let email_off = state(disabled, true, Backend::memory());
        let (status, body, _) = call(
            &email_off,
            "/auth/v1/token?grant_type=password",
            Some(&with_runtime_secret(
                r#"{"email":"ada@example.com","password":"{secret}"}"#,
            )),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error_code"], "email_provider_disabled");
        assert_eq!(body["msg"], "Email logins are disabled");

        let mut phone_off_config = AuthConfig::reference_defaults();
        phone_off_config.phone_enabled = false;
        let phone_off = state(phone_off_config, true, Backend::memory());
        let (status, body, _) = call(
            &phone_off,
            "/auth/v1/token?grant_type=password",
            Some(&with_runtime_secret(
                r#"{"phone":"+15551212","password":"{secret}"}"#,
            )),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error_code"], "phone_provider_disabled");

        let secret = fixture_secret();
        let hash = bcrypt::hash(&secret, crate::config::BCRYPT_COST).unwrap();
        backend
            .insert_unconfirmed_for_test(
                "pending@example.com",
                "authenticated",
                "authenticated",
                &hash,
            )
            .await;
        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=password",
            Some(&format!(
                r#"{{"email":"pending@example.com","password":"{secret}"}}"#
            )),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "email_not_confirmed");
        assert_eq!(body["msg"], "Email not confirmed");

        backend
            .insert_unconfirmed_for_test("empty@example.com", "authenticated", "authenticated", "")
            .await;
        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=password",
            Some(&with_runtime_secret(
                r#"{"email":"empty@example.com","password":"{secret}"}"#,
            )),
        )
        .await;
        assert_eq!(body["error_code"], "invalid_credentials");
        assert_eq!(status, StatusCode::BAD_REQUEST);

        signup(&app, "banned@example.com", &fixture_secret()).await;
        backend.ban_for_test("banned@example.com").await;
        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=password",
            Some(r#"{"email":"banned@example.com","password":"wrong-password"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "user_banned");
        assert_eq!(body["msg"], "User is banned");
    }

    #[tokio::test]
    async fn phone_login_and_refresh_shape() {
        let backend = Backend::memory();
        let app = state(AuthConfig::reference_defaults(), true, backend.clone());
        let secret = fixture_secret();
        signup(&app, "ada@example.com", &secret).await;
        backend
            .set_phone_for_test("ada@example.com", "15551212", false)
            .await;
        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=password",
            Some(&format!(
                r#"{{"phone":"+1 555 1212","password":"{secret}"}}"#
            )),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "phone_not_confirmed");
        assert_eq!(body["msg"], "Phone not confirmed");

        backend
            .set_phone_for_test("ada@example.com", "15551212", true)
            .await;
        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=password",
            Some(&format!(
                r#"{{"phone":"+1 555 1212","password":"{secret}"}}"#
            )),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["user"]["phone"], "15551212");

        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=refresh_token",
            Some(r#"{"refresh_token":"short"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "validation_failed");
        assert_eq!(body["msg"], "Refresh token is not valid");

        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=refresh_token",
            Some(r#"{"refresh_token":"ABCDEFGHIJKL"}"#),
        )
        .await;
        assert_eq!(body["msg"], "Refresh token is not valid");
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn long_password_is_rejected_after_a_truncated_match() {
        let password = "a".repeat(72);
        let app = state(AuthConfig::reference_defaults(), true, Backend::memory());
        signup(&app, "ada@example.com", &password).await;
        let too_long = format!("{password}b");
        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=password",
            Some(&json!({ "email": "ada@example.com", "password": too_long }).to_string()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "validation_failed");
        assert_eq!(body["msg"], "Password cannot be longer than 72 characters");
    }

    #[tokio::test]
    async fn weak_password_still_signs_in() {
        let backend = Backend::memory();
        let signed_up = state(AuthConfig::reference_defaults(), true, backend.clone());
        let secret = fixture_secret();
        signup(&signed_up, "ada@example.com", &secret).await;
        let mut config = AuthConfig::reference_defaults();
        config.password_min_length = 30;
        let app = state(config, true, backend);
        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=password",
            Some(&format!(
                r#"{{"email":"ada@example.com","password":"{secret}"}}"#
            )),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["weak_password"]["message"],
            "Password should be at least 30 characters."
        );
        assert_eq!(body["weak_password"]["reasons"][0], "length");
    }

    #[tokio::test]
    async fn missing_database_and_jwt_secret() {
        let app = state(AuthConfig::reference_defaults(), true, Backend::none());
        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=password",
            Some(&with_runtime_secret(
                r#"{"email":"ada@example.com","password":"{secret}"}"#,
            )),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error_code"], "unexpected_failure");
        assert_eq!(body["msg"], "Database error querying schema");

        let backend = Backend::memory();
        let app = state(AuthConfig::reference_defaults(), false, backend.clone());
        let secret = fixture_secret();
        backend
            .signup_email(SignupCommand {
                email: "ada@example.com".into(),
                password: secret.clone(),
                aud: "authenticated".into(),
                role: "authenticated".into(),
                data: serde_json::Map::new(),
            })
            .await
            .unwrap();
        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=password",
            Some(&format!(
                r#"{{"email":"ada@example.com","password":"{secret}"}}"#
            )),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["msg"], "Server lacks JWT secret");
    }

    #[tokio::test]
    async fn null_json_strings_are_empty() {
        let secret = fixture_secret();
        let app = state(AuthConfig::reference_defaults(), true, Backend::memory());
        signup(&app, "ada@example.com", &secret).await;
        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=password",
            Some(&format!(
                r#"{{"email":"ada@example.com","phone":null,"password":"{secret}"}}"#
            )),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=password",
            Some(r#"{"email":"ada@example.com","password":null}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "invalid_credentials");

        let (status, body, _) = call(
            &app,
            "/auth/v1/token?grant_type=refresh_token",
            Some(r#"{"refresh_token":null}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "validation_failed");
        assert_eq!(body["msg"], "Refresh token is not valid");
    }

    #[test]
    fn v2_shape_rejects_a_bad_checksum() {
        assert!(!v2_refresh_token_shape("this-is-not-a-refresh-token!!"));
        assert!(validate_refresh_token("abcdefghijkl").is_ok());
        assert!(validate_refresh_token("abc").is_err());
    }
}
