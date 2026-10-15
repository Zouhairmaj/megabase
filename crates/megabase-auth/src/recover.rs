// Ported from supabase/auth internal/api/recover.go, internal/api/mail.go
// (sendPasswordRecovery, sendEmail, checkEmailAddressAuthorization,
// validateSentWithinFrequencyLimit), internal/api/errors.go, and
// internal/mailer/templatemailer/template.go (MIT), pin v2.197.0.

//! `POST /recover`: start password recovery for an email address.
//!
//! Unknown addresses answer 200 `{}` before any limit or delivery, so the
//! answer does not reveal whether an account exists. For a known address the
//! recovery email goes through GoTrue's mail client: with no SMTP host it is
//! the no-op client, which accepts the message without sending it, and the
//! token is stored for `POST /verify`. An SMTP host that cannot be reached
//! fails the way GoTrue's dialer does (500, nothing stored). A reachable host
//! would need the SMTP conversation, templates, and TLS, which are not
//! ported, so that request stays 501. The PKCE flow (`code_challenge`) also
//! stays 501.

use std::time::{Duration, SystemTime};

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use megabase_core::MegabaseNotImplemented;
use serde::Deserialize;
use serde_json::json;
use tokio::net::TcpStream;

use crate::admin_batch2::generate_otp;
use crate::error::{bad_json, go_quote, unexpected, validation, GoTrueError};
use crate::routes::{read_body, request_aud, validate_email, validate_pkce};
use crate::state::{AuthState, MailConfig};
use crate::store::{email_otp_hash, LoginChannel};
use crate::token::null_as_default;
use crate::COMPONENT;

const UNIT_RECOVER: &str = "auth:route:POST /auth/v1/recover";

/// gomail's `Dialer.Timeout`.
const SMTP_DIAL_TIMEOUT: Duration = Duration::from_secs(10);

const SEND_FAILED: &str = "Error sending recovery email";

#[derive(Deserialize)]
struct RecoverBody {
    #[serde(default, deserialize_with = "null_as_default")]
    email: String,
    #[serde(default, deserialize_with = "null_as_default")]
    code_challenge: String,
    #[serde(default, deserialize_with = "null_as_default")]
    code_challenge_method: String,
}

impl RecoverBody {
    /// `RecoverParams.Validate`. Returns the lowercased email.
    fn validate(&self) -> Result<String, GoTrueError> {
        if self.email.is_empty() {
            return Err(validation("Password recovery requires an email"));
        }
        let email = validate_email(&self.email)?;
        validate_pkce(&self.code_challenge_method, &self.code_challenge)?;
        Ok(email)
    }
}

/// Why a recovery request stopped after the user lookup.
enum Stop {
    Error(GoTrueError),
    NotPorted,
}

impl From<GoTrueError> for Stop {
    fn from(error: GoTrueError) -> Self {
        Self::Error(error)
    }
}

// megabase:unit auth:route:POST /auth/v1/recover
pub(crate) async fn recover(
    State(state): State<AuthState>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if !state.config.email_enabled {
        return GoTrueError::new(
            StatusCode::BAD_REQUEST,
            "email_provider_disabled",
            "Email logins are disabled",
        )
        .into_response();
    }
    let bytes = match read_body(body).await {
        Ok(bytes) => bytes,
        Err(error) => return error.into_response(),
    };
    let params: RecoverBody = match serde_json::from_slice(&bytes) {
        Ok(params) => params,
        Err(error) => return bad_json(error).into_response(),
    };
    let email = match params.validate() {
        Ok(email) => email,
        Err(error) => return error.into_response(),
    };
    if !params.code_challenge.is_empty() {
        // PKCE recovery writes `auth.flow_state` and a `pkce_` token prefix
        // that verify does not exchange yet.
        return MegabaseNotImplemented::new(COMPONENT, UNIT_RECOVER).into_response();
    }
    let aud = request_aud(&headers, &state.config);
    let user = match state
        .backend
        .find_login_user(LoginChannel::Email, &email, &aud)
        .await
    {
        Ok(Some(user)) => user,
        Ok(None) => return empty_ok(),
        Err(error) => {
            tracing::error!(%error, "recover user lookup failed");
            return unexpected("Unable to process request").into_response();
        }
    };
    match send_recovery(&state, &user).await {
        Ok(()) => empty_ok(),
        Err(Stop::Error(error)) => error.into_response(),
        Err(Stop::NotPorted) => {
            MegabaseNotImplemented::new(COMPONENT, UNIT_RECOVER).into_response()
        }
    }
}

