// Ported from supabase/auth internal/api/resend.go, internal/api/mail.go
// (sendConfirmation, sendEmailChange, sendEmail), internal/api/pkce.go, and
// internal/api/apilimiter/apilimiter.go (MIT), pin v2.197.0.

//! `POST /resend`: send a signup, email-change, SMS, or phone-change code again.
//!
//! Validation matches GoTrue, including case-insensitive JSON keys. The
//! header rate limit (`GOTRUE_RATE_LIMIT_OTP`) runs before the body is read.
//! An unknown address, an already-confirmed signup, and an email change with
//! nothing pending answer 200 `{}` and send nothing. Signup and email-change
//! mail use the same client as `POST /recover`. SMS delivery is not ported.
//! OTP generation fails the request when the system random source cannot be
//! read. A database error is 500, not an empty 200.

use std::time::SystemTime;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use megabase_core::MegabaseNotImplemented;
use serde::Deserialize;
use uuid::Uuid;

use crate::admin_batch2::generate_otp;
use crate::config::AuthConfig;
use crate::error::{unexpected, validation, GoTrueError};
use crate::gojson;
use crate::ratelimit::perform_rate_limiting;
use crate::recover::{authorize_recipient, deliver, frequency_limited, take_email_slot, Stop};
use crate::routes::{read_body, request_aud, validate_email, validate_pkce};
use crate::state::AuthState;
use crate::store::{
    email_otp_hash, EmailChangeResend, FlowStateRecord, LoginChannel, SignupResend, UserRecord,
};
use crate::token::unhandled;
use crate::user::validate_phone;
use crate::COMPONENT;

const UNIT: &str = "auth:route:POST /auth/v1/resend";
const CONFIRM_FAILED: &str = "Error sending confirmation email";
const CHANGE_FAILED: &str = "Error sending email change email";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Signup,
    EmailChange,
    Sms,
    PhoneChange,
}

/// `ResendConfirmationParams`. Keys are remapped by [`gojson::decode`] so
/// an ASCII case-insensitive duplicate keeps the last value in document
/// order, matching Go `encoding/json`.
#[derive(Default, Deserialize)]
struct ResendBody {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    phone: String,
    #[serde(default)]
    code_challenge: String,
    #[serde(default)]
    code_challenge_method: String,
}

impl ResendBody {
    /// `ResendConfirmationParams.Validate`. Stores the normalized address.
    fn validate(&mut self, config: &AuthConfig) -> Result<(), GoTrueError> {
        match self.r#type.as_str() {
            "signup" | "email_change" => {
                validate_pkce(&self.code_challenge_method, &self.code_challenge)?;
            }
            "sms" | "phone_change" => {}
            _ => {
                return Err(validation(
                    "Missing one of these types: signup, email_change, sms, phone_change",
                ))
            }
        }
        if self.email.is_empty() && self.r#type == "signup" {
            return Err(validation("Type provided requires an email address"));
        }
        if self.phone.is_empty() && self.r#type == "sms" {
            return Err(validation("Type provided requires a phone number"));
        }
        if !self.email.is_empty() && !self.phone.is_empty() {
            return Err(validation(
                "Only an email address or phone number should be provided.",
            ));
        } else if !self.email.is_empty() {
            if !config.email_enabled {
                return Err(GoTrueError::new(
                    StatusCode::BAD_REQUEST,
                    "email_provider_disabled",
                    "Email logins are disabled",
                ));
            }
            self.email = validate_email(&self.email)?;
        } else if !self.phone.is_empty() {
            if !config.phone_enabled {
                return Err(GoTrueError::new(
                    StatusCode::BAD_REQUEST,
                    "phone_provider_disabled",
                    "Phone logins are disabled",
                ));
            }
            self.phone = validate_phone(&self.phone)?;
        } else {
            return Err(validation("Missing email address or phone number"));
        }
        Ok(())
    }

    fn kind(&self) -> Kind {
        match self.r#type.as_str() {
            "email_change" => Kind::EmailChange,
            "sms" => Kind::Sms,
            "phone_change" => Kind::PhoneChange,
            _ => Kind::Signup,
        }
    }
}

