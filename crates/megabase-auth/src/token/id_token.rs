// Ported from supabase/auth internal/api/token_oidc.go (`IdTokenGrant`,
// `getProvider`), internal/api/provider/apple.go, and
// internal/api/provider/azure.go (MIT), pin v2.197.0.

//! `POST /auth/v1/token?grant_type=id_token`: sign in with an OIDC ID token
//! from a native provider SDK.
//!
//! Request validation, identity-link authentication, and provider and
//! issuer resolution are ported. Once a provider resolves as enabled, GoTrue
//! fetches the issuer's OIDC discovery document and JWKS and verifies the
//! token. Megabase makes no outbound OIDC requests yet, so that step is
//! HTTP 501. The reference stack enables no provider, so every request there
//! ends before it.

use axum::body::Body;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::alphabet::URL_SAFE;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use base64::Engine;
use megabase_core::MegabaseNotImplemented;
use serde::Deserialize;
use serde_json::Value;

use super::{grant_json, null_as_default};
use crate::error::{go_quote, unexpected, validation, GoTrueError, OAuthError};
use crate::gojson::{syntax_error, Pairs, ID_TOKEN_GRANT_PARAMS};
use crate::state::AuthState;
use crate::user::require_user;
use crate::COMPONENT;

const UNIT: &str = "auth:grant-type:id_token";

const APPLE_ISSUER: &str = "https://appleid.apple.com";
const OTHER_APPLE_ISSUER: &str = "https://account.apple.com";
const GOOGLE_ISSUER: &str = "https://accounts.google.com";
const FACEBOOK_ISSUER: &str = "https://www.facebook.com";
const KAKAO_ISSUER: &str = "https://kauth.kakao.com";
const VERCEL_MARKETPLACE_ISSUER: &str = "https://marketplace.vercel.com";
const SNAPCHAT_ISSUER: &str = "https://accounts.snapchat.com/accounts/oauth2/token";
const AZURE_PREFIX: &str = "https://login.microsoftonline.com/";
const AZURE_STS_PREFIX: &str = "https://sts.windows.net/";

/// Go `base64.RawURLEncoding`: no padding, and non-zero trailing bits are
/// accepted.
const RAW_URL: GeneralPurpose = GeneralPurpose::new(
    &URL_SAFE,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::RequireNone)
        .with_decode_allow_trailing_bits(true),
);

#[derive(Deserialize)]
struct IdTokenBody {
    #[serde(default, deserialize_with = "null_as_default")]
    id_token: String,
    #[serde(default, deserialize_with = "null_as_default")]
    provider: String,
    #[serde(default, deserialize_with = "null_as_default")]
    client_id: String,
    #[serde(default, deserialize_with = "null_as_default")]
    issuer: String,
    #[serde(default, deserialize_with = "null_as_default")]
    link_identity: bool,
}

// megabase:unit auth:grant-type:id_token
pub(super) async fn grant(state: AuthState, headers: HeaderMap, body: Body) -> Response {
    let params = match grant_json::<IdTokenBody>(body, &ID_TOKEN_GRANT_PARAMS).await {
        Ok(params) => params,
        Err(error) => return error.into_response(),
    };
    if params.id_token.is_empty() {
        return OAuthError::new("invalid request", "id_token required").into_response();
    }
    if params.provider.is_empty() && (params.client_id.is_empty() || params.issuer.is_empty()) {
        return OAuthError::new(
            "invalid request",
            "provider or client_id and issuer required",
        )
        .into_response();
    }
    if params.link_identity {
        if headers
            .get(AUTHORIZATION)
            .is_none_or(|value| value.is_empty())
        {
            return OAuthError::new(
                "invalid request",
                "Linking requires a valid user access token in Authorization",
            )
            .into_response();
        }
        if let Err(response) = require_user(&state, &headers).await {
            return *response;
        }
    }
    if let Err(error) = resolve_provider(&state, &params).await {
        return error.into_response();
    }
    // Enabled provider: OIDC discovery, JWKS, and ID token verification.
    MegabaseNotImplemented::new(COMPONENT, UNIT).into_response()
}

