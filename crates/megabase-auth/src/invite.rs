// Ported from supabase/auth internal/api/invite.go (Invite) and
// internal/api/mail.go (sendInvite, sendEmail) (MIT), pin v2.197.0.

//! `POST /invite`: an admin invites a user by email.
//!
//! Requires an admin JWT (`requireAdminCredentials`). A new address becomes an
//! unconfirmed user with no password, `data` as `user_metadata`, and an email
//! identity. An unconfirmed user is re-invited; a confirmed one is 422
//! `email_exists`. The invite email goes through the same mail client as
//! `POST /recover`: no SMTP host is the no-op client, an unreachable host is
//! 500 and nothing is stored, and a reachable host stays 501 because the SMTP
//! exchange is not ported. The stored `confirmation_token` is redeemed by
//! `POST /verify` with `type=invite`.

use std::time::SystemTime;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use megabase_core::MegabaseNotImplemented;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::admin::verified_admin_role;
use crate::admin_batch2::generate_otp;
use crate::error::{unexpected, GoTrueError};
use crate::gojson;
use crate::http::json_ok;
use crate::jsonutil::{format_ts, user_json};
use crate::recover::{check_send_allowed, deliver, Stop};
use crate::routes::{read_body, request_aud, validate_email};
use crate::state::AuthState;
use crate::store::{email_otp_hash, InviteCommand, InviteOutcome, LoginChannel};
use crate::token::null_as_default;
use crate::COMPONENT;

const UNIT_INVITE: &str = "auth:route:POST /auth/v1/invite";

/// `DuplicateEmailMsg` in `internal/api/errors.go`.
const DUPLICATE_EMAIL: &str = "A user with this email address has already been registered";
const SEND_FAILED: &str = "Error sending invite email";

/// `InviteParams`. Unknown keys are ignored and `null` reads as absent.
#[derive(Deserialize, Default)]
struct InviteBody {
    #[serde(default, deserialize_with = "null_as_default")]
    email: String,
    #[serde(default)]
    data: Option<Map<String, Value>>,
}

// megabase:unit auth:route:POST /auth/v1/invite
pub(crate) async fn invite(
    State(state): State<AuthState>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let admin_role = match verified_admin_role(&state, &headers) {
        Ok(role) => role,
        Err(error) => return error.into_response(),
    };
    match run(&state, &headers, &admin_role, body).await {
        Ok(response) => response,
        Err(Stop::Error(error)) => error.into_response(),
        Err(Stop::NotPorted) => MegabaseNotImplemented::new(COMPONENT, UNIT_INVITE).into_response(),
    }
}

async fn run(
    state: &AuthState,
    headers: &HeaderMap,
    admin_role: &str,
    body: Body,
) -> Result<Response, Stop> {
    let bytes = read_body(body).await?;
    // Go decodes a top-level `null` into zero-valued params.
    let params: InviteBody = if bytes.trim_ascii() == b"null" {
        InviteBody::default()
    } else {
        gojson::decode(&bytes, &gojson::INVITE_PARAMS)?
    };
    let email = validate_email(&params.email)?;
    let aud = request_aud(headers, &state.config);
    let existing = state
        .backend
        .find_login_user(LoginChannel::Email, &email, &aud)
        .await
        .map_err(|error| {
            tracing::error!(%error, "invite user lookup failed");
            unexpected("Database error finding user")
        })?;
    if existing.is_some_and(|user| user.email_confirmed_at.is_some()) {
        return Err(email_exists().into());
    }
    let otp = generate_otp(state.otp_length).map_err(|_| unexpected(SEND_FAILED))?;
    let token_hash = email_otp_hash(&email, &otp);
    check_send_allowed(state, &email)?;
    deliver(&state.mail, SEND_FAILED).await?;
    let sent_at = SystemTime::now();
    let outcome = state
        .backend
        .invite_user(InviteCommand {
            email: &email,
            aud: &aud,
            role: &state.config.jwt_default_group,
            data: params.data.unwrap_or_default(),
            token_hash: &token_hash,
            admin_role,
            sent_at,
        })
        .await
        .map_err(|error| {
            tracing::error!(%error, "saving invite failed");
            unexpected("Error inviting user")
        })?;
    match outcome {
        InviteOutcome::Invited(user) => Ok(json_ok(&invited_json(&user, sent_at))),
        InviteOutcome::EmailExists => Err(email_exists().into()),
    }
}