// megabase:unit auth:route:POST /auth/v1/resend
pub(crate) async fn resend(
    State(state): State<AuthState>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if let Err(error) = perform_rate_limiting(
        &state.resend_limiter,
        state.sb_forwarded_for_enabled,
        &state.rate_limit_header,
        &headers,
    ) {
        return error.into_response();
    }
    let bytes = match read_body(body).await {
        Ok(bytes) => bytes,
        Err(error) => return error.into_response(),
    };
    // Go unmarshals a top-level `null` into the zero-valued params struct.
    let mut params: ResendBody =
        match gojson::decode::<Option<ResendBody>>(&bytes, &gojson::RESEND_PARAMS) {
            Ok(params) => params.unwrap_or_default(),
            Err(error) => return error.into_response(),
        };
    if let Err(error) = params.validate(&state.config) {
        return error.into_response();
    }
    let aud = request_aud(&headers, &state.config);
    let (channel, identifier) = if params.email.is_empty() {
        (LoginChannel::Phone, params.phone.clone())
    } else {
        (LoginChannel::Email, params.email.clone())
    };
    let user = match state
        .backend
        .find_login_user(channel, &identifier, &aud)
        .await
    {
        Ok(Some(user)) => user,
        Ok(None) => return crate::recover::empty_ok(),
        Err(error) => {
            tracing::error!(%error, "resend user lookup failed");
            return unexpected("Unable to process request").into_response();
        }
    };
    let pending = match state.backend.pending_mail(user.id).await {
        Ok(pending) => pending,
        Err(error) => {
            tracing::error!(%error, "resend mail state lookup failed");
            return unexpected("Unable to process request").into_response();
        }
    };
    let kind = params.kind();
    if (kind == Kind::Signup && user.email_confirmed_at.is_some())
        || (kind == Kind::Sms && user.phone_confirmed_at.is_some())
        || (kind == Kind::EmailChange && pending.email_change.is_empty())
        || (kind == Kind::PhoneChange && pending.phone_change.is_empty())
    {
        return crate::recover::empty_ok();
    }
    if kind == Kind::Sms || kind == Kind::PhoneChange {
        return MegabaseNotImplemented::new(COMPONENT, UNIT).into_response();
    }
    let pkce = !params.code_challenge.is_empty();
    let flow = match pkce_flow(
        &params.code_challenge_method,
        &params.code_challenge,
        user.id,
        kind,
    ) {
        Ok(flow) => flow,
        Err(()) => return unhandled(),
    };
    let result = match kind {
        Kind::Signup => {
            send_signup(
                &state,
                &user,
                pending.confirmation_sent_at,
                pkce,
                flow.as_ref(),
            )
            .await
        }
        Kind::EmailChange => {
            send_email_change(
                &state,
                &user,
                &pending.email_change,
                pending.email_change_sent_at,
                pkce,
                flow.as_ref(),
            )
            .await
        }
        Kind::Sms | Kind::PhoneChange => unreachable!("sms returns before send"),
    };
    match result {
        Ok(()) => crate::recover::empty_ok(),
        Err(Stop::Error(error)) => error.into_response(),
        Err(Stop::NotPorted) => MegabaseNotImplemented::new(COMPONENT, UNIT).into_response(),
    }
}

fn pkce_flow(
    method: &str,
    challenge: &str,
    user_id: Uuid,
    kind: Kind,
) -> Result<Option<FlowStateRecord>, ()> {
    if challenge.is_empty() {
        return Ok(None);
    }
    let method = method.to_ascii_lowercase();
    if method != "s256" && method != "plain" {
        return Err(());
    }
    let authentication_method = match kind {
        Kind::EmailChange => "email_change",
        _ => "email/signup",
    };
    Ok(Some(FlowStateRecord {
        provider_type: authentication_method.into(),
        auth_code: Some(Uuid::new_v4().to_string()),
        code_challenge: Some(challenge.to_string()),
        code_challenge_method: Some(method),
        user_id: Some(user_id),
        authentication_method: authentication_method.into(),
        provider_access_token: String::new(),
        provider_refresh_token: String::new(),
        created_at: None,
        auth_code_issued_at: None,
        referrer: None,
        linking_target_id: None,
        email_optional: false,
        oauth_client_state_id: None,
    }))
}

