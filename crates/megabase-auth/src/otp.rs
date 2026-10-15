// Ported from supabase/auth internal/api/magic_link.go and internal/api/otp.go
// (MIT), pin v2.197.0.

//! `POST /magiclink` and `POST /otp` up to the mail and SMS step.
//!
//! Provider gates, body decoding, validation, and the `create_user: false`
//! lookup match GoTrue. Every request that passes them would sign the user up
//! and send a magic link or an OTP; the mailer and SMS sender are not ported,
//! so those requests stay 501 before any write or user lookup that could
//! reveal whether the account exists.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use megabase_core::MegabaseNotImplemented;
use serde::de::IgnoredAny;
use serde::Deserialize;

use crate::error::GoTrueError;
use crate::gojson;
use crate::routes::{
    read_body, request_aud, valid_channel, validate_email, validate_pkce, INVALID_CHANNEL,
};
use crate::state::AuthState;
use crate::store::LoginChannel;
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

pub(crate) async fn magiclink(State(state): State<AuthState>, body: Body) -> Response {
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
        Ok(()) => MegabaseNotImplemented::new(COMPONENT, UNIT_MAGICLINK).into_response(),
        Err(error) => error.into_response(),
    }
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
    let result = if !params.email.is_empty() {
        magic_link_gate(&state).map_or_else(|| magic_link_params(&bytes), Err)
    } else if !params.phone.is_empty() {
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

/// `MagicLink` decode and `MagicLinkParams.Validate`.
fn magic_link_params(bytes: &[u8]) -> Result<(), GoTrueError> {
    let params = decode_first(bytes)?;
    if params.email.is_empty() {
        return Err(GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "Password recovery requires an email",
        ));
    }
    validate_email(&params.email)?;
    validate_pkce(&params.code_challenge_method, &params.code_challenge)
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

    fn app(config: AuthConfig, backend: Backend) -> Router {
        crate::router_with_state(AuthState::new(config, None, backend))
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
        assert_501(
            &post(&app, path, r#"{"email":"A@example.com"}"#).await,
            UNIT_MAGICLINK,
        );
        let pkce = json_body(&[
            ("email", "a@example.com"),
            ("code_challenge", CHALLENGE),
            ("code_challenge_method", "s256"),
        ]);
        assert_501(&post(&app, path, &pkce).await, UNIT_MAGICLINK);
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
        assert_501(
            &post(&app, path, r#"{"email":"a@example.com"} trailing"#).await,
            UNIT_MAGICLINK,
        );
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
        assert_501(
            &post(&app, path, r#"{"email":"a@example.com"}"#).await,
            UNIT_OTP,
        );
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
        assert_501(
            &post(
                &app,
                path,
                r#"{"email":"KNOWN@example.com","create_user":false}"#,
            )
            .await,
            UNIT_OTP,
        );
        // `null` keeps Go's `CreateUser: true`.
        assert_501(
            &post(
                &app,
                path,
                r#"{"email":"new@example.com","create_user":null}"#,
            )
            .await,
            UNIT_OTP,
        );
    }
}
