// Ported from supabase/auth internal/api/magic_link.go, internal/api/otp.go,
// internal/api/mail.go (sendMagicLink), and internal/crypto/password.go
// (GeneratePassword) (MIT), pin v2.197.0.

//! `POST /magiclink` and `POST /otp`.
//!
//! Provider gates, body decoding, validation, and the `create_user: false`
//! lookup match GoTrue. The email flow is served. With autoconfirm on, a new
//! address is signed up, then the magic link goes through the same mail
//! client as `POST /recover` (the no-op client when no SMTP host is set).
//! With autoconfirm off, a new or unconfirmed address gets the signup
//! confirmation email instead (`POST /signup`'s path), and a confirmed one
//! gets the magic link. The SMS sender is not ported: a phone OTP stays 501.

use std::time::SystemTime;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use megabase_core::MegabaseNotImplemented;
use serde::de::IgnoredAny;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::admin_batch2::random_bytes;
use crate::confirm::{confirm_signup, UnconfirmedSignup};
use crate::error::{unexpected, GoTrueError};
use crate::gojson;
use crate::hooks::TokenHookCall;
use crate::jsonutil::unix_secs;
use crate::recover::{create_email_flow, send_recovery, RecoveryMail, Stop};
use crate::routes::{
    read_body, request_aud, signup_outcome, valid_channel, validate_email, validate_pkce,
    INVALID_CHANNEL,
};
use crate::state::AuthState;
use crate::store::{LoginChannel, SignupCommand, UserRecord};
use crate::token::unhandled;
use crate::user::validate_phone;
use crate::COMPONENT;

const UNIT_MAGICLINK: &str = "auth:route:POST /auth/v1/magiclink";
const UNIT_OTP: &str = "auth:route:POST /auth/v1/otp";

const DECODE_PREFIX: &str = "Could not read verification params: ";

#[derive(Default, Deserialize)]
struct MagicLinkBody {
    #[serde(default)]
    email: String,
    #[serde(default)]
    data: Option<Map<String, Value>>,
    #[serde(default)]
    code_challenge_method: String,
    #[serde(default)]
    code_challenge: String,
}

#[derive(Deserialize)]
struct OtpBody {
    #[serde(default)]
    email: String,
    #[serde(default)]
    phone: String,
    #[serde(default = "create_user_default")]
    create_user: bool,
    // `data` is type-checked by `gojson::decode` and only used after delivery.
    #[serde(default)]
    channel: String,
    #[serde(default)]
    code_challenge_method: String,
    #[serde(default)]
    code_challenge: String,
}

impl Default for OtpBody {
    fn default() -> Self {
        Self {
            email: String::new(),
            phone: String::new(),
            create_user: true,
            channel: String::new(),
            code_challenge_method: String::new(),
            code_challenge: String::new(),
        }
    }
}

/// `Otp` starts from `OtpParams{CreateUser: true}`.
fn create_user_default() -> bool {
    true
}

// megabase:unit auth:route:POST /auth/v1/magiclink
pub(crate) async fn magiclink(
    State(state): State<AuthState>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if let Some(error) = magic_link_gate(&state) {
        return error.into_response();
    }
    // `json.NewDecoder(r.Body)` sees the 1 MiB `MaxBytesReader` error.
    let bytes = match read_body(body).await {
        Ok(bytes) => bytes,
        Err(error) if error.status == StatusCode::PAYLOAD_TOO_LARGE => {
            return decode_error("http: request body too large").into_response()
        }
        Err(error) => return error.into_response(),
    };
    match magic_link_params(&bytes) {
        Ok(params) => send_magic_link(&state, &headers, params, UNIT_MAGICLINK).await,
        Err(error) => error.into_response(),
    }
}

