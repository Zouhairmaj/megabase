// Ported from supabase/auth internal/api/reauthenticate.go,
// internal/api/mail.go (sendReauthenticationOtp, sendEmail), and
// internal/api/auth.go (requireAuthentication) (MIT), pin v2.197.0.

//! `GET /reauthenticate`: email a fresh OTP to the signed-in user.
//!
//! The route requires the same bearer session as `GET /user`. A user with
//! an email must already be confirmed; a phone-only user must have a
//! confirmed phone. The email goes through the same mail client as
//! `POST /recover`. The SMS sender is not ported, so a confirmed phone-only
//! user stays HTTP 501. OTP generation fails the request when the system
//! random source cannot be read.

use std::time::SystemTime;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use megabase_core::MegabaseNotImplemented;

use crate::admin_batch2::generate_otp;
use crate::error::{unexpected, validation, GoTrueError};
use crate::recover::{check_send_allowed, deliver, frequency_limited, Stop};
use crate::state::AuthState;
use crate::store::{email_otp_hash, UserRecord};
use crate::user::require_user;
use crate::COMPONENT;

const UNIT: &str = "auth:route:GET /auth/v1/reauthenticate";
const SEND_FAILED: &str = "Error sending reauthentication email";

// megabase:unit auth:route:GET /auth/v1/reauthenticate
pub(crate) async fn reauthenticate(State(state): State<AuthState>, headers: HeaderMap) -> Response {
    let authed = match require_user(&state, &headers).await {
        Ok(authed) => authed,
        Err(response) => return *response,
    };
    let email = authed.user.email.as_str();
    let phone = authed.user.phone.as_str();
    if email.is_empty() && phone.is_empty() {
        return validation("Reauthentication requires the user to have an email or a phone number")
            .into_response();
    }
    if !email.is_empty() {
        if authed.user.email_confirmed_at.is_none() {
            return GoTrueError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "email_not_confirmed",
                "Please verify your email first.",
            )
            .into_response();
        }
    } else if authed.user.phone_confirmed_at.is_none() {
        return GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "phone_not_confirmed",
            "Please verify your phone first.",
        )
        .into_response();
    }
    if email.is_empty() {
        return MegabaseNotImplemented::new(COMPONENT, UNIT).into_response();
    }
    match send_reauthentication(&state, &authed.user).await {
        Ok(()) => crate::recover::empty_ok(),
        Err(Stop::Error(error)) => error.into_response(),
        Err(Stop::NotPorted) => MegabaseNotImplemented::new(COMPONENT, UNIT).into_response(),
    }
}

