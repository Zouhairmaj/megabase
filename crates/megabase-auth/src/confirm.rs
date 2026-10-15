// Ported from supabase/auth internal/api/signup.go (Signup, sanitizeUser),
// internal/api/mail.go (sendConfirmation, sendEmail), and
// internal/api/pkce.go (generateFlowState, addFlowPrefixToToken) (MIT),
// pin v2.197.0.

//! `POST /signup` for an email address with `GOTRUE_MAILER_AUTOCONFIRM=false`.
//!
//! A new address becomes an unconfirmed user and gets a confirmation email;
//! an unconfirmed one gets a fresh email and keeps its password and data. A
//! confirmed address answers 200 with an obfuscated user, so the response
//! does not reveal that the account exists. The email goes through the same
//! mail client as `POST /recover`. The PKCE flow (`code_challenge`) also
//! writes an `email/signup` flow state and stores the token with a `pkce_`
//! prefix, so `GET /verify?type=signup` redirects with an auth code that
//! `grant_type=pkce` redeems.

use std::time::SystemTime;

use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use megabase_core::MegabaseNotImplemented;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::admin_batch2::generate_otp;
use crate::error::unexpected;
use crate::http::json_ok;
use crate::jsonutil::{format_ts, user_json};
use crate::recover::{check_send_allowed, deliver, frequency_limited, over_email_limit, Stop};
use crate::routes::request_aud;
use crate::state::AuthState;
use crate::store::{
    email_otp_hash, hash_new_password, ConfirmationOutcome, ConfirmationSignup, FlowStateRecord,
    UserRecord,
};
use crate::token::unhandled;
use crate::COMPONENT;

const UNIT_SIGNUP: &str = "auth:route:POST /auth/v1/signup";
const SEND_FAILED: &str = "Error sending confirmation email";
const SAVE_FAILED: &str = "Database error saving new user";

/// `models.EmailSignup.String()`: the flow state's authentication method.
const EMAIL_SIGNUP: &str = "email/signup";

/// A validated email signup that GoTrue does not autoconfirm.
pub(crate) struct UnconfirmedSignup {
    /// Lowercased, validated email.
    pub email: String,
    pub password: String,
    pub data: Map<String, Value>,
    pub code_challenge: String,
    pub code_challenge_method: String,
}

/// The `Signup` branch for `Mailer.Autoconfirm == false`.
pub(crate) async fn signup_unconfirmed(
    state: &AuthState,
    headers: &HeaderMap,
    params: UnconfirmedSignup,
) -> Response {
    match run(state, headers, &params).await {
        Ok(response) => response,
        Err(Stop::Error(error)) => error.into_response(),
        Err(Stop::NotPorted) => MegabaseNotImplemented::new(COMPONENT, UNIT_SIGNUP).into_response(),
    }
}