/// `MagicLink` after its params are valid: an unknown or unconfirmed
/// address is signed up first (`Signup` with a random password), then a
/// confirmed user gets the magic link, a recovery token sent with the magic
/// link template. The PKCE flow writes a `magiclink` flow state before the
/// send, as `/recover` does.
///
/// With autoconfirm off, a new or unconfirmed address goes through `Signup`
/// instead, whose confirmation email is the magic link; the answer is `{}`
/// either way.
async fn send_magic_link(
    state: &AuthState,
    headers: &HeaderMap,
    params: MagicLinkBody,
    unit: &'static str,
) -> Response {
    let aud = request_aud(headers, &state.config);
    let user = match find_confirmed(state, &params.email, &aud).await {
        Ok(Some(user)) => user,
        Ok(None) if !state.config.mailer_autoconfirm => {
            return confirm_for_magic_link(state, headers, params, unit).await;
        }
        Ok(None) => {
            if let Err(response) = signup_for_magic_link(state, headers, &params, aud.clone()).await
            {
                return *response;
            }
            // GoTrue calls `MagicLink` again on the same params; the user
            // is confirmed now.
            match find_confirmed(state, &params.email, &aud).await {
                Ok(Some(user)) => user,
                Ok(None) => return unhandled(),
                Err(error) => return error.into_response(),
            }
        }
        Err(error) => return error.into_response(),
    };
    let pkce = !params.code_challenge.is_empty();
    if pkce {
        if let Err(response) = create_email_flow(
            state,
            &user,
            RecoveryMail::MagicLink,
            &params.code_challenge,
            &params.code_challenge_method,
        )
        .await
        {
            return *response;
        }
    }
    match send_recovery(state, &user, pkce, RecoveryMail::MagicLink).await {
        Ok(()) => (StatusCode::OK, Json(json!({}))).into_response(),
        Err(Stop::Error(error)) => error.into_response(),
        Err(Stop::NotPorted) => MegabaseNotImplemented::new(COMPONENT, unit).into_response(),
    }
}

/// `FindUserByEmailAndAudience` followed by `IsConfirmed`: `None` is the
/// `isNewUser` case (no row, or an unconfirmed email).
async fn find_confirmed(
    state: &AuthState,
    email: &str,
    aud: &str,
) -> Result<Option<UserRecord>, GoTrueError> {
    match state
        .backend
        .find_login_user(LoginChannel::Email, email, aud)
        .await
    {
        Ok(user) => Ok(user.filter(|user| user.email_confirmed_at.is_some())),
        Err(error) => {
            tracing::error!(%error, "magic link user lookup failed");
            Err(unexpected("Database error finding user"))
        }
    }
}

/// The `Signup` call `MagicLink` makes for a new address while autoconfirm is
/// on: a 33-character random password (`crypto.GeneratePassword`), the
/// request's `data`, and the session `Signup` issues, which is discarded.
async fn signup_for_magic_link(
    state: &AuthState,
    headers: &HeaderMap,
    params: &MagicLinkBody,
    aud: String,
) -> Result<(), Box<Response>> {
    let config = &state.config;
    signup_gates(state)?;
    if state.jwt.is_none() {
        return Err(Box::new(
            unexpected("Server lacks JWT secret").into_response(),
        ));
    }
    let password = generate_password().map_err(|_| Box::new(unhandled()))?;
    let now = unix_secs(SystemTime::now());
    // `Signup` issues its session with `models.PasswordGrant`.
    let hook = TokenHookCall::new(
        config.custom_access_token_hook.as_ref(),
        headers,
        "password",
        &config.jwt_issuer,
        now,
        config.jwt_exp_seconds,
    );
    let result = state
        .backend
        .signup_email(
            SignupCommand {
                email: params.email.clone(),
                password,
                aud,
                role: config.jwt_default_group.clone(),
                data: params.data.clone().unwrap_or_default(),
            },
            hook.as_ref(),
        )
        .await;
    signup_outcome(result).map(drop)
}

/// The `Signup` call `MagicLink` makes for a new or unconfirmed address
/// while autoconfirm is off: the same generated password and `data`, plus
/// the PKCE params, so `sendConfirmation` writes the `email/signup` flow
/// state and a `pkce_` token. `MagicLink` discards `Signup`'s body and
/// answers `{}`.
async fn confirm_for_magic_link(
    state: &AuthState,
    headers: &HeaderMap,
    params: MagicLinkBody,
    unit: &'static str,
) -> Response {
    if let Err(response) = signup_gates(state) {
        return *response;
    }
    let Ok(password) = generate_password() else {
        return unhandled();
    };
    let signup = UnconfirmedSignup {
        email: params.email,
        password,
        data: params.data.unwrap_or_default(),
        code_challenge: params.code_challenge,
        code_challenge_method: params.code_challenge_method,
    };
    match confirm_signup(state, headers, &signup).await {
        Ok(response) if response.status().is_success() => {
            (StatusCode::OK, Json(json!({}))).into_response()
        }
        Ok(response) => response,
        Err(Stop::Error(error)) => error.into_response(),
        Err(Stop::NotPorted) => MegabaseNotImplemented::new(COMPONENT, unit).into_response(),
    }
}