/// `getProvider` up to the OIDC discovery fetch: `Ok` when the provider
/// resolves and is enabled, otherwise GoTrue's error response.
async fn resolve_provider(state: &AuthState, params: &IdTokenBody) -> Result<(), GoTrueError> {
    let external = &state.config.external;
    let id_token = &state.config.id_token;
    let provider = params.provider.as_str();
    let requested = params.issuer.as_str();
    let (enabled, issuer) = if provider == "apple" || is_apple_issuer(requested) {
        let detected = detect_issuer(&params.id_token)
            .ok_or_else(|| invalid("Unable to detect issuer in ID token for Apple provider"))?;
        if !is_apple_issuer(&detected) {
            return Err(invalid(
                "Detected ID token issuer is not an Apple ID token issuer",
            ));
        }
        if !requested.is_empty() && requested != detected {
            return Err(invalid("Provided issuer does not match ID token issuer"));
        }
        (external.apple, detected)
    } else if provider == "google" || requested == GOOGLE_ISSUER {
        (external.google, GOOGLE_ISSUER.to_string())
    } else if provider == "azure" || is_azure_issuer(requested) {
        let detected = detect_issuer(&params.id_token)
            .ok_or_else(|| invalid("Unable to detect issuer in ID token for Azure provider"))?;
        if !detected.starts_with(AZURE_PREFIX) && !detected.starts_with(AZURE_STS_PREFIX) {
            return Err(invalid(
                "Detected ID token issuer is not an Azure ID token issuer",
            ));
        }
        if !requested.is_empty() && requested != detected {
            return Err(invalid("Provided issuer does not match ID token issuer"));
        }
        (external.azure, detected)
    } else if provider == "facebook" || requested == FACEBOOK_ISSUER {
        (external.facebook, FACEBOOK_ISSUER.to_string())
    } else if provider == "keycloak"
        || (external.keycloak
            && !id_token.keycloak_url.is_empty()
            && requested == id_token.keycloak_url)
    {
        (external.keycloak, id_token.keycloak_url.clone())
    } else if provider == "kakao" || requested == KAKAO_ISSUER {
        (external.kakao, KAKAO_ISSUER.to_string())
    } else if provider == "vercel_marketplace" || requested == VERCEL_MARKETPLACE_ISSUER {
        (
            id_token.vercel_marketplace,
            VERCEL_MARKETPLACE_ISSUER.to_string(),
        )
    } else if provider == "snapchat" || requested == SNAPCHAT_ISSUER {
        (external.snapchat, SNAPCHAT_ISSUER.to_string())
    } else if provider.starts_with("custom:") {
        return custom_provider(state, provider).await;
    } else if id_token
        .allowed_issuers
        .iter()
        .any(|allowed| allowed == requested)
    {
        // Deprecated arbitrary issuer with the caller's client id.
        (true, requested.to_string())
    } else {
        return Err(invalid(format!(
            "Custom OIDC provider {} not allowed",
            go_quote(provider)
        )));
    };
    if !enabled {
        return Err(GoTrueError::new(
            StatusCode::BAD_REQUEST,
            "provider_disabled",
            format!("Provider (issuer {}) is not enabled", go_quote(&issuer)),
        ));
    }
    Ok(())
}