async fn send_signup(
    state: &AuthState,
    user: &UserRecord,
    sent_at: Option<SystemTime>,
    pkce: bool,
    flow: Option<&FlowStateRecord>,
) -> Result<(), Stop> {
    if let Some(message) = frequency_limited(sent_at, state.mail.max_frequency, SystemTime::now()) {
        return Err(crate::recover::over_email_limit(message).into());
    }
    let otp = generate_otp(state.otp_length).map_err(|_| unexpected(CONFIRM_FAILED))?;
    let mut token_hash = email_otp_hash(&user.email, &otp);
    if pkce {
        token_hash.insert_str(0, "pkce_");
    }
    check_mail(state, &user.email, None)?;
    deliver(&state.mail, CONFIRM_FAILED).await?;
    let sent_at = SystemTime::now();
    state
        .backend
        .save_signup_resend(SignupResend {
            user,
            token_hash: &token_hash,
            sent_at,
            flow,
        })
        .await
        .map_err(|error| {
            tracing::error!(%error, "saving resend confirmation failed");
            unexpected(CONFIRM_FAILED)
        })?;
    Ok(())
}

async fn send_email_change(
    state: &AuthState,
    user: &UserRecord,
    new_email: &str,
    sent_at: Option<SystemTime>,
    pkce: bool,
    flow: Option<&FlowStateRecord>,
) -> Result<(), Stop> {
    if let Some(message) = frequency_limited(sent_at, state.mail.max_frequency, SystemTime::now()) {
        return Err(crate::recover::over_email_limit(message).into());
    }
    let otp_new = generate_otp(state.otp_length).map_err(|_| unexpected(CHANGE_FAILED))?;
    let mut token_new = email_otp_hash(new_email, &otp_new);
    let secure = state.config.secure_email_change && !user.email.is_empty();
    let token_current = if secure {
        let otp_current = generate_otp(state.otp_length).map_err(|_| unexpected(CHANGE_FAILED))?;
        let mut hash = email_otp_hash(&user.email, &otp_current);
        if pkce {
            hash.insert_str(0, "pkce_");
        }
        Some(hash)
    } else {
        None
    };
    if pkce {
        token_new.insert_str(0, "pkce_");
    }
    let current = secure.then_some(user.email.as_str());
    check_mail(state, new_email, current)?;
    deliver(&state.mail, CHANGE_FAILED).await?;
    state
        .backend
        .save_email_change_resend(EmailChangeResend {
            user,
            email_change: new_email,
            token_current: token_current.as_deref(),
            token_new: &token_new,
            sent_at: SystemTime::now(),
            flow,
        })
        .await
        .map_err(|error| {
            tracing::error!(%error, "saving resend email change failed");
            unexpected(CHANGE_FAILED)
        })?;
    Ok(())
}