async fn run(
    state: &AuthState,
    headers: &HeaderMap,
    params: &UnconfirmedSignup,
) -> Result<Response, Stop> {
    let aud = request_aud(headers, &state.config);
    let existing = state
        .backend
        .signup_confirmation_lookup(&params.email, &aud)
        .await
        .map_err(|error| {
            tracing::error!(%error, "signup user lookup failed");
            unexpected("Database error finding user")
        })?;
    let mut cmd = ConfirmationSignup {
        email: &params.email,
        aud: &aud,
        role: &state.config.jwt_default_group,
        data: &params.data,
        password_hash: None,
        token_hash: "",
        flow: None,
        sent_at: SystemTime::now(),
    };
    if existing.as_ref().is_some_and(|found| found.confirmed) {
        // `UserExistsError`: only the audit entry, no email.
        return save(state, cmd, params).await;
    }
    if existing.is_none() {
        // `ToUserModel` hashes outside the transaction.
        let hash = hash_new_password(params.password.clone())
            .await
            .map_err(|error| {
                tracing::error!(%error, "hashing signup password failed");
                unexpected(SAVE_FAILED)
            })?;
        cmd.password_hash = Some(hash);
    }
    let flow = if params.code_challenge.is_empty() {
        None
    } else {
        // `ParseCodeChallengeMethod`; its plain error is GoTrue's default 500,
        // and the transaction (user included) rolls back.
        let method = params.code_challenge_method.to_ascii_lowercase();
        if method != "s256" && method != "plain" {
            return Ok(unhandled());
        }
        Some(FlowStateRecord {
            provider_type: "email".into(),
            auth_code: Some(Uuid::new_v4().to_string()),
            code_challenge: Some(params.code_challenge.clone()),
            code_challenge_method: Some(method),
            user_id: None,
            authentication_method: EMAIL_SIGNUP.into(),
            provider_access_token: String::new(),
            provider_refresh_token: String::new(),
            created_at: None,
            auth_code_issued_at: None,
            referrer: None,
            linking_target_id: None,
        })
    };
    cmd.flow = flow.as_ref();
    let sent_at = existing.and_then(|found| found.confirmation_sent_at);
    if let Some(message) = frequency_limited(sent_at, state.mail.max_frequency, SystemTime::now()) {
        return Err(over_email_limit(message).into());
    }
    let otp = generate_otp(state.otp_length).map_err(|_| unexpected(SEND_FAILED))?;
    let mut token_hash = email_otp_hash(&params.email, &otp);
    if cmd.flow.is_some() {
        token_hash.insert_str(0, "pkce_");
    }
    check_send_allowed(state, &params.email)?;
    deliver(&state.mail, SEND_FAILED).await?;
    cmd.token_hash = &token_hash;
    cmd.sent_at = SystemTime::now();
    save(state, cmd, params).await
}

async fn save(
    state: &AuthState,
    cmd: ConfirmationSignup<'_>,
    params: &UnconfirmedSignup,
) -> Result<Response, Stop> {
    let sent_at = cmd.sent_at;
    let outcome = state
        .backend
        .save_signup_confirmation(cmd)
        .await
        .map_err(|error| {
            tracing::error!(%error, "saving unconfirmed signup failed");
            unexpected(SAVE_FAILED)
        })?;
    Ok(match outcome {
        ConfirmationOutcome::Sent { user, invited_at } => {
            json_ok(&sent_json(&user, sent_at, invited_at))
        }
        ConfirmationOutcome::Repeated(user) => json_ok(&sanitized_json(&user, params, sent_at)),
    })
}

/// The user `Signup` returns after `sendConfirmation`. An invited user's
/// metadata and identities are blanked (`HasBeenInvited`).
fn sent_json(user: &UserRecord, sent_at: SystemTime, invited_at: Option<SystemTime>) -> Value {
    let mut body = user_json(user);
    if let Value::Object(map) = &mut body {
        if let Some(invited_at) = invited_at {
            map.insert("invited_at".into(), json!(format_ts(invited_at)));
            map.insert("user_metadata".into(), json!({}));
            map.insert("identities".into(), json!([]));
        }
        map.insert("confirmation_sent_at".into(), json!(format_ts(sent_at)));
    }
    body
}