/// The `Signup` checks that run before its user lookup: `DisableSignup`,
/// then the generated password against `GOTRUE_PASSWORD_MIN_LENGTH`.
fn signup_gates(state: &AuthState) -> Result<(), Box<Response>> {
    let config = &state.config;
    if config.disable_signup {
        return Err(Box::new(
            GoTrueError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "signup_disabled",
                "Signups not allowed for this instance",
            )
            .into_response(),
        ));
    }
    if GENERATED_PASSWORD_LENGTH < config.password_min_length {
        return Err(Box::new(
            GoTrueError::weak_password(
                format!(
                    "Password should be at least {} characters.",
                    config.password_min_length
                ),
                vec!["length"],
            )
            .into_response(),
        ));
    }
    Ok(())
}

const GENERATED_PASSWORD_LENGTH: usize = 33;

/// `crypto.GeneratePassword(nil, 33)`: letters and digits. GoTrue's
/// required-character groups are not configurable in Megabase yet.
fn generate_password() -> Result<String, crate::http::AuthError> {
    const ALL_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    // 248 is the largest multiple of 62 below 256; higher bytes are skipped
    // so every character is equally likely.
    let mut password = String::with_capacity(GENERATED_PASSWORD_LENGTH);
    while password.len() < GENERATED_PASSWORD_LENGTH {
        for byte in random_bytes(GENERATED_PASSWORD_LENGTH)? {
            if password.len() == GENERATED_PASSWORD_LENGTH {
                break;
            }
            if byte < 248 {
                password.push(char::from(ALL_CHARS[usize::from(byte) % ALL_CHARS.len()]));
            }
        }
    }
    Ok(password)
}

// The `/otp` decode and `create_user` lookup are served; delivery is not.
pub(crate) async fn otp(
    State(state): State<AuthState>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let bytes = match read_body(body).await {
        Ok(bytes) => bytes,
        Err(error) => return error.into_response(),
    };
    let params: OtpBody = if is_null(&bytes) {
        OtpBody::default()
    } else {
        match gojson::decode(&bytes, &gojson::OTP_PARAMS) {
            Ok(params) => params,
            Err(error) => return error.into_response(),
        }
    };
    if let Err(error) = validate_otp(&params) {
        return error.into_response();
    }
    match should_create_user(&state, &headers, &params).await {
        Ok(true) => {}
        Ok(false) => {
            return GoTrueError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "otp_disabled",
                "Signups not allowed for otp",
            )
            .into_response()
        }
        Err(error) => return error.into_response(),
    }
    if !params.email.is_empty() {
        // `Otp` hands the request to `MagicLink`, which decodes the body again.
        if let Some(error) = magic_link_gate(&state) {
            return error.into_response();
        }
        return match magic_link_params(&bytes) {
            Ok(params) => send_magic_link(&state, &headers, params, UNIT_OTP).await,
            Err(error) => error.into_response(),
        };
    }
    let result = if !params.phone.is_empty() {
        sms_params(&state, &params)
    } else {
        Err(crate::error::validation(
            "One of email or phone must be set",
        ))
    };
    match result {
        Ok(()) => MegabaseNotImplemented::new(COMPONENT, UNIT_OTP).into_response(),
        Err(error) => error.into_response(),
    }
}

/// `OtpParams.Validate`.
fn validate_otp(params: &OtpBody) -> Result<(), GoTrueError> {
    if !params.email.is_empty() && !params.phone.is_empty() {
        return Err(crate::error::validation(
            "Only an email address or phone number should be provided",
        ));
    }
    if !params.email.is_empty() && !params.channel.is_empty() {
        return Err(crate::error::validation(
            "Channel should only be specified with Phone OTP",
        ));
    }
    validate_pkce(&params.code_challenge_method, &params.code_challenge)
}

/// `shouldCreateUser`: with `create_user: false`, an unknown address is
/// refused. A database error other than "not found" lets the request through.
async fn should_create_user(
    state: &AuthState,
    headers: &HeaderMap,
    params: &OtpBody,
) -> Result<bool, GoTrueError> {
    if params.create_user {
        return Ok(true);
    }
    let aud = request_aud(headers, &state.config);
    let lookup = if !params.email.is_empty() {
        let email = validate_email(&params.email)?;
        state
            .backend
            .find_login_user(LoginChannel::Email, &email, &aud)
            .await
    } else if !params.phone.is_empty() {
        let phone = validate_phone(&params.phone)?;
        state
            .backend
            .find_login_user(LoginChannel::Phone, &phone, &aud)
            .await
    } else {
        return Ok(true);
    };
    Ok(!matches!(lookup, Ok(None)))
}