/// Authorization for every recipient, then one email-send slot.
fn check_mail(state: &AuthState, recipient: &str, also: Option<&str>) -> Result<(), Stop> {
    authorize_recipient(state, recipient)?;
    if let Some(also) = also {
        authorize_recipient(state, also)?;
    }
    take_email_slot(state)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use axum::Router;
    use megabase_core::Hs256;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use super::*;
    use crate::config::AuthConfig;
    use crate::ratelimit::KeyedLimiter;
    use crate::store::Backend;

    const SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";
    /// RFC 7636 appendix B.
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    #[test]
    fn case_insensitive_duplicates_keep_the_last_key_in_document_order() {
        let params: ResendBody = gojson::decode(
            br#"{"type":"signup","EMAIL":"first@example.com","email":"second@example.com","EMAIL":"third@example.com"}"#,
            &gojson::RESEND_PARAMS,
        )
        .unwrap_or_else(|error| panic!("{}", error.message));
        assert_eq!(params.r#type, "signup");
        assert_eq!(params.email, "third@example.com");
    }

    fn app(autoconfirm: bool) -> (Router, Backend) {
        let backend = Backend::memory();
        let jwt = Hs256::new(SECRET.as_bytes()).unwrap();
        let mut config = AuthConfig::reference_defaults();
        config.mailer_autoconfirm = autoconfirm;
        let mut state = AuthState::new(config, Some(jwt), backend.clone());
        state.mail.smtp_host.clear();
        state.mail.max_frequency = Duration::ZERO;
        (crate::router_with_state(state), backend)
    }

    async fn call(app: &Router, body: &str, header: Option<(&str, &str)>) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/auth/v1/resend")
            .header("content-type", "application/json");
        if let Some((name, value)) = header {
            builder = builder.header(name, value);
        }
        let response = app
            .clone()
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn signup(app: &Router, email: &str) {
        let request = Request::builder()
            .method("POST")
            .uri("/auth/v1/signup")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "email": email, "password": "secret-pass-1" }).to_string(),
            ))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_and_confirmed_addresses_send_nothing() {
        let (app, backend) = app(true);
        let (status, body) = call(
            &app,
            r#"{"type":"signup","email":"ghost@example.com"}"#,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({}));
        signup(&app, "ada@example.com").await;
        let (status, body) =
            call(&app, r#"{"type":"signup","email":"Ada@Example.com"}"#, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            backend.confirmation_token_for_test("ada@example.com").await,
            ""
        );
        let (status, body) = call(
            &app,
            r#"{"type":"email_change","email":"ada@example.com"}"#,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({}));
    }

    #[tokio::test]
    async fn unconfirmed_signup_resend_stores_a_fresh_token() {
        let (app, backend) = app(false);
        signup(&app, "ada@example.com").await;
        let first = backend.confirmation_token_for_test("ada@example.com").await;
        assert_eq!(first.len(), 56);
        let (status, body) =
            call(&app, r#"{"type":"signup","email":"ADA@example.com"}"#, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let second = backend.confirmation_token_for_test("ada@example.com").await;
        assert_eq!(second.len(), 56);
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn email_change_resend_stores_both_secure_hashes_and_a_pkce_flow() {
        let (app, backend) = app(true);
        signup(&app, "ada@example.com").await;
        backend
            .stage_email_change_for_test("ada@example.com", "new@example.com")
            .await;
        let body = json!({
            "type": "email_change",
            "email": "ada@example.com",
            "code_challenge": CHALLENGE,
            "code_challenge_method": "S256",
        })
        .to_string();
        let (status, json_body) = call(&app, &body, None).await;
        assert_eq!(status, StatusCode::OK, "{json_body}");
        let (current, new) = backend
            .email_change_tokens_for_test("ada@example.com")
            .await;
        assert!(current.starts_with("pkce_"), "{current}");
        assert!(new.starts_with("pkce_"), "{new}");
        assert_eq!(current.len(), 61);
        assert_eq!(new.len(), 61);
        assert_ne!(current, new);
        let user_id = {
            let request = Request::builder()
                .method("POST")
                .uri("/auth/v1/token?grant_type=password")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "email": "ada@example.com", "password": "secret-pass-1" }).to_string(),
                ))
                .unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let session: Value = serde_json::from_slice(&bytes).unwrap();
            uuid::Uuid::parse_str(session["user"]["id"].as_str().unwrap()).unwrap()
        };
        let (method, challenge) = backend.flow_for_user_for_test(user_id).await.unwrap();
        assert_eq!(method, "email_change");
        assert_eq!(challenge, CHALLENGE);
    }

    #[tokio::test]
    async fn header_limit_runs_before_the_body_is_parsed() {
        let backend = Backend::memory();
        let mut state = AuthState::new(AuthConfig::reference_defaults(), None, backend);
        state.mail.smtp_host.clear();
        state.rate_limit_header = "x-rate-key".into();
        state.resend_limiter = std::sync::Arc::new(KeyedLimiter::new(0.0, 1.0));
        let app = crate::router_with_state(state);
        let (status, _) = call(
            &app,
            r#"{"type":"signup","email":"ghost@example.com"}"#,
            Some(("x-rate-key", "one")),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, body) = call(&app, "{", Some(("x-rate-key", "one"))).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body["error_code"], "over_request_rate_limit");
    }

    #[tokio::test]
    async fn repeated_signup_inside_the_frequency_window_is_429() {
        let backend = Backend::memory();
        let jwt = Hs256::new(SECRET.as_bytes()).unwrap();
        let mut config = AuthConfig::reference_defaults();
        config.mailer_autoconfirm = false;
        let mut state = AuthState::new(config, Some(jwt), backend.clone());
        state.mail.smtp_host.clear();
        let app = crate::router_with_state(state);
        signup(&app, "ada@example.com").await;
        let first = backend.confirmation_token_for_test("ada@example.com").await;
        let (status, body) =
            call(&app, r#"{"type":"signup","email":"ada@example.com"}"#, None).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body["error_code"], "over_email_send_rate_limit");
        assert_eq!(
            backend.confirmation_token_for_test("ada@example.com").await,
            first
        );
    }
}