async fn send_reauthentication(state: &AuthState, user: &UserRecord) -> Result<(), Stop> {
    let pending = state.backend.pending_mail(user.id).await.map_err(|error| {
        tracing::error!(%error, "reauthentication mail state lookup failed");
        unexpected(SEND_FAILED)
    })?;
    if let Some(message) = frequency_limited(
        pending.reauthentication_sent_at,
        state.mail.max_frequency,
        SystemTime::now(),
    ) {
        return Err(crate::recover::over_email_limit(message).into());
    }
    // A failed read of the random source fails the request. Nothing is stored.
    let otp = generate_otp(state.otp_length).map_err(|_| unexpected(SEND_FAILED))?;
    let token_hash = email_otp_hash(&user.email, &otp);
    check_send_allowed(state, &user.email)?;
    deliver(&state.mail, SEND_FAILED).await?;
    state
        .backend
        .save_reauthentication(user, &token_hash, SystemTime::now())
        .await
        .map_err(|error| {
            tracing::error!(%error, "saving reauthentication token failed");
            unexpected(SEND_FAILED)
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::body::{to_bytes, Body};
    use axum::http::{header::ALLOW, Request};
    use axum::Router;
    use megabase_core::Hs256;
    use serde_json::{json, Value};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::config::AuthConfig;
    use crate::store::Backend;

    const SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";

    fn app() -> (Router, Backend, Hs256) {
        let backend = Backend::memory();
        let jwt = Hs256::new(SECRET.as_bytes()).unwrap();
        let mut state = AuthState::new(
            AuthConfig::reference_defaults(),
            Some(jwt.clone()),
            backend.clone(),
        );
        state.mail.smtp_host.clear();
        (crate::router_with_state(state), backend, jwt)
    }

    fn token_for(jwt: &Hs256, user_id: Uuid) -> String {
        let exp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        jwt.sign(&json!({
            "sub": user_id.to_string(),
            "role": "authenticated",
            "aud": "authenticated",
            "exp": exp,
        }))
        .unwrap()
        .as_str()
        .to_string()
    }

    async fn call(app: &Router, method: &str, path: &str, bearer: &str) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(path);
        if !bearer.is_empty() {
            builder = builder.header("authorization", format!("Bearer {bearer}"));
        }
        let response = app
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let allow = response
            .headers()
            .get(ALLOW)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let mut body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if status == StatusCode::METHOD_NOT_ALLOWED {
            body = json!({ "allow": allow });
        }
        (status, body)
    }

    async fn signup(app: &Router, email: &str) -> String {
        let request = Request::builder()
            .method("POST")
            .uri("/auth/v1/signup")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "email": email, "password": "secret-pass-1" }).to_string(),
            ))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        body["access_token"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn missing_bearer_is_401_and_other_methods_are_405() {
        let (app, _, _) = app();
        let (status, body) = call(&app, "GET", "/auth/v1/reauthenticate", "").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error_code"], "no_authorization");
        let (status, body) = call(&app, "POST", "/auth/v1/reauthenticate", "").await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(body["allow"], "GET");
    }

    #[tokio::test]
    async fn confirmed_email_stores_a_token_and_the_next_request_is_limited() {
        let (app, backend, _) = app();
        let access = signup(&app, "ada@example.com").await;
        let (status, body) = call(&app, "GET", "/auth/v1/reauthenticate", &access).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body, json!({}));
        let hash = backend
            .reauthentication_token_for_test("ada@example.com")
            .await;
        assert_eq!(hash.len(), 56);
        let (status, body) = call(&app, "GET", "/auth/v1/reauthenticate", &access).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body["error_code"], "over_email_send_rate_limit");
        assert_eq!(
            backend
                .reauthentication_token_for_test("ada@example.com")
                .await,
            hash
        );
    }

    #[tokio::test]
    async fn unconfirmed_email_and_phone_follow_gotrue_before_any_send() {
        let (app, backend, jwt) = app();
        let mut config = AuthConfig::reference_defaults();
        config.mailer_autoconfirm = false;
        let mut state = AuthState::new(config, Some(jwt.clone()), backend.clone());
        state.mail.smtp_host.clear();
        let off = crate::router_with_state(state);
        let request = Request::builder()
            .method("POST")
            .uri("/auth/v1/signup")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "email": "new@example.com", "password": "secret-pass-1" }).to_string(),
            ))
            .unwrap();
        let response = off.clone().oneshot(request).await.unwrap();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let created: Value = serde_json::from_slice(&bytes).unwrap();
        let user_id = Uuid::parse_str(created["id"].as_str().unwrap()).unwrap();
        let access = token_for(&jwt, user_id);
        let (status, body) = call(&off, "GET", "/auth/v1/reauthenticate", &access).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error_code"], "email_not_confirmed");
        assert_eq!(body["msg"], "Please verify your email first.");

        let bare = backend.plant_contact_for_test("", "", false, false).await;
        let (status, body) = call(
            &app,
            "GET",
            "/auth/v1/reauthenticate",
            &token_for(&jwt, bare),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["msg"],
            "Reauthentication requires the user to have an email or a phone number"
        );

        let phone = backend
            .plant_contact_for_test("", "15550001111", false, false)
            .await;
        let (status, body) = call(
            &app,
            "GET",
            "/auth/v1/reauthenticate",
            &token_for(&jwt, phone),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error_code"], "phone_not_confirmed");

        let confirmed = backend
            .plant_contact_for_test("", "15550002222", false, true)
            .await;
        let (status, body) = call(
            &app,
            "GET",
            "/auth/v1/reauthenticate",
            &token_for(&jwt, confirmed),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], UNIT);
    }

    #[tokio::test]
    async fn unreachable_smtp_stores_nothing() {
        let backend = Backend::memory();
        let jwt = Hs256::new(SECRET.as_bytes()).unwrap();
        let mut state =
            AuthState::new(AuthConfig::reference_defaults(), Some(jwt), backend.clone());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        state.mail.smtp_host = "127.0.0.1".into();
        state.mail.smtp_port = listener.local_addr().unwrap().port();
        drop(listener);
        let app = crate::router_with_state(state);
        let access = signup(&app, "ada@example.com").await;
        let (status, body) = call(&app, "GET", "/auth/v1/reauthenticate", &access).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["msg"], SEND_FAILED);
        assert_eq!(
            backend
                .reauthentication_token_for_test("ada@example.com")
                .await,
            ""
        );
    }
}