/// The provider checks at the top of `MagicLink`.
fn magic_link_gate(state: &AuthState) -> Option<GoTrueError> {
    let config = &state.config;
    if !config.email_enabled {
        return Some(GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "email_provider_disabled",
            "Email logins are disabled",
        ));
    }
    if !config.magic_link_enabled {
        return Some(GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "email_provider_disabled",
            "Login with magic link is disabled",
        ));
    }
    None
}

/// `MagicLink` decode and `MagicLinkParams.Validate`, which lowercases the
/// email.
fn magic_link_params(bytes: &[u8]) -> Result<MagicLinkBody, GoTrueError> {
    let mut params = decode_first(bytes)?;
    if params.email.is_empty() {
        return Err(GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "Password recovery requires an email",
        ));
    }
    params.email = validate_email(&params.email)?;
    validate_pkce(&params.code_challenge_method, &params.code_challenge)?;
    Ok(params)
}

/// `json.NewDecoder(r.Body).Decode`: only the first JSON value is read, so
/// trailing bytes are ignored, and an empty body is `EOF`.
fn decode_first(bytes: &[u8]) -> Result<MagicLinkBody, GoTrueError> {
    let mut stream = serde_json::Deserializer::from_slice(bytes).into_iter::<IgnoredAny>();
    match stream.next() {
        None => return Err(decode_error("EOF")),
        Some(Err(error)) if error.is_eof() => return Err(decode_error("unexpected EOF")),
        Some(Err(error)) => {
            let detail = gojson::syntax_error(bytes).unwrap_or_else(|| error.to_string());
            return Err(decode_error(detail));
        }
        Some(Ok(_)) => {}
    }
    let first = &bytes[..stream.byte_offset()];
    if is_null(first) {
        return Ok(MagicLinkBody::default());
    }
    gojson::decode(first, &gojson::MAGIC_LINK_PARAMS).map_err(|mut error| {
        if let Some(detail) = error
            .message
            .strip_prefix("Could not parse request body as JSON: ")
        {
            error.message = format!("{DECODE_PREFIX}{detail}");
        }
        error
    })
}

fn decode_error(detail: impl std::fmt::Display) -> GoTrueError {
    GoTrueError::new(
        StatusCode::BAD_REQUEST,
        "bad_json",
        format!("{DECODE_PREFIX}{detail}"),
    )
}

/// `SmsOtp` up to `sendPhoneConfirmation`. Its `SmsParams` decode of the same
/// body cannot fail once `OtpParams` decoded: the shared fields have the same
/// Go types.
fn sms_params(state: &AuthState, params: &OtpBody) -> Result<(), GoTrueError> {
    let config = &state.config;
    if !config.phone_enabled {
        return Err(GoTrueError::new(
            StatusCode::BAD_REQUEST,
            "phone_provider_disabled",
            "Unsupported phone provider",
        ));
    }
    let channel = if params.channel.is_empty() {
        "sms"
    } else {
        params.channel.as_str()
    };
    validate_phone(&params.phone)?;
    if !valid_channel(channel, &config.sms_provider) {
        return Err(crate::error::validation(INVALID_CHANNEL));
    }
    Ok(())
}

