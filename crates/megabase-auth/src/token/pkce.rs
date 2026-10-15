// Ported from supabase/auth internal/api/token.go (`PKCE`),
// internal/models/flow_state.go, internal/models/factor.go, and
// internal/security/pkce.go (MIT), pin v2.197.0.

//! `POST /auth/v1/token?grant_type=pkce`: exchange an auth code and its
//! code verifier for a session.

use std::time::SystemTime;

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{grant_json, null_as_default, session_response, store_failure, Extra};
use crate::error::{validation, GoTrueError};
use crate::gojson::PKCE_GRANT_PARAMS;
use crate::hooks::TokenHookCall;
use crate::jsonutil::unix_secs;
use crate::state::AuthState;
use crate::store::{FlowStateRecord, PkceRedemption};

const INVALID_CODE_CHALLENGE: &str = "code challenge does not match previously saved code verifier";
const INVALID_CODE_METHOD: &str = "code challenge method not supported";
const NOT_PKCE: &str = "PKCE verification not applicable for implicit flow";

#[derive(Deserialize)]
struct PkceBody {
    #[serde(default, deserialize_with = "null_as_default")]
    auth_code: String,
    #[serde(default, deserialize_with = "null_as_default")]
    code_verifier: String,
}

// No `auth:grant-type:pkce` marker yet: no ported route writes an
// `auth.flow_state` row with an `auth_code`, so a client cannot obtain a
// redeemable code (see `specs/auth/token.md`).
pub(super) async fn grant(state: AuthState, headers: HeaderMap, body: Body) -> Response {
    let params = match grant_json::<PkceBody>(body, &PKCE_GRANT_PARAMS).await {
        Ok(params) => params,
        Err(error) => return error.into_response(),
    };
    if params.auth_code.is_empty() || params.code_verifier.is_empty() {
        return validation("invalid request: both auth code and code verifier should be non-empty")
            .into_response();
    }
    let (flow_id, flow) = match state
        .backend
        .find_flow_state_by_auth_code(&params.auth_code)
        .await
    {
        Ok(Some(found)) => found,
        Ok(None) => return flow_state_not_found(),
        Err(error) => return store_failure(error),
    };
    let Some(user_id) = flow.user_id else {
        return flow_state_not_found();
    };
    if flow.is_expired(SystemTime::now()) {
        return GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "flow_state_expired",
            "invalid flow state, flow state has expired",
        )
        .into_response();
    }
    // `FindUserByID` returns a bare not-found error, which GoTrue's default
    // handler reports as 500.
    match state.backend.load_user(user_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return unhandled(),
        Err(error) => return store_failure(error),
    }
    if let Err(message) = verify_pkce(&flow, &params.code_verifier) {
        return GoTrueError::new(StatusCode::BAD_REQUEST, "bad_code_verifier", message)
            .into_response();
    }
    let Some(amr_method) = authentication_method(&flow.authentication_method) else {
        return unhandled();
    };
    let now = unix_secs(SystemTime::now());
    let hook = TokenHookCall::new(
        state.config.custom_access_token_hook.as_ref(),
        &headers,
        amr_method,
        &state.config.jwt_issuer,
        now,
        state.config.jwt_exp_seconds,
    );
    let redemption = PkceRedemption {
        flow_id,
        user_id,
        amr_method,
        provider_type: &flow.provider_type,
    };
    let issued = match state
        .backend
        .redeem_pkce_flow(&redemption, hook.as_ref())
        .await
    {
        Ok(Some(issued)) => issued,
        // A concurrent request redeemed the code first.
        Ok(None) => return flow_state_not_found(),
        Err(error) => return store_failure(error),
    };
    let extra = Extra::Provider {
        access: flow.provider_access_token,
        refresh: flow.provider_refresh_token,
    };
    session_response(&state, &issued, now, false, extra)
}

fn flow_state_not_found() -> Response {
    GoTrueError::new(
        StatusCode::NOT_FOUND,
        "flow_state_not_found",
        "invalid flow state, no valid flow state found",
    )
    .into_response()
}

/// GoTrue's `handleResponseErrorDefault` for an error that is not an
/// `HTTPError`.
fn unhandled() -> Response {
    GoTrueError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "unexpected_failure",
        "Unexpected failure, please check server logs for more information",
    )
    .into_response()
}

/// `FlowState.VerifyPKCE` and `security.VerifyPKCEChallenge`.
fn verify_pkce(flow: &FlowStateRecord, verifier: &str) -> Result<(), &'static str> {
    if !flow.is_pkce() {
        return Err(NOT_PKCE);
    }
    let challenge = flow.code_challenge.as_deref().unwrap_or_default();
    let method = flow
        .code_challenge_method
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let expected = match method.as_str() {
        "s256" => URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        "plain" => verifier.to_string(),
        _ => return Err(INVALID_CODE_METHOD),
    };
    if constant_time_eq(challenge.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(INVALID_CODE_CHALLENGE)
    }
}

