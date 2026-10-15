// Ported from supabase/auth internal/api/jwks.go, internal/api/api.go,
// internal/api/middleware.go and internal/models/oauth_scope.go (MIT), pin v2.197.0.

//! JWKS and OIDC / OAuth 2.0 authorization server discovery.
//!
//! `GET /auth/v1/.well-known/jwks.json` lists the public signing keys. With
//! `JWT_SECRET` as the only key (the reference stack) GoTrue hides the HMAC
//! key, so the set is empty. `GOTRUE_JWT_KEYS` is not ported and answers 501.
//! `GET /auth/v1/.well-known/openid-configuration` and
//! `GET /.well-known/oauth-authorization-server` share one document; the
//! second is 404 `feature_disabled` while the OAuth server is off. Spec:
//! `specs/auth/well-known.md`.

use axum::extract::State;
use axum::http::{
    header::{CACHE_CONTROL, CONTENT_TYPE},
    HeaderValue, StatusCode,
};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use megabase_core::MegabaseNotImplemented;
use serde::Serialize;

use crate::http::AuthError;
use crate::routes::get_only;
use crate::state::AuthState;
use crate::COMPONENT;

const UNIT_JWKS: &str = "auth:route:GET /auth/v1/.well-known/jwks.json";

/// `models.SupportedOAuthScopes`.
const SUPPORTED_OAUTH_SCOPES: &[&str] = &["openid", "profile", "email", "phone", "offline_access"];

const CLAIMS_SUPPORTED: &[&str] = &[
    "sub",
    "aud",
    "iss",
    "exp",
    "iat",
    "auth_time",
    "nonce",
    "email",
    "email_verified",
    "phone_number",
    "phone_number_verified",
    "name",
    "picture",
    "preferred_username",
    "updated_at",
];

pub(crate) fn router(state: AuthState) -> Router {
    Router::new()
        .route("/auth/v1/.well-known/jwks.json", get_only(get(jwks)))
        .route(
            "/auth/v1/.well-known/openid-configuration",
            get_only(get(openid_configuration)),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get_only(get(oauth_authorization_server)),
        )
        .with_state(state)
}

#[derive(Serialize)]
struct Jwks {
    keys: [(); 0],
}

// megabase:unit auth:route:GET /auth/v1/.well-known/jwks.json
async fn jwks(State(state): State<AuthState>) -> Response {
    if state.jwt_keys_configured {
        return MegabaseNotImplemented::new(COMPONENT, UNIT_JWKS).into_response();
    }
    cached_json(&Jwks { keys: [] })
}

/// `OpenIDConfigurationResponse`, fields in Go declaration order.
#[derive(Serialize)]
struct OpenIdConfiguration<'a> {
    issuer: &'a str,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    userinfo_endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    registration_endpoint: Option<String>,
    scopes_supported: &'static [&'static str],
    response_types_supported: &'static [&'static str],
    response_modes_supported: &'static [&'static str],
    grant_types_supported: &'static [&'static str],
    subject_types_supported: &'static [&'static str],
    id_token_signing_alg_values_supported: &'static [&'static str],
    token_endpoint_auth_methods_supported: &'static [&'static str],
    claims_supported: &'static [&'static str],
    code_challenge_methods_supported: &'static [&'static str],
}

impl<'a> OpenIdConfiguration<'a> {
    fn new(state: &'a AuthState) -> Self {
        let issuer = state.config.jwt_issuer.as_str();
        let base = issuer.trim_end_matches('/');
        Self {
            issuer,
            authorization_endpoint: format!("{base}/oauth/authorize"),
            token_endpoint: format!("{base}/oauth/token"),
            jwks_uri: format!("{base}/.well-known/jwks.json"),
            userinfo_endpoint: format!("{base}/oauth/userinfo"),
            registration_endpoint: (state.oauth_server_enabled && state.oauth_dynamic_registration)
                .then(|| format!("{base}/oauth/clients/register")),
            scopes_supported: SUPPORTED_OAUTH_SCOPES,
            response_types_supported: &["code"],
            response_modes_supported: &["query"],
            grant_types_supported: &["authorization_code", "refresh_token"],
            subject_types_supported: &["public"],
            id_token_signing_alg_values_supported: &["RS256", "HS256", "ES256"],
            token_endpoint_auth_methods_supported: &[
                "client_secret_basic",
                "client_secret_post",
                "none",
            ],
            claims_supported: CLAIMS_SUPPORTED,
            code_challenge_methods_supported: &["S256", "plain"],
        }
    }
}