fn is_null(bytes: &[u8]) -> bool {
    bytes.trim_ascii() == b"null"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;
    use crate::store::Backend;
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::Router;
    use serde_json::Value;
    use tower::ServiceExt;

    const CHALLENGE: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ";

    const SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";

    fn app(config: AuthConfig, backend: Backend) -> Router {
        let jwt = megabase_core::Hs256::new(SECRET.as_bytes()).unwrap();
        let mut state = AuthState::new(config, Some(jwt), backend);
        // No SMTP host: GoTrue's no-op mail client.
        state.mail.smtp_host = String::new();
        crate::router_with_state(state)
    }

    fn reference() -> Router {
        app(AuthConfig::reference_defaults(), Backend::memory())
    }

    async fn post(app: &Router, path: &str, body: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn assert_error(result: &(StatusCode, Value), status: u16, code: &str, msg: &str) {
        assert_eq!(result.0.as_u16(), status, "{}", result.1);
        assert_eq!(result.1["error_code"], code, "{}", result.1);
        assert_eq!(result.1["msg"], msg, "{}", result.1);
    }

    fn assert_501(result: &(StatusCode, Value), unit: &str) {
        assert_eq!(result.0, StatusCode::NOT_IMPLEMENTED, "{}", result.1);
        assert_eq!(result.1["unit"], unit, "{}", result.1);
    }

    #[tokio::test]
    async fn magiclink_validates_before_the_mail_step() {
        let app = reference();
        let path = "/auth/v1/magiclink";
        for body in ["{}", "null"] {
            assert_error(
                &post(&app, path, body).await,
                422,
                "validation_failed",
                "Password recovery requires an email",
            );
        }
        assert_error(
            &post(&app, path, r#"{"email":"nope"}"#).await,
            400,
            "validation_failed",
            "Unable to validate email address: invalid format",
        );
        assert_error(
            &post(
                &app,
                path,
                r#"{"email":"a@example.com","code_challenge":"x"}"#,
            )
            .await,
            400,
            "validation_failed",
            "PKCE flow requires code_challenge_method and code_challenge",
        );
        assert_ok(&post(&app, path, r#"{"email":"A@example.com"}"#).await);
        let pkce = json_body(&[
            ("email", "b@example.com"),
            ("code_challenge", CHALLENGE),
            ("code_challenge_method", "s256"),
        ]);
        assert_ok(&post(&app, path, &pkce).await);
    }

    fn assert_ok(result: &(StatusCode, Value)) {
        assert_eq!(result.0, StatusCode::OK, "{}", result.1);
        assert_eq!(result.1, serde_json::json!({}));
    }

    fn json_body(pairs: &[(&str, &str)]) -> String {
        let object: serde_json::Map<String, Value> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), Value::from(*value)))
            .collect();
        Value::Object(object).to_string()
    }

    #[tokio::test]
    async fn magiclink_decodes_like_a_go_json_decoder() {
        let app = reference();
        let path = "/auth/v1/magiclink";
        assert_error(
            &post(&app, path, "").await,
            400,
            "bad_json",
            "Could not read verification params: EOF",
        );
        assert_error(
            &post(&app, path, r#"{"email":"#).await,
            400,
            "bad_json",
            "Could not read verification params: unexpected EOF",
        );
        assert_error(
            &post(&app, path, "x").await,
            400,
            "bad_json",
            "Could not read verification params: invalid character 'x' looking for beginning of value",
        );
        assert_error(
            &post(&app, path, r#"{"email":1}"#).await,
            400,
            "bad_json",
            "Could not read verification params: json: cannot unmarshal number into Go struct field MagicLinkParams.email of type string",
        );
        // The decoder stops after the first value.
        assert_ok(&post(&app, path, r#"{"email":"a@example.com"} trailing"#).await);
    }

    #[tokio::test]
    async fn magiclink_provider_gates_are_422() {
        let mut config = AuthConfig::reference_defaults();
        config.magic_link_enabled = false;
        assert_error(
            &post(&app(config, Backend::memory()), "/auth/v1/magiclink", "{}").await,
            422,
            "email_provider_disabled",
            "Login with magic link is disabled",
        );
        let mut config = AuthConfig::reference_defaults();
        config.email_enabled = false;
        let off = app(config, Backend::memory());
        assert_error(
            &post(&off, "/auth/v1/magiclink", "{}").await,
            422,
            "email_provider_disabled",
            "Email logins are disabled",
        );
        assert_error(
            &post(&off, "/auth/v1/otp", r#"{"email":"a@example.com"}"#).await,
            422,
            "email_provider_disabled",
            "Email logins are disabled",
        );
    }

    #[tokio::test]
    async fn otp_validates_and_dispatches() {
        let app = reference();
        let path = "/auth/v1/otp";
        for body in ["{}", "null", r#"{"email":""}"#] {
            assert_error(
                &post(&app, path, body).await,
                400,
                "validation_failed",
                "One of email or phone must be set",
            );
        }
        assert_error(
            &post(&app, path, r#"{"create_user":"no"}"#).await,
            400,
            "bad_json",
            "Could not parse request body as JSON: json: cannot unmarshal string into Go struct field OtpParams.create_user of type bool",
        );
        assert_error(
            &post(
                &app,
                path,
                r#"{"email":"a@example.com","phone":"15550001111"}"#,
            )
            .await,
            400,
            "validation_failed",
            "Only an email address or phone number should be provided",
        );
        assert_error(
            &post(&app, path, r#"{"email":"a@example.com","channel":"sms"}"#).await,
            400,
            "validation_failed",
            "Channel should only be specified with Phone OTP",
        );
        assert_ok(&post(&app, path, r#"{"email":"a@example.com"}"#).await);
        assert_error(
            &post(&app, path, r#"{"phone":"abc"}"#).await,
            400,
            "validation_failed",
            "Invalid phone number format (E.164 required)",
        );
        assert_error(
            &post(
                &app,
                path,
                r#"{"phone":"15550001111","channel":"whatsapp"}"#,
            )
            .await,
            400,
            "validation_failed",
            INVALID_CHANNEL,
        );
        assert_501(
            &post(&app, path, r#"{"phone":"+1 555 000 1111"}"#).await,
            UNIT_OTP,
        );
    }

    #[tokio::test]
    async fn otp_phone_provider_off_is_unsupported() {
        let mut config = AuthConfig::reference_defaults();
        config.phone_enabled = false;
        let app = app(config, Backend::memory());
        assert_error(
            &post(&app, "/auth/v1/otp", r#"{"phone":"15550001111"}"#).await,
            400,
            "phone_provider_disabled",
            "Unsupported phone provider",
        );
    }

    #[tokio::test]
    async fn otp_without_create_user_refuses_unknown_addresses() {
        let backend = Backend::memory();
        backend
            .insert_unconfirmed_for_test("known@example.com", "authenticated", "authenticated", "")
            .await;
        let app = app(AuthConfig::reference_defaults(), backend);
        let path = "/auth/v1/otp";
        for body in [
            r#"{"email":"new@example.com","create_user":false}"#,
            r#"{"phone":"15550001111","create_user":false}"#,
        ] {
            assert_error(
                &post(&app, path, body).await,
                422,
                "otp_disabled",
                "Signups not allowed for otp",
            );
        }
        assert_error(
            &post(&app, path, r#"{"email":"bad","create_user":false}"#).await,
            400,
            "validation_failed",
            "Unable to validate email address: invalid format",
        );
        // The unconfirmed user is confirmed by `MagicLink`'s signup, then
        // gets the link.
        assert_ok(
            &post(
                &app,
                path,
                r#"{"email":"KNOWN@example.com","create_user":false}"#,
            )
            .await,
        );
        // `null` keeps Go's `CreateUser: true`.
        assert_ok(
            &post(
                &app,
                path,
                r#"{"email":"new@example.com","create_user":null}"#,
            )
            .await,
        );
    }

    /// RFC 7636 appendix B.
    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const S256_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    async fn get_location(app: &Router, path: &str) -> (StatusCode, String) {
        let request = Request::builder()
            .method("GET")
            .uri(path)
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let location = response
            .headers()
            .get("location")
            .map(|value| value.to_str().unwrap().to_string())
            .unwrap_or_default();
        (response.status(), location)
    }

    #[tokio::test]
    async fn pkce_magic_link_signs_up_verifies_and_exchanges() {
        let backend = Backend::memory();
        let app = app(AuthConfig::reference_defaults(), backend.clone());
        let body = serde_json::json!({
            "email": "New@Example.com",
            "data": { "name": "Ada" },
            "code_challenge": S256_CHALLENGE,
            "code_challenge_method": "S256",
        })
        .to_string();
        assert_ok(&post(&app, "/auth/v1/magiclink", &body).await);
        let token = backend.recovery_token_for_test("new@example.com").await;
        assert!(token.starts_with("pkce_"), "{token}");

        let link = format!("/auth/v1/verify?type=magiclink&token={token}");
        let (status, location) = get_location(&app, &link).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let code = location
            .strip_prefix("http://localhost:3000?code=")
            .unwrap_or_else(|| panic!("{location}"))
            .to_string();

        let exchange = serde_json::json!({ "auth_code": code, "code_verifier": VERIFIER });
        let path = "/auth/v1/token?grant_type=pkce";
        let (status, session) = post(&app, path, &exchange.to_string()).await;
        assert_eq!(status, StatusCode::OK, "{session}");
        assert_eq!(session["user"]["email"], "new@example.com");
        assert_eq!(session["user"]["user_metadata"]["name"], "Ada");
        assert!(session["user"]["email_confirmed_at"].is_string());

        let (status, again) = post(&app, path, &exchange.to_string()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(again["error_code"], "flow_state_not_found");
    }

    #[tokio::test]
    async fn implicit_magic_link_redirects_with_a_session() {
        let backend = Backend::memory();
        let app = app(AuthConfig::reference_defaults(), backend.clone());
        let signup = r#"{"email":"known@example.com","password":"known-password-1"}"#;
        assert_eq!(
            post(&app, "/auth/v1/signup", signup).await.0,
            StatusCode::OK
        );
        assert_ok(
            &post(
                &app,
                "/auth/v1/otp",
                r#"{"email":"known@example.com","create_user":false}"#,
            )
            .await,
        );
        let token = backend.recovery_token_for_test("known@example.com").await;
        assert!(!token.is_empty() && !token.starts_with("pkce_"), "{token}");
        // The signup password is untouched.
        let login = r#"{"email":"known@example.com","password":"known-password-1"}"#;
        let (status, _) = post(&app, "/auth/v1/token?grant_type=password", login).await;
        assert_eq!(status, StatusCode::OK);

        let link = format!("/auth/v1/verify?type=magiclink&token={token}");
        let (status, location) = get_location(&app, &link).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert!(
            location.starts_with("http://localhost:3000#access_token="),
            "{location}"
        );
        assert!(
            location.ends_with("&token_type=bearer&type=magiclink"),
            "{location}"
        );
        // The link is single-use.
        let (_, location) = get_location(&app, &link).await;
        assert!(location.contains("error_code=otp_expired"), "{location}");

        // A second link within the frequency window is refused.
        let result = post(
            &app,
            "/auth/v1/magiclink",
            r#"{"email":"known@example.com"}"#,
        )
        .await;
        assert_eq!(result.0, StatusCode::TOO_MANY_REQUESTS, "{}", result.1);
        assert_eq!(result.1["error_code"], "over_email_send_rate_limit");
    }

    #[tokio::test]
    async fn magic_link_signup_gates() {
        for autoconfirm in [true, false] {
            let mut config = AuthConfig::reference_defaults();
            config.mailer_autoconfirm = autoconfirm;
            config.disable_signup = true;
            let backend = Backend::memory();
            backend
                .insert_unconfirmed_for_test(
                    "known@example.com",
                    "authenticated",
                    "authenticated",
                    "",
                )
                .await;
            let closed = app(config, backend);
            for email in ["new@example.com", "known@example.com"] {
                let body = serde_json::json!({ "email": email }).to_string();
                assert_error(
                    &post(&closed, "/auth/v1/magiclink", &body).await,
                    422,
                    "signup_disabled",
                    "Signups not allowed for this instance",
                );
            }

            let mut config = AuthConfig::reference_defaults();
            config.mailer_autoconfirm = autoconfirm;
            config.password_min_length = 40;
            let strict = app(config, Backend::memory());
            let result = post(
                &strict,
                "/auth/v1/magiclink",
                r#"{"email":"a@example.com"}"#,
            )
            .await;
            assert_eq!(result.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", result.1);
            assert_eq!(result.1["error_code"], "weak_password");
        }
    }

    /// Autoconfirm off, no SMTP host, and no `GOTRUE_SMTP_MAX_FREQUENCY`
    /// wait so one address can get several emails.
    fn unconfirmed_app(backend: Backend) -> Router {
        let mut config = AuthConfig::reference_defaults();
        config.mailer_autoconfirm = false;
        let jwt = megabase_core::Hs256::new(SECRET.as_bytes()).unwrap();
        let mut state = AuthState::new(config, Some(jwt), backend);
        state.mail.smtp_host = String::new();
        state.mail.max_frequency = std::time::Duration::ZERO;
        crate::router_with_state(state)
    }

    #[tokio::test]
    async fn pkce_magic_link_without_autoconfirm_sends_the_confirmation() {
        let backend = Backend::memory();
        let app = unconfirmed_app(backend.clone());
        let body = serde_json::json!({
            "email": "New@Example.com",
            "data": { "name": "Ada" },
            "code_challenge": S256_CHALLENGE,
            "code_challenge_method": "S256",
        })
        .to_string();
        assert_ok(&post(&app, "/auth/v1/magiclink", &body).await);
        // `Signup` sent the confirmation email; no magic link was sent.
        let token = backend.confirmation_token_for_test("new@example.com").await;
        assert!(token.starts_with("pkce_"), "{token}");
        assert_eq!(backend.recovery_token_for_test("new@example.com").await, "");

        let link = format!("/auth/v1/verify?type=signup&token={token}");
        let (status, location) = get_location(&app, &link).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let code = location
            .strip_prefix("http://localhost:3000?code=")
            .unwrap_or_else(|| panic!("{location}"))
            .to_string();
        let exchange = serde_json::json!({ "auth_code": code, "code_verifier": VERIFIER });
        let path = "/auth/v1/token?grant_type=pkce";
        let (status, session) = post(&app, path, &exchange.to_string()).await;
        assert_eq!(status, StatusCode::OK, "{session}");
        assert_eq!(session["user"]["email"], "new@example.com");
        assert_eq!(session["user"]["user_metadata"]["name"], "Ada");
        assert!(session["user"]["email_confirmed_at"].is_string());

        // The address is confirmed now, so the next request is a magic link.
        let again = serde_json::json!({
            "email": "new@example.com",
            "code_challenge": S256_CHALLENGE,
            "code_challenge_method": "s256",
        })
        .to_string();
        assert_ok(&post(&app, "/auth/v1/otp", &again).await);
        let token = backend.recovery_token_for_test("new@example.com").await;
        assert!(token.starts_with("pkce_"), "{token}");
    }

    #[tokio::test]
    async fn otp_without_autoconfirm_resends_to_an_unconfirmed_user() {
        let backend = Backend::memory();
        let app = unconfirmed_app(backend.clone());
        let signup = r#"{"email":"known@example.com","password":"known-password-1"}"#;
        assert_eq!(
            post(&app, "/auth/v1/signup", signup).await.0,
            StatusCode::OK
        );
        let first = backend
            .confirmation_token_for_test("known@example.com")
            .await;
        assert_ok(
            &post(
                &app,
                "/auth/v1/otp",
                r#"{"email":"known@example.com","create_user":false}"#,
            )
            .await,
        );
        let token = backend
            .confirmation_token_for_test("known@example.com")
            .await;
        assert!(!token.is_empty() && !token.starts_with("pkce_"), "{token}");
        assert_ne!(token, first);

        let verify = serde_json::json!({ "type": "signup", "token_hash": token }).to_string();
        let (status, session) = post(&app, "/auth/v1/verify", &verify).await;
        assert_eq!(status, StatusCode::OK, "{session}");
        // `Signup` keeps an unconfirmed user's password.
        let (status, _) = post(&app, "/auth/v1/token?grant_type=password", signup).await;
        assert_eq!(status, StatusCode::OK);

        // An unknown challenge method is `Signup`'s 500, and nothing is kept.
        let body = serde_json::json!({
            "email": "bad@example.com",
            "code_challenge": S256_CHALLENGE,
            "code_challenge_method": "sha1",
        })
        .to_string();
        let result = post(&app, "/auth/v1/magiclink", &body).await;
        assert_eq!(result.0, StatusCode::INTERNAL_SERVER_ERROR, "{}", result.1);
        assert_eq!(result.1["error_code"], "unexpected_failure");
        assert!(backend
            .password_hash_for_test("bad@example.com")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn magic_link_without_autoconfirm_reports_mail_failures() {
        // A reachable SMTP host needs the SMTP exchange, which is not ported.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = AuthConfig::reference_defaults();
        config.mailer_autoconfirm = false;
        let backend = Backend::memory();
        let jwt = megabase_core::Hs256::new(SECRET.as_bytes()).unwrap();
        let mut state = AuthState::new(config, Some(jwt), backend.clone());
        state.mail.smtp_host = "127.0.0.1".into();
        state.mail.smtp_port = listener.local_addr().unwrap().port();
        let app = crate::router_with_state(state);
        assert_501(
            &post(&app, "/auth/v1/magiclink", r#"{"email":"a@example.com"}"#).await,
            UNIT_MAGICLINK,
        );
        assert_501(
            &post(&app, "/auth/v1/otp", r#"{"email":"a@example.com"}"#).await,
            UNIT_OTP,
        );
        assert!(backend
            .password_hash_for_test("a@example.com")
            .await
            .is_none());
    }

    #[test]
    fn generated_passwords_are_alphanumeric() {
        let password = generate_password().unwrap();
        assert_eq!(password.len(), GENERATED_PASSWORD_LENGTH);
        assert!(password.bytes().all(|byte| byte.is_ascii_alphanumeric()));
        assert_ne!(password, generate_password().unwrap());
    }
}