/// Go `subtle.ConstantTimeCompare`: unequal lengths fail at once, equal
/// lengths compare every byte.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

/// `ParseAuthenticationMethod` followed by `String()`: the AMR method the
/// session records. Any value ending in `signup` is `email/signup`.
fn authentication_method(raw: &str) -> Option<&'static str> {
    if raw.ends_with("signup") {
        return Some("email/signup");
    }
    Some(match raw {
        "oauth" => "oauth",
        "password" => "password",
        "otp" => "otp",
        "totp" => "totp",
        "recovery" => "recovery",
        "invite" => "invite",
        "sso/saml" => "sso/saml",
        "magiclink" => "magiclink",
        "email_change" => "email_change",
        "token_refresh" => "token_refresh",
        "mfa/phone" => "mfa/phone",
        "mfa/webauthn" => "mfa/webauthn",
        "mfa/recovery_code" => "mfa/recovery_code",
        "web3" => "web3",
        "oauth_provider/authorization_code" => "oauth_provider/authorization_code",
        "passkey" => "passkey",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::http::Request;
    use serde_json::{json, Value};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::config::AuthConfig;
    use crate::routes::router;
    use crate::store::Backend;
    use megabase_core::Hs256;

    const SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";
    /// RFC 7636 appendix B, as in upstream `internal/security/pkce_test.go`.
    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    fn app(backend: Backend) -> AuthState {
        let jwt = Hs256::new(SECRET.as_bytes()).unwrap();
        AuthState::new(AuthConfig::reference_defaults(), Some(jwt), backend)
    }

    async fn post(state: &AuthState, body: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .method("POST")
            .uri("/auth/v1/token?grant_type=pkce")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = router(state.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn flow(user_id: Option<Uuid>, code: &str) -> FlowStateRecord {
        FlowStateRecord {
            provider_type: "email".into(),
            auth_code: Some(code.into()),
            code_challenge: Some(CHALLENGE.into()),
            code_challenge_method: Some("s256".into()),
            user_id,
            authentication_method: "email/signup".into(),
            provider_access_token: String::new(),
            provider_refresh_token: String::new(),
            created_at: Some(SystemTime::now()),
            auth_code_issued_at: None,
            referrer: None,
            linking_target_id: None,
        }
    }

    async fn user(state: &AuthState) -> Uuid {
        let request = Request::builder()
            .method("POST")
            .uri("/auth/v1/signup")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "email": "pkce@example.com", "password": VERIFIER }).to_string(),
            ))
            .unwrap();
        let response = router(state.clone()).oneshot(request).await.unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        body["user"]["id"].as_str().unwrap().parse().unwrap()
    }

    #[tokio::test]
    async fn exchanges_a_code_once() {
        let backend = Backend::memory();
        let state = app(backend.clone());
        let user_id = user(&state).await;
        let mut planted = flow(Some(user_id), "code-1");
        planted.provider_access_token = "provider-access".into();
        backend
            .plant_flow_state_for_test(Uuid::new_v4(), planted)
            .await;
        let body = json!({ "auth_code": "code-1", "code_verifier": VERIFIER }).to_string();
        let (status, session) = post(&state, &body).await;
        assert_eq!(status, StatusCode::OK, "{session}");
        assert_eq!(session["token_type"], "bearer");
        assert_eq!(session["user"]["id"], user_id.to_string());
        assert_eq!(session["provider_token"], "provider-access");
        assert!(session.get("provider_refresh_token").is_none());
        assert!(session.get("weak_password").is_none());
        assert_eq!(session["refresh_token"].as_str().unwrap().len(), 12);
        let token = session["access_token"].as_str().unwrap();
        let payload = token.split('.').nth(1).unwrap();
        let claims: Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
        assert_eq!(claims["amr"][0]["method"], "email/signup");

        let (status, again) = post(&state, &body).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(again["error_code"], "flow_state_not_found");
        assert_eq!(
            again["msg"],
            "invalid flow state, no valid flow state found"
        );
    }

    #[tokio::test]
    async fn concurrent_redemptions_issue_one_session() {
        let backend = Backend::memory();
        let state = app(backend.clone());
        let user_id = user(&state).await;
        let flow_id = Uuid::new_v4();
        backend
            .plant_flow_state_for_test(flow_id, flow(Some(user_id), "race"))
            .await;
        // Both requests pass the lookup before either redeems.
        let first = backend.find_flow_state_by_auth_code("race").await.unwrap();
        let second = backend.find_flow_state_by_auth_code("race").await.unwrap();
        assert!(first.is_some() && second.is_some());
        let redemption = PkceRedemption {
            flow_id,
            user_id,
            amr_method: "email/signup",
            provider_type: "email",
        };
        let (a, b) = tokio::join!(
            backend.redeem_pkce_flow(&redemption, None),
            backend.redeem_pkce_flow(&redemption, None),
        );
        let issued = [a.unwrap(), b.unwrap()];
        assert_eq!(issued.iter().filter(|s| s.is_some()).count(), 1);
        let body = json!({ "auth_code": "race", "code_verifier": VERIFIER }).to_string();
        let (status, again) = post(&state, &body).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(again["error_code"], "flow_state_not_found");
    }

    #[tokio::test]
    async fn rejected_hook_keeps_the_code_redeemable() {
        let backend = Backend::memory();
        let mut config = AuthConfig::reference_defaults();
        config.custom_access_token_hook =
            crate::hooks::AccessTokenHook::from_lookup(&|key| match key {
                "GOTRUE_HOOK_CUSTOM_ACCESS_TOKEN_ENABLED" => Some("true".into()),
                "GOTRUE_HOOK_CUSTOM_ACCESS_TOKEN_URI" => {
                    Some("pg-functions://postgres/public/custom_access_token_hook".into())
                }
                _ => None,
            })
            .unwrap();
        let jwt = Hs256::new(SECRET.as_bytes()).unwrap();
        let state = AuthState::new(config, Some(jwt), backend.clone());
        // The hook accepts the signup session and rejects every later one.
        let calls = std::sync::atomic::AtomicUsize::new(0);
        backend
            .set_access_token_hook_for_test(std::sync::Arc::new(move |event: &Value| {
                let first = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
                first.then(|| json!({ "claims": event["claims"] }).to_string())
            }))
            .await;
        let user_id = user(&state).await;
        backend
            .plant_flow_state_for_test(Uuid::new_v4(), flow(Some(user_id), "hooked"))
            .await;
        let body = json!({ "auth_code": "hooked", "code_verifier": VERIFIER }).to_string();
        let (status, _) = post(&state, &body).await;
        assert!(status.is_server_error(), "{status}");
        assert!(backend
            .find_flow_state_by_auth_code("hooked")
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn validation_and_lookup_errors() {
        let backend = Backend::memory();
        let state = app(backend.clone());
        let (status, body) = post(&state, r#"{"auth_code":"x"}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "validation_failed");
        assert_eq!(
            body["msg"],
            "invalid request: both auth code and code verifier should be non-empty"
        );

        let (status, body) = post(&state, r#"{"auth_code":1}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["msg"],
            "Could not parse request body as JSON: json: cannot unmarshal number into Go struct field PKCEGrantParams.auth_code of type string"
        );

        let missing = r#"{"auth_code":"missing","code_verifier":"v"}"#;
        let (status, body) = post(&state, missing).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error_code"], "flow_state_not_found");

        backend
            .plant_flow_state_for_test(Uuid::new_v4(), flow(None, "no-user"))
            .await;
        let (status, _) = post(&state, r#"{"auth_code":"no-user","code_verifier":"v"}"#).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let mut old = flow(Some(Uuid::new_v4()), "old");
        old.created_at = Some(SystemTime::now() - Duration::from_secs(301));
        backend.plant_flow_state_for_test(Uuid::new_v4(), old).await;
        let (status, body) = post(&state, r#"{"auth_code":"old","code_verifier":"v"}"#).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error_code"], "flow_state_expired");
        assert_eq!(body["msg"], "invalid flow state, flow state has expired");

        backend
            .plant_flow_state_for_test(Uuid::new_v4(), flow(Some(Uuid::new_v4()), "ghost"))
            .await;
        let (status, body) = post(&state, r#"{"auth_code":"ghost","code_verifier":"v"}"#).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error_code"], "unexpected_failure");
    }

    #[tokio::test]
    async fn wrong_verifier_is_bad_code_verifier() {
        let backend = Backend::memory();
        let state = app(backend.clone());
        let user_id = user(&state).await;
        backend
            .plant_flow_state_for_test(Uuid::new_v4(), flow(Some(user_id), "code-2"))
            .await;
        let (status, body) = post(&state, r#"{"auth_code":"code-2","code_verifier":"nope"}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "bad_code_verifier");
        assert_eq!(body["msg"], INVALID_CODE_CHALLENGE);
    }

    #[test]
    fn verifier_methods() {
        let mut planted = flow(None, "c");
        assert!(verify_pkce(&planted, VERIFIER).is_ok());
        planted.code_challenge_method = Some("PLAIN".into());
        assert_eq!(verify_pkce(&planted, VERIFIER), Err(INVALID_CODE_CHALLENGE));
        assert!(verify_pkce(&planted, CHALLENGE).is_ok());
        planted.code_challenge_method = Some("s512".into());
        assert_eq!(verify_pkce(&planted, CHALLENGE), Err(INVALID_CODE_METHOD));
        planted.code_challenge = Some(String::new());
        assert_eq!(verify_pkce(&planted, CHALLENGE), Err(NOT_PKCE));
    }

    #[test]
    fn authentication_methods_follow_parse_then_string() {
        assert_eq!(authentication_method("magiclink"), Some("magiclink"));
        assert_eq!(authentication_method("phone/signup"), Some("email/signup"));
        assert_eq!(authentication_method("anonymous"), None);
        assert_eq!(authentication_method("nope"), None);
    }
}