fn empty_ok() -> Response {
    (StatusCode::OK, Json(json!({}))).into_response()
}

/// `sendPasswordRecovery` for the implicit flow.
async fn send_recovery(state: &AuthState, user: &crate::store::UserRecord) -> Result<(), Stop> {
    let mail = &state.mail;
    let sent_at = state
        .backend
        .recovery_sent_at(user.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "recovery_sent_at lookup failed");
            unexpected(SEND_FAILED)
        })?;
    if let Some(message) = frequency_limited(sent_at, mail.max_frequency, SystemTime::now()) {
        return Err(over_email_limit(message).into());
    }
    let otp = generate_otp(state.otp_length).map_err(|_| unexpected(SEND_FAILED))?;
    let token_hash = email_otp_hash(&user.email, &otp);
    check_send_allowed(state, &user.email)?;
    deliver(mail).await?;
    state
        .backend
        .save_recovery(user, &token_hash, SystemTime::now())
        .await
        .map_err(|error| {
            tracing::error!(%error, "saving recovery token failed");
            unexpected(SEND_FAILED)
        })?;
    Ok(())
}

/// `validateSentWithinFrequencyLimit`: the message when another email to
/// this user is still too soon.
fn frequency_limited(
    sent_at: Option<SystemTime>,
    frequency: Duration,
    now: SystemTime,
) -> Option<String> {
    let next = sent_at? + frequency;
    let left = next
        .duration_since(now)
        .ok()
        .filter(|left| !left.is_zero())?;
    Some(format!(
        "For security purposes, you can only request this after {} seconds.",
        left.as_secs()
    ))
}

fn over_email_limit(message: impl Into<String>) -> GoTrueError {
    GoTrueError::new(
        StatusCode::TOO_MANY_REQUESTS,
        "over_email_send_rate_limit",
        message,
    )
}

/// The checks `sendEmail` runs before it hands the message to the client:
/// the authorized-address list and the email-send limiter.
fn check_send_allowed(state: &AuthState, recipient: &str) -> Result<(), GoTrueError> {
    let mail = &state.mail;
    if !recipient.is_empty() && !address_authorized(&mail.authorized_addresses, recipient) {
        return Err(GoTrueError::new(
            StatusCode::BAD_REQUEST,
            "email_address_not_authorized",
            format!(
                "Email address {} cannot be used as it is not authorized",
                go_quote(recipient)
            ),
        ));
    }
    if mail.email_limiter.rate().events() == 0.0 {
        return Err(over_email_limit("email rate limit exceeded"));
    }
    if !state.config.mailer_autoconfirm && !mail.email_limiter.allow() {
        return Err(over_email_limit("email rate limit exceeded"));
    }
    Ok(())
}

/// `checkEmailAddressAuthorization`. A `+label` before the `@` is ignored.
fn address_authorized(allowed: &[String], email: &str) -> bool {
    if allowed.is_empty() {
        return true;
    }
    let normalized = strip_labels(email);
    allowed
        .iter()
        .any(|address| normalized.eq_ignore_ascii_case(address))
}