/// The `custom:` branch of `getProvider`.
async fn custom_provider(state: &AuthState, provider: &str) -> Result<(), GoTrueError> {
    if !state.custom_oauth_enabled {
        return Err(invalid("Custom OAuth providers are disabled"));
    }
    let row = match state.backend.find_custom_provider(provider).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            return Err(invalid(format!(
                "Custom provider {} not found",
                go_quote(provider)
            )));
        }
        Err(error) => {
            tracing::error!(%error, "custom provider lookup failed");
            return Err(unexpected("Error finding custom provider"));
        }
    };
    if !row.enabled {
        return Err(GoTrueError::new(
            StatusCode::BAD_REQUEST,
            "provider_disabled",
            format!("Custom provider {} is disabled", go_quote(provider)),
        ));
    }
    if row.provider_type != "oidc" {
        return Err(invalid(format!(
            "Provider {} is not an OIDC provider",
            go_quote(provider)
        )));
    }
    if row.issuer.is_none() {
        return Err(invalid(format!(
            "OIDC provider {} missing issuer",
            go_quote(provider)
        )));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> GoTrueError {
    validation(message)
}

fn is_apple_issuer(issuer: &str) -> bool {
    issuer == APPLE_ISSUER || issuer == OTHER_APPLE_ISSUER
}

/// `^https://login[.]microsoftonline[.]com/([^/]+)/v2[.]0/?$`
fn is_azure_issuer(issuer: &str) -> bool {
    let Some(rest) = issuer.strip_prefix(AZURE_PREFIX) else {
        return false;
    };
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    rest.strip_suffix("/v2.0")
        .is_some_and(|tenant| !tenant.is_empty() && !tenant.contains('/'))
}

/// Go `base64.RawURLEncoding.DecodeString`, which skips CR and LF.
pub(super) fn raw_url_decode(input: &str) -> Option<Vec<u8>> {
    let cleaned: String = input
        .chars()
        .filter(|c| !matches!(c, '\r' | '\n'))
        .collect();
    RAW_URL.decode(cleaned).ok()
}

/// `DetectAppleIDTokenIssuer` / `DetectAzureIDTokenIssuer`: the unverified
/// `iss` claim. `None` is the Go error (not three segments, bad base64, or a
/// payload `json.Unmarshal` rejects).
fn detect_issuer(id_token: &str) -> Option<String> {
    let parts: Vec<&str> = id_token.split('.').collect();
    let [_, payload, _] = parts.as_slice() else {
        return None;
    };
    let bytes = raw_url_decode(payload)?;
    if syntax_error(&bytes).is_some() {
        return None;
    }
    // Go replaces invalid UTF-8 inside strings with U+FFFD.
    let text = String::from_utf8_lossy(&bytes);
    let Ok(Pairs(pairs)) = serde_json::from_str::<Pairs>(&text) else {
        // A top-level `null` leaves the struct empty; any other non-object
        // is an `UnmarshalTypeError`.
        return match serde_json::from_str::<Value>(&text).ok()? {
            Value::Null => Some(String::new()),
            _ => None,
        };
    };
    let mut issuer = String::new();
    for (key, value) in pairs {
        if !folds_to_iss(&key) {
            continue;
        }
        match value {
            Value::String(value) => issuer = value,
            Value::Null => {}
            _ => return None,
        }
    }
    Some(issuer)
}

/// encoding/json field matching: case-insensitive, with the Unicode simple
/// folds of `s` (U+017F) and `k` (U+212A).
fn folds_to_iss(key: &str) -> bool {
    let folded: String = key
        .chars()
        .map(|c| match c {
            '\u{17f}' => 's',
            '\u{212a}' => 'k',
            c => c,
        })
        .collect();
    folded.eq_ignore_ascii_case("iss")
}

#[cfg(test)]
mod tests {
    use axum::http::Request;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use serde_json::json;
    use tower::ServiceExt;

    use super::*;
    use crate::config::AuthConfig;
    use crate::routes::router;
    use crate::store::Backend;

    fn token(payload: &str) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(payload))
    }

    async fn post_with(
        state: AuthState,
        body: &Value,
        authorization: Option<&str>,
    ) -> (StatusCode, Value, HeaderMap) {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/auth/v1/token?grant_type=id_token")
            .header("content-type", "application/json");
        if let Some(value) = authorization {
            builder = builder.header("authorization", value);
        }
        let request = builder.body(Body::from(body.to_string())).unwrap();
        let response = router(state).oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            headers,
        )
    }

    fn reference() -> AuthState {
        AuthState::new(AuthConfig::reference_defaults(), None, Backend::memory())
    }

    async fn post(body: Value) -> (StatusCode, Value) {
        let (status, body, _) = post_with(reference(), &body, None).await;
        (status, body)
    }

    #[tokio::test]
    async fn request_validation_is_an_oauth_error() {
        let (status, body, headers) = post_with(reference(), &json!({}), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            json!({ "error": "invalid request", "error_description": "id_token required" })
        );
        assert!(headers.get("x-sb-error-code").is_none());

        let (_, body) = post(json!({ "id_token": "t", "client_id": "c" })).await;
        assert_eq!(
            body["error_description"],
            "provider or client_id and issuer required"
        );

        let (_, body) =
            post(json!({ "id_token": "t", "provider": "google", "link_identity": true })).await;
        assert_eq!(
            body["error_description"],
            "Linking requires a valid user access token in Authorization"
        );

        let (status, body) = post(json!({ "id_token": 1 })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "bad_json");
    }

    #[tokio::test]
    async fn linking_runs_require_authentication() {
        let body = json!({ "id_token": "t", "provider": "google", "link_identity": true });
        let (status, body, _) = post_with(reference(), &body, Some("Bearer nope")).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["msg"], "Server lacks JWT secret");
    }

    #[tokio::test]
    async fn disabled_providers_on_the_reference_stack() {
        let (status, body) = post(json!({ "id_token": "t", "provider": "google" })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "provider_disabled");
        assert_eq!(
            body["msg"],
            "Provider (issuer \"https://accounts.google.com\") is not enabled"
        );

        let (_, body) = post(json!({
            "id_token": "t",
            "client_id": "c",
            "issuer": "https://www.facebook.com",
        }))
        .await;
        assert_eq!(
            body["msg"],
            "Provider (issuer \"https://www.facebook.com\") is not enabled"
        );

        let (_, body) = post(json!({ "id_token": "t", "provider": "keycloak" })).await;
        assert_eq!(body["msg"], "Provider (issuer \"\") is not enabled");

        for (provider, issuer) in [
            ("kakao", KAKAO_ISSUER),
            ("vercel_marketplace", VERCEL_MARKETPLACE_ISSUER),
            ("snapchat", SNAPCHAT_ISSUER),
        ] {
            let (_, body) = post(json!({ "id_token": "t", "provider": provider })).await;
            assert_eq!(
                body["msg"],
                format!("Provider (issuer \"{issuer}\") is not enabled")
            );
        }
    }

    #[tokio::test]
    async fn apple_and_azure_read_the_unverified_issuer() {
        let (_, body) = post(json!({ "id_token": "not-a-jwt", "provider": "apple" })).await;
        assert_eq!(body["error_code"], "validation_failed");
        assert_eq!(
            body["msg"],
            "Unable to detect issuer in ID token for Apple provider"
        );

        let other = token(r#"{"iss":"https://evil.example"}"#);
        let (_, body) = post(json!({ "id_token": other, "provider": "apple" })).await;
        assert_eq!(
            body["msg"],
            "Detected ID token issuer is not an Apple ID token issuer"
        );

        let apple = token(r#"{"ISS":"https://account.apple.com"}"#);
        let (_, body) = post(json!({
            "id_token": apple,
            "client_id": "c",
            "issuer": "https://appleid.apple.com",
        }))
        .await;
        assert_eq!(
            body["msg"],
            "Provided issuer does not match ID token issuer"
        );

        let (_, body) = post(json!({ "id_token": apple, "provider": "apple" })).await;
        assert_eq!(body["error_code"], "provider_disabled");
        assert_eq!(
            body["msg"],
            "Provider (issuer \"https://account.apple.com\") is not enabled"
        );

        let azure = token(r#"{"iss":"https://sts.windows.net/tenant/"}"#);
        let (_, body) = post(json!({ "id_token": azure, "provider": "azure" })).await;
        assert_eq!(
            body["msg"],
            "Provider (issuer \"https://sts.windows.net/tenant/\") is not enabled"
        );

        let (_, body) = post(json!({
            "id_token": token(r#"{"iss":1}"#),
            "client_id": "c",
            "issuer": "https://login.microsoftonline.com/common/v2.0",
        }))
        .await;
        assert_eq!(
            body["msg"],
            "Unable to detect issuer in ID token for Azure provider"
        );

        let (_, body) = post(json!({ "id_token": token("null"), "provider": "azure" })).await;
        assert_eq!(
            body["msg"],
            "Detected ID token issuer is not an Azure ID token issuer"
        );
    }

    #[tokio::test]
    async fn custom_and_arbitrary_issuers() {
        let (_, body) = post(json!({ "id_token": "t", "provider": "custom:acme" })).await;
        assert_eq!(body["error_code"], "validation_failed");
        assert_eq!(body["msg"], "Custom provider \"custom:acme\" not found");

        let mut off = reference();
        off.custom_oauth_enabled = false;
        let body = json!({ "id_token": "t", "provider": "custom:acme" });
        let (_, body, _) = post_with(off, &body, None).await;
        assert_eq!(body["msg"], "Custom OAuth providers are disabled");

        let none = AuthState::new(AuthConfig::reference_defaults(), None, Backend::none());
        let body = json!({ "id_token": "t", "provider": "custom:acme" });
        let (status, body, _) = post_with(none, &body, None).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["msg"], "Error finding custom provider");

        let (_, body) = post(json!({
            "id_token": "t",
            "client_id": "c",
            "issuer": "https://issuer.example",
        }))
        .await;
        assert_eq!(body["msg"], "Custom OIDC provider \"\" not allowed");

        let mut config = AuthConfig::reference_defaults();
        config.id_token.allowed_issuers = vec!["https://issuer.example".into()];
        let allowed = AuthState::new(config, None, Backend::memory());
        let body = json!({ "id_token": "t", "client_id": "c", "issuer": "https://issuer.example" });
        let (status, body, _) = post_with(allowed, &body, None).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], UNIT);
    }

    #[tokio::test]
    async fn enabled_provider_is_not_implemented() {
        let mut config = AuthConfig::reference_defaults();
        config.external.google = true;
        let state = AuthState::new(config, None, Backend::memory());
        let body = json!({ "id_token": "t", "provider": "google" });
        let (status, body, _) = post_with(state, &body, None).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], UNIT);
    }

    #[test]
    fn issuer_helpers() {
        assert!(is_azure_issuer(
            "https://login.microsoftonline.com/common/v2.0/"
        ));
        assert!(!is_azure_issuer("https://login.microsoftonline.com//v2.0"));
        assert!(!is_azure_issuer(
            "https://login.microsoftonline.com/a/b/v2.0"
        ));
        assert_eq!(
            detect_issuer(&token(r#"{"iss":"a","iſs":"b"}"#)).as_deref(),
            Some("b")
        );
        assert_eq!(
            detect_issuer(&token(r#"{"iss":"a","iss":null}"#)).as_deref(),
            Some("a")
        );
        assert_eq!(detect_issuer(&token("[]")), None);
        assert_eq!(detect_issuer(&token("{")), None);
        assert_eq!(detect_issuer("a.b.c.d"), None);
        assert_eq!(detect_issuer("a.!!.c"), None);
    }
}