/// `sanitizeUser`: a confirmed user's signup answers like a new one, with a
/// random id, the request's `data`, and no timestamps or identities.
fn sanitized_json(user: &UserRecord, params: &UnconfirmedSignup, now: SystemTime) -> Value {
    let mut shown = user.clone();
    shown.id = Uuid::new_v4();
    shown.role = String::new();
    shown.phone = String::new();
    shown.created_at = now;
    shown.updated_at = now;
    shown.last_sign_in_at = None;
    shown.confirmed_at = None;
    shown.email_confirmed_at = None;
    shown.phone_confirmed_at = None;
    shown.identities = Vec::new();
    shown.user_metadata = Value::Object(params.data.clone());
    shown.app_metadata = json!({ "provider": "email", "providers": ["email"] });
    let mut body = user_json(&shown);
    if let Value::Object(map) = &mut body {
        map.insert("confirmation_sent_at".into(), json!(format_ts(now)));
    }
    body
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use megabase_core::Hs256;
    use tower::ServiceExt;

    use super::*;
    use crate::config::AuthConfig;
    use crate::jsonutil::unix_secs;
    use crate::store::Backend;

    const SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";
    /// RFC 7636 appendix B, as in upstream `internal/security/pkce_test.go`.
    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    fn jwt() -> Hs256 {
        Hs256::new(SECRET.as_bytes()).unwrap()
    }

    /// Autoconfirm off, no SMTP host: GoTrue's no-op mail client.
    fn app() -> (Router, Backend) {
        app_on(Backend::memory(), None)
    }

    fn app_on(backend: Backend, max_frequency: Option<std::time::Duration>) -> (Router, Backend) {
        let mut config = AuthConfig::reference_defaults();
        config.mailer_autoconfirm = false;
        let mut state = AuthState::new(config, Some(jwt()), backend.clone());
        state.mail.smtp_host = String::new();
        if let Some(frequency) = max_frequency {
            state.mail.max_frequency = frequency;
        }
        (crate::router_with_state(state), backend)
    }

    async fn call(app: &Router, method: &str, path: &str, body: &str) -> (StatusCode, Value) {
        call_as(app, method, path, body, "").await
    }

    async fn call_as(
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

    async fn location(app: &Router, path: &str) -> (StatusCode, String) {
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
    async fn unconfirmed_signup_sends_confirmation_then_verify_signs_in() {
        let (app, backend) = app();
        let body = json!({
            "email": "New@Example.com",
            "password": "secret-pass-1",
            "data": { "name": "Ada" },
        })
        .to_string();
        let (status, user) = call(&app, "POST", "/auth/v1/signup", &body).await;
        assert_eq!(status, StatusCode::OK, "{user}");
        assert!(user.get("access_token").is_none(), "no session: {user}");
        assert_eq!(user["email"], "new@example.com");
        assert_eq!(user["role"], "authenticated");
        assert!(user.get("email_confirmed_at").is_none());
        assert!(user["confirmation_sent_at"].is_string());
        assert_eq!(
            user["user_metadata"],
            json!({
                "sub": user["id"],
                "email": "new@example.com",
                "email_verified": false,
                "phone_verified": false,
                "name": "Ada",
            })
        );
        assert_eq!(user["identities"][0]["provider"], "email");
        let hash = backend.confirmation_token_for_test("new@example.com").await;
        assert_eq!(hash.len(), 56, "SHA-224 hex of email + OTP");

        // `GOTRUE_SMTP_MAX_FREQUENCY` applies to the second email.
        let (status, json) = call(&app, "POST", "/auth/v1/signup", &body).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(json["error_code"], "over_email_send_rate_limit");
        assert_eq!(
            backend.confirmation_token_for_test("new@example.com").await,
            hash
        );

        let verify = json!({ "type": "signup", "token_hash": hash }).to_string();
        let (status, session) = call(&app, "POST", "/auth/v1/verify", &verify).await;
        assert_eq!(status, StatusCode::OK, "{session}");
        assert_eq!(session["user"]["id"], user["id"]);
        assert!(session["user"]["email_confirmed_at"].is_string());

        // A confirmed address answers like a new signup (`sanitizeUser`).
        let again = json!({ "email": "new@example.com", "password": "other-pass-1" });
        let (status, shown) = call(&app, "POST", "/auth/v1/signup", &again.to_string()).await;
        assert_eq!(status, StatusCode::OK, "{shown}");
        assert_ne!(shown["id"], user["id"]);
        assert_eq!(shown["role"], "");
        assert_eq!(shown["identities"], json!([]));
        assert_eq!(shown["user_metadata"], json!({}));
        assert!(shown.get("email_confirmed_at").is_none());
        assert!(shown["confirmation_sent_at"].is_string());

        // The stored password is the first one.
        let login = |password: &str| {
            json!({ "email": "new@example.com", "password": password }).to_string()
        };
        let path = "/auth/v1/token?grant_type=password";
        let (status, _) = call(&app, "POST", path, &login("secret-pass-1")).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = call(&app, "POST", path, &login("other-pass-1")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn pkce_signup_verify_then_exchange_the_code() {
        let (app, backend) = app();
        let body = json!({
            "email": "pkce@example.com",
            "password": "secret-pass-1",
            "code_challenge": CHALLENGE,
            "code_challenge_method": "S256",
        })
        .to_string();
        let (status, user) = call(&app, "POST", "/auth/v1/signup", &body).await;
        assert_eq!(status, StatusCode::OK, "{user}");
        let token = backend
            .confirmation_token_for_test("pkce@example.com")
            .await;
        assert!(token.starts_with("pkce_"), "{token}");

        let link = format!("/auth/v1/verify?type=signup&token={token}");
        let (status, location) = location(&app, &link).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let code = location
            .strip_prefix("http://localhost:3000?code=")
            .unwrap_or_else(|| panic!("{location}"))
            .to_string();

        let exchange = json!({ "auth_code": code, "code_verifier": VERIFIER }).to_string();
        let path = "/auth/v1/token?grant_type=pkce";
        let (status, session) = call(&app, "POST", path, &exchange).await;
        assert_eq!(status, StatusCode::OK, "{session}");
        assert_eq!(session["user"]["id"], user["id"]);
        assert!(session["user"]["email_confirmed_at"].is_string());
        let (status, json) = call(&app, "POST", path, &exchange).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["error_code"], "flow_state_not_found");
    }

    #[tokio::test]
    async fn bad_challenge_method_and_mail_failure_store_nothing() {
        let (app, backend) = app();
        let body = json!({
            "email": "bad@example.com",
            "password": "secret-pass-1",
            "code_challenge": CHALLENGE,
            "code_challenge_method": "sha1",
        })
        .to_string();
        let (status, json) = call(&app, "POST", "/auth/v1/signup", &body).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json["error_code"], "unexpected_failure");
        assert!(backend
            .password_hash_for_test("bad@example.com")
            .await
            .is_none());

        // A reachable SMTP host needs the SMTP exchange, which is not ported.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let backend = Backend::memory();
        let mut config = AuthConfig::reference_defaults();
        config.mailer_autoconfirm = false;
        let mut state = AuthState::new(config, Some(jwt()), backend.clone());
        state.mail.smtp_host = "127.0.0.1".into();
        state.mail.smtp_port = listener.local_addr().unwrap().port();
        let app = crate::router_with_state(state);
        let body = json!({ "email": "smtp@example.com", "password": "secret-pass-1" });
        let (status, json) = call(&app, "POST", "/auth/v1/signup", &body.to_string()).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(json["unit"], UNIT_SIGNUP);
        assert!(backend
            .password_hash_for_test("smtp@example.com")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn invited_user_signup_hides_metadata_and_keeps_no_password() {
        let (app, backend) = app();
        let exp = unix_secs(SystemTime::now()) + 600;
        let admin = jwt()
            .sign(&json!({ "role": "service_role", "exp": exp }))
            .unwrap()
            .to_string();
        let invite = r#"{"email":"guest@example.com","data":{"team":"a"}}"#;
        let (status, invited) = call_as(&app, "POST", "/auth/v1/invite", invite, &admin).await;
        assert_eq!(status, StatusCode::OK, "{invited}");
        let body = json!({ "email": "guest@example.com", "password": "secret-pass-1" });
        // The invite email set `confirmation_sent_at`, so the frequency
        // limit applies to the confirmation email.
        let (status, json) = call(&app, "POST", "/auth/v1/signup", &body.to_string()).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{json}");
        let (app, _) = app_on(backend.clone(), Some(std::time::Duration::ZERO));
        let (status, user) = call(&app, "POST", "/auth/v1/signup", &body.to_string()).await;
        assert_eq!(status, StatusCode::OK, "{user}");
        assert_eq!(user["id"], invited["id"]);
        assert_eq!(user["user_metadata"], json!({}));
        assert_eq!(user["identities"], json!([]));
        assert!(user["invited_at"].is_string());
        // An unconfirmed user is not updated: no password is stored.
        assert_eq!(
            backend.password_hash_for_test("guest@example.com").await,
            Some(String::new())
        );
    }
}