/// `emailLabelPattern.ReplaceAllString(email, "@")` for `[+][^@]+@`.
fn strip_labels(email: &str) -> String {
    let mut out = String::with_capacity(email.len());
    let mut rest = email;
    while let Some(plus) = rest.find('+') {
        let after = &rest[plus + 1..];
        match after.find('@') {
            Some(at) if at > 0 => {
                out.push_str(&rest[..plus]);
                out.push('@');
                rest = &after[at + 1..];
            }
            _ => {
                out.push_str(&rest[..=plus]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Hand the recovery email to the configured mail client.
///
/// No SMTP host is GoTrue's `noopclient`, which succeeds. Otherwise the
/// first step of gomail's `DialAndSend` is a TCP dial with a ten-second
/// timeout; a failed dial is GoTrue's 500. A host that answers would need the
/// SMTP exchange, which is not ported.
async fn deliver(mail: &MailConfig) -> Result<(), Stop> {
    if mail.smtp_host.is_empty() {
        tracing::info!("no SMTP host configured; recovery email not sent (no-op mail client)");
        return Ok(());
    }
    let dial = TcpStream::connect((mail.smtp_host.as_str(), mail.smtp_port));
    match tokio::time::timeout(SMTP_DIAL_TIMEOUT, dial).await {
        Ok(Ok(_stream)) => Err(Stop::NotPorted),
        Ok(Err(error)) => {
            tracing::error!(%error, host = %mail.smtp_host, "SMTP dial failed");
            Err(unexpected(SEND_FAILED).into())
        }
        Err(_) => {
            tracing::error!(host = %mail.smtp_host, "SMTP dial timed out");
            Err(unexpected(SEND_FAILED).into())
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::Router;
    use megabase_core::Hs256;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::config::AuthConfig;
    use crate::store::Backend;

    const SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";

    fn app_with(smtp_host: &str, smtp_port: u16) -> (Router, Backend) {
        let backend = Backend::memory();
        let jwt = Hs256::new(SECRET.as_bytes()).unwrap();
        let mut state =
            AuthState::new(AuthConfig::reference_defaults(), Some(jwt), backend.clone());
        state.mail.smtp_host = smtp_host.into();
        state.mail.smtp_port = smtp_port;
        (crate::router_with_state(state), backend)
    }

    async fn call(
        app: &Router,
        method: &str,
        path: &str,
        body: &str,
        bearer: &str,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if !bearer.is_empty() {
            builder = builder.header("authorization", format!("Bearer {bearer}"));
        }
        let request = builder.body(Body::from(body.to_string())).unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn test_password() -> String {
        format!("pw-{}", uuid::Uuid::new_v4().simple())
    }

    async fn signup(app: &Router, email: &str, password: &str) {
        let body = json!({ "email": email, "password": password }).to_string();
        let (status, _) = call(app, "POST", "/auth/v1/signup", &body, "").await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn recover_validates_like_gotrue() {
        let (app, _) = app_with("", 0);
        let post = |body: &'static str| call(&app, "POST", "/auth/v1/recover", body, "");
        for body in ["{}", r#"{"email":null}"#] {
            let (status, json) = post(body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(json["error_code"], "validation_failed");
            assert_eq!(json["msg"], "Password recovery requires an email");
        }
        let (status, json) = post(r#"{"email":"nope"}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json["msg"],
            "Unable to validate email address: invalid format"
        );
        let (status, json) = post(r#"{"email":"a@example.com","code_challenge":"x"}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json["msg"],
            "PKCE flow requires code_challenge_method and code_challenge"
        );
        let (status, json) = post("{").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error_code"], "bad_json");

        let mut config = AuthConfig::reference_defaults();
        config.email_enabled = false;
        let off = crate::router_with_state(AuthState::new(config, None, Backend::memory()));
        let (status, json) = call(
            &off,
            "POST",
            "/auth/v1/recover",
            r#"{"email":"a@example.com"}"#,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error_code"], "email_provider_disabled");
    }

    #[tokio::test]
    async fn unknown_email_is_200_even_when_mail_would_fail() {
        // Port 1 on loopback refuses the dial; unknown users never reach it.
        let (app, _) = app_with("127.0.0.1", 1);
        let (status, json) = call(
            &app,
            "POST",
            "/auth/v1/recover",
            r#"{"email":"ghost@example.com"}"#,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json, json!({}));
    }

    #[tokio::test]
    async fn recover_verify_update_password_then_sign_in() {
        let (app, backend) = app_with("", 0);
        let old_password = test_password();
        signup(&app, "real@example.com", &old_password).await;
        let (status, json) = call(
            &app,
            "POST",
            "/auth/v1/recover",
            r#"{"email":"Real@Example.com"}"#,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json, json!({}));
        let hash = backend.recovery_token_for_test("real@example.com").await;
        assert_eq!(hash.len(), 56, "SHA-224 hex of email + OTP");

        // A second request inside `GOTRUE_SMTP_MAX_FREQUENCY` is refused.
        let (status, json) = call(
            &app,
            "POST",
            "/auth/v1/recover",
            r#"{"email":"real@example.com"}"#,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(json["error_code"], "over_email_send_rate_limit");
        let msg = json["msg"].as_str().unwrap();
        assert!(
            msg.starts_with("For security purposes, you can only request this after "),
            "{msg}"
        );
        assert_eq!(
            backend.recovery_token_for_test("real@example.com").await,
            hash
        );

        let verify = json!({ "type": "recovery", "token_hash": hash }).to_string();
        let (status, session) = call(&app, "POST", "/auth/v1/verify", &verify, "").await;
        assert_eq!(status, StatusCode::OK, "{session}");
        let access = session["access_token"].as_str().unwrap().to_string();
        let (status, json) = call(
            &app,
            "PUT",
            "/auth/v1/user",
            r#"{"password":"brand-new-1"}"#,
            &access,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");

        let login = |password: &str| {
            json!({ "email": "real@example.com", "password": password }).to_string()
        };
        let path = "/auth/v1/token?grant_type=password";
        let (status, _) = call(&app, "POST", path, &login("brand-new-1"), "").await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = call(&app, "POST", path, &login(&old_password), "").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        // The token is single-use.
        let (status, _) = call(&app, "POST", "/auth/v1/verify", &verify, "").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn unreachable_smtp_host_is_500_and_stores_nothing() {
        let (app, backend) = app_with("127.0.0.1", 1);
        signup(&app, "real@example.com", &test_password()).await;
        let (status, json) = call(
            &app,
            "POST",
            "/auth/v1/recover",
            r#"{"email":"real@example.com"}"#,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json["code"], 500);
        assert_eq!(json["error_code"], "unexpected_failure");
        assert_eq!(json["msg"], SEND_FAILED);
        assert_eq!(
            backend.recovery_token_for_test("real@example.com").await,
            ""
        );
    }

    #[tokio::test]
    async fn reachable_smtp_host_and_pkce_stay_501() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (app, backend) = app_with("127.0.0.1", port);
        signup(&app, "real@example.com", &test_password()).await;
        let (status, json) = call(
            &app,
            "POST",
            "/auth/v1/recover",
            r#"{"email":"real@example.com"}"#,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(json["unit"], UNIT_RECOVER);
        assert_eq!(
            backend.recovery_token_for_test("real@example.com").await,
            ""
        );
        let pkce = r#"{"email":"real@example.com","code_challenge":"abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG","code_challenge_method":"s256"}"#;
        let (status, json) = call(&app, "POST", "/auth/v1/recover", pkce, "").await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(json["unit"], UNIT_RECOVER);
    }

    #[tokio::test]
    async fn email_send_limits_and_authorized_addresses() {
        let mut state = AuthState::new(AuthConfig::reference_defaults(), None, Backend::memory());
        state.mail.email_limiter = std::sync::Arc::new(crate::ratelimit::EmailLimiter::new(
            crate::ratelimit::EmailRate::Interval { events: 0.0 },
        ));
        assert_eq!(
            check_send_allowed(&state, "a@example.com")
                .unwrap_err()
                .message,
            "email rate limit exceeded"
        );
        state.mail.email_limiter = std::sync::Arc::new(crate::ratelimit::EmailLimiter::new(
            crate::ratelimit::EmailRate::Interval { events: 1.0 },
        ));
        // Autoconfirm skips the limiter, as GoTrue does.
        assert!(check_send_allowed(&state, "a@example.com").is_ok());
        assert!(check_send_allowed(&state, "a@example.com").is_ok());
        state.config.mailer_autoconfirm = false;
        assert!(check_send_allowed(&state, "a@example.com").is_ok());
        let limited = check_send_allowed(&state, "a@example.com").unwrap_err();
        assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(limited.error_code, "over_email_send_rate_limit");

        state.config.mailer_autoconfirm = true;
        state.mail.authorized_addresses = vec!["ok@example.com".into()];
        let denied = check_send_allowed(&state, "no@example.com").unwrap_err();
        assert_eq!(denied.error_code, "email_address_not_authorized");
        assert_eq!(
            denied.message,
            "Email address \"no@example.com\" cannot be used as it is not authorized"
        );
        assert!(check_send_allowed(&state, "ok+tag@example.com").is_ok());
    }

    #[test]
    fn frequency_limit_reports_whole_seconds_left() {
        let now = SystemTime::now();
        let minute = Duration::from_secs(60);
        assert_eq!(frequency_limited(None, minute, now), None);
        let sent = now - Duration::from_millis(15_500);
        assert_eq!(
            frequency_limited(Some(sent), minute, now).as_deref(),
            Some("For security purposes, you can only request this after 44 seconds.")
        );
        let old = now - Duration::from_secs(61);
        assert_eq!(frequency_limited(Some(old), minute, now), None);
    }

    #[test]
    fn labels_are_ignored_for_authorized_addresses() {
        assert_eq!(strip_labels("a+tag@example.com"), "a@example.com");
        assert_eq!(strip_labels("a+@example.com"), "a+@example.com");
        assert_eq!(strip_labels("plain@example.com"), "plain@example.com");
        let allowed = vec!["A@Example.com".to_string()];
        assert!(address_authorized(&allowed, "a+x@example.com"));
        assert!(!address_authorized(&allowed, "b@example.com"));
        assert!(address_authorized(&[], "anyone@example.com"));
    }
}