// megabase:unit auth:route:GET /auth/v1/.well-known/openid-configuration
async fn openid_configuration(State(state): State<AuthState>) -> Response {
    cached_json(&OpenIdConfiguration::new(&state))
}

// megabase:unit auth:route:GET /.well-known/oauth-authorization-server
async fn oauth_authorization_server(State(state): State<AuthState>) -> Response {
    // `requireOAuthServerEnabled`.
    if !state.oauth_server_enabled {
        return AuthError::feature_disabled("OAuth server is disabled").into_response();
    }
    cached_json(&OpenIdConfiguration::new(&state))
}

/// `sendJSON` with `Cache-Control: public, max-age=600`.
fn cached_json(value: &impl Serialize) -> Response {
    let Ok(bytes) = go_marshal(value) else {
        return AuthError::internal("Error encoding json response").into_response();
    };
    (
        StatusCode::OK,
        [
            (CONTENT_TYPE, HeaderValue::from_static("application/json")),
            (
                CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=600"),
            ),
        ],
        bytes,
    )
        .into_response()
}

/// Go `json.Marshal` escapes `<`, `>`, `&`, U+2028 and U+2029. In serde_json
/// output those characters only occur inside strings, so a textual
/// replacement after serialization is exact.
fn go_marshal(value: &impl Serialize) -> serde_json::Result<String> {
    let text = serde_json::to_string(value)?;
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            other => out.push(other),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{header::ALLOW, Request};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    async fn get_path(state: AuthState, method: &str, path: &str) -> Response {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .unwrap();
        router(state).oneshot(request).await.unwrap()
    }

    async fn body_text(response: Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn expected_document(registration: bool) -> Value {
        let mut document = json!({
            "issuer": "http://localhost:8000/auth/v1",
            "authorization_endpoint": "http://localhost:8000/auth/v1/oauth/authorize",
            "token_endpoint": "http://localhost:8000/auth/v1/oauth/token",
            "jwks_uri": "http://localhost:8000/auth/v1/.well-known/jwks.json",
            "userinfo_endpoint": "http://localhost:8000/auth/v1/oauth/userinfo",
            "scopes_supported": ["openid", "profile", "email", "phone", "offline_access"],
            "response_types_supported": ["code"],
            "response_modes_supported": ["query"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256", "HS256", "ES256"],
            "token_endpoint_auth_methods_supported":
                ["client_secret_basic", "client_secret_post", "none"],
            "claims_supported": CLAIMS_SUPPORTED,
            "code_challenge_methods_supported": ["S256", "plain"],
        });
        if registration {
            document["registration_endpoint"] =
                json!("http://localhost:8000/auth/v1/oauth/clients/register");
        }
        document
    }

    #[tokio::test]
    async fn jwks_hides_the_hmac_key() {
        let response = get_path(
            AuthState::reference(),
            "GET",
            "/auth/v1/.well-known/jwks.json",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
        assert_eq!(response.headers()[CACHE_CONTROL], "public, max-age=600");
        assert_eq!(body_text(response).await, r#"{"keys":[]}"#);
    }

    #[tokio::test]
    async fn jwks_with_configured_keys_is_not_implemented() {
        let state = AuthState::from_lookup(|key| {
            (key == "GOTRUE_JWT_KEYS").then(|| r#"[{"kty":"EC"}]"#.into())
        });
        let response = get_path(state, "GET", "/auth/v1/.well-known/jwks.json").await;
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);

        let state = AuthState::from_lookup(|key| (key == "GOTRUE_JWT_KEYS").then(|| " [] ".into()));
        let response = get_path(state, "GET", "/auth/v1/.well-known/jwks.json").await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn openid_configuration_matches_the_reference_stack() {
        let response = get_path(
            AuthState::reference(),
            "GET",
            "/auth/v1/.well-known/openid-configuration",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CACHE_CONTROL], "public, max-age=600");
        let text = body_text(response).await;
        assert!(
            text.starts_with(
                r#"{"issuer":"http://localhost:8000/auth/v1","authorization_endpoint""#
            ),
            "{text}"
        );
        let body: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(body, expected_document(false));
    }

    #[tokio::test]
    async fn oauth_authorization_server_needs_the_oauth_server() {
        let response = get_path(
            AuthState::reference(),
            "GET",
            "/.well-known/oauth-authorization-server",
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body: Value = serde_json::from_str(&body_text(response).await).unwrap();
        assert_eq!(
            body,
            json!({"code": 404, "error_code": "feature_disabled", "msg": "OAuth server is disabled"})
        );

        let mut state = AuthState::reference();
        state.oauth_server_enabled = true;
        let response = get_path(state, "GET", "/.well-known/oauth-authorization-server").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = serde_json::from_str(&body_text(response).await).unwrap();
        assert_eq!(body, expected_document(false));
    }

    #[tokio::test]
    async fn registration_endpoint_needs_the_oauth_server_and_dynamic_registration() {
        let mut state = AuthState::reference();
        state.oauth_dynamic_registration = true;
        let response = get_path(
            state.clone(),
            "GET",
            "/auth/v1/.well-known/openid-configuration",
        )
        .await;
        let body: Value = serde_json::from_str(&body_text(response).await).unwrap();
        assert_eq!(body, expected_document(false));

        state.oauth_server_enabled = true;
        let response = get_path(state, "GET", "/auth/v1/.well-known/openid-configuration").await;
        let body: Value = serde_json::from_str(&body_text(response).await).unwrap();
        assert_eq!(body, expected_document(true));
    }

    #[tokio::test]
    async fn trailing_slashes_are_trimmed_from_endpoints_but_not_the_issuer() {
        let mut state = AuthState::reference();
        state.config.jwt_issuer = "https://example.com/auth/v1//".into();
        let response = get_path(state, "GET", "/auth/v1/.well-known/openid-configuration").await;
        let body: Value = serde_json::from_str(&body_text(response).await).unwrap();
        assert_eq!(body["issuer"], "https://example.com/auth/v1//");
        assert_eq!(
            body["token_endpoint"],
            "https://example.com/auth/v1/oauth/token"
        );
    }

    #[tokio::test]
    async fn issuer_is_html_escaped_like_go() {
        let mut state = AuthState::reference();
        state.config.jwt_issuer = "https://a.example/?x=<1>&y".into();
        let response = get_path(state, "GET", "/auth/v1/.well-known/openid-configuration").await;
        let text = body_text(response).await;
        assert!(
            text.contains("\"issuer\":\"https://a.example/?x=\\u003c1\\u003e\\u0026y\""),
            "{text}"
        );
    }

    #[tokio::test]
    async fn other_methods_get_chi_405() {
        for path in [
            "/auth/v1/.well-known/jwks.json",
            "/auth/v1/.well-known/openid-configuration",
            "/.well-known/oauth-authorization-server",
        ] {
            for method in ["HEAD", "POST", "PUT", "DELETE"] {
                let response = get_path(AuthState::reference(), method, path).await;
                assert_eq!(
                    response.status(),
                    StatusCode::METHOD_NOT_ALLOWED,
                    "{method} {path}"
                );
                assert_eq!(response.headers()[ALLOW], "GET", "{method} {path}");
            }
        }
    }
}