fn email_exists() -> GoTrueError {
    GoTrueError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "email_exists",
        DUPLICATE_EMAIL,
    )
}

/// The user model `Invite` returns: `sendInvite` set `invited_at` and
/// `confirmation_sent_at` on it after the last load.
fn invited_json(user: &crate::store::UserRecord, sent_at: SystemTime) -> Value {
    let mut body = user_json(user);
    if let Value::Object(map) = &mut body {
        map.insert("invited_at".into(), json!(format_ts(sent_at)));
        map.insert("confirmation_sent_at".into(), json!(format_ts(sent_at)));
    }
    body
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::Router;
    use megabase_core::Hs256;
    use tower::ServiceExt;

    use super::*;
    use crate::config::AuthConfig;
    use crate::jsonutil::unix_secs;
    use crate::store::Backend;

    const SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";

    fn jwt() -> Hs256 {
        Hs256::new(SECRET.as_bytes()).unwrap()
    }

    fn token(role: &str) -> String {
        let exp = unix_secs(SystemTime::now()) + 600;
        jwt()
            .sign(&json!({ "role": role, "exp": exp }))
            .unwrap()
            .to_string()
    }

    fn app_with(smtp_host: &str, smtp_port: u16) -> (Router, Backend) {
        let backend = Backend::memory();
        let mut state = AuthState::new(
            AuthConfig::reference_defaults(),
            Some(jwt()),
            backend.clone(),
        );
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

    async fn invite_as(app: &Router, body: &str) -> (StatusCode, Value) {
        call(app, "POST", "/auth/v1/invite", body, &token("service_role")).await
    }

    #[tokio::test]
    async fn invite_requires_admin_credentials() {
        let (app, backend) = app_with("", 0);
        let body = r#"{"email":"new@example.com"}"#;
        let (status, json) = call(&app, "POST", "/auth/v1/invite", body, "").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(json["error_code"], "no_authorization");
        let (status, json) = call(&app, "POST", "/auth/v1/invite", body, &token("anon")).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(json["error_code"], "not_admin");
        let (status, json) = call(&app, "POST", "/auth/v1/invite", body, "not-a-jwt").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(json["error_code"], "bad_jwt");
        assert_eq!(backend.invite_token_for_test("new@example.com").await.0, "");
    }

    #[tokio::test]
    async fn invite_validates_like_gotrue() {
        let (app, _) = app_with("", 0);
        for body in ["{}", "null", r#"{"email":null}"#] {
            let (status, json) = invite_as(&app, body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(json["error_code"], "validation_failed");
            assert_eq!(json["msg"], "An email address is required");
        }
        let (status, json) = invite_as(&app, r#"{"email":"nope"}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json["msg"],
            "Unable to validate email address: invalid format"
        );
        let (status, json) = invite_as(&app, "{").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error_code"], "bad_json");
        let (status, json) = invite_as(&app, r#"{"email":"a@example.com","data":"x"}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error_code"], "bad_json");
    }

    #[tokio::test]
    async fn invite_creates_unconfirmed_user_with_metadata() {
        let (app, backend) = app_with("", 0);
        let (status, user) =
            invite_as(&app, r#"{"email":"New@Example.com","data":{"name":"Ada"}}"#).await;
        assert_eq!(status, StatusCode::OK, "{user}");
        assert_eq!(user["email"], "new@example.com");
        assert_eq!(user["aud"], "authenticated");
        assert_eq!(user["role"], "authenticated");
        assert_eq!(user["phone"], "");
        assert_eq!(user["user_metadata"], json!({ "name": "Ada" }));
        assert_eq!(
            user["app_metadata"],
            json!({ "provider": "email", "providers": ["email"] })
        );
        assert!(user["invited_at"].is_string());
        assert_eq!(user["invited_at"], user["confirmation_sent_at"]);
        assert!(user.get("email_confirmed_at").is_none());
        assert!(user.get("confirmed_at").is_none());
        assert_eq!(user["is_anonymous"], false);
        let identity = &user["identities"][0];
        assert_eq!(identity["provider"], "email");
        assert_eq!(identity["id"], user["id"]);
        assert_eq!(
            identity["identity_data"],
            json!({
                "sub": user["id"],
                "email": "new@example.com",
                "email_verified": false,
                "phone_verified": false,
            })
        );
        let (hash, invited_at) = backend.invite_token_for_test("new@example.com").await;
        assert_eq!(hash.len(), 56, "SHA-224 hex of email + OTP");
        assert!(invited_at.is_some());
    }

    #[tokio::test]
    async fn reinvite_unconfirmed_resends_and_confirmed_is_422() {
        let (app, backend) = app_with("", 0);
        let (status, first) = invite_as(&app, r#"{"email":"again@example.com"}"#).await;
        assert_eq!(status, StatusCode::OK);
        let (first_hash, _) = backend.invite_token_for_test("again@example.com").await;
        // `data` only seeds a new user.
        let body = r#"{"email":"again@example.com","data":{"ignored":true}}"#;
        let (status, second) = invite_as(&app, body).await;
        assert_eq!(status, StatusCode::OK, "{second}");
        assert_eq!(second["id"], first["id"]);
        assert_eq!(second["user_metadata"], json!({}));
        let (second_hash, _) = backend.invite_token_for_test("again@example.com").await;
        assert_ne!(
            first_hash, second_hash,
            "a fresh token replaces the old one"
        );

        let signup = json!({ "email": "taken@example.com", "password": "secret-pass-1" });
        let (status, _) = call(&app, "POST", "/auth/v1/signup", &signup.to_string(), "").await;
        assert_eq!(status, StatusCode::OK);
        let (status, json) = invite_as(&app, r#"{"email":"taken@example.com"}"#).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(json["error_code"], "email_exists");
        assert_eq!(json["msg"], DUPLICATE_EMAIL);
    }

    #[tokio::test]
    async fn invite_verify_then_set_password() {
        let (app, backend) = app_with("", 0);
        let (status, _) = invite_as(&app, r#"{"email":"join@example.com"}"#).await;
        assert_eq!(status, StatusCode::OK);
        let (hash, _) = backend.invite_token_for_test("join@example.com").await;
        let verify = json!({ "type": "invite", "token_hash": hash }).to_string();
        let (status, session) = call(&app, "POST", "/auth/v1/verify", &verify, "").await;
        assert_eq!(status, StatusCode::OK, "{session}");
        assert!(session["user"]["email_confirmed_at"].is_string());
        let access = session["access_token"].as_str().unwrap().to_string();
        let (status, json) = call(
            &app,
            "PUT",
            "/auth/v1/user",
            r#"{"password":"chosen-pass-1"}"#,
            &access,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let login = json!({ "email": "join@example.com", "password": "chosen-pass-1" });
        let path = "/auth/v1/token?grant_type=password";
        let (status, _) = call(&app, "POST", path, &login.to_string(), "").await;
        assert_eq!(status, StatusCode::OK);
        // Confirmed now, so a second invite is refused.
        let (status, _) = invite_as(&app, r#"{"email":"join@example.com"}"#).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn mail_limits_and_delivery_failures_store_nothing() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let (app, backend) = app_with("127.0.0.1", closed);
        let (status, json) = invite_as(&app, r#"{"email":"lost@example.com"}"#).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json["msg"], SEND_FAILED);
        assert_eq!(
            backend.invite_token_for_test("lost@example.com").await.0,
            ""
        );
        assert!(backend
            .find_login_user(LoginChannel::Email, "lost@example.com", "authenticated")
            .await
            .unwrap()
            .is_none());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (app, _) = app_with("127.0.0.1", port);
        let (status, json) = invite_as(&app, r#"{"email":"smtp@example.com"}"#).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(json["unit"], UNIT_INVITE);

        let backend = Backend::memory();
        let mut state = AuthState::new(
            AuthConfig::reference_defaults(),
            Some(jwt()),
            backend.clone(),
        );
        state.mail.smtp_host = String::new();
        state.mail.authorized_addresses = vec!["ok@example.com".into()];
        let app = crate::router_with_state(state);
        let (status, json) = invite_as(&app, r#"{"email":"no@example.com"}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error_code"], "email_address_not_authorized");
        let (status, _) = invite_as(&app, r#"{"email":"ok@example.com"}"#).await;
        assert_eq!(status, StatusCode::OK);
    }
}
