// Ported from supabase/auth internal/api/api.go, internal/api/middleware.go
// (`requireSAMLEnabled`), internal/api/sso.go, internal/api/saml.go,
// internal/api/samlacs.go and internal/models/sso.go (MIT), pin v2.197.0.

//! `/auth/v1/sso` routes.
//!
//! GoTrue mounts every SSO route in one chi sub-router whose first
//! middleware is `requireSAMLEnabled`, so every method on every path under
//! `/sso` is 404 `saml_provider_disabled` while `GOTRUE_SAML_ENABLED` is off
//! (the reference stack). With SAML on, `POST /sso` runs the rate limit,
//! parameter checks, the provider lookup, and the PKCE checks; signing the
//! SAML `AuthnRequest`, the SP metadata document, and verifying an
//! assertion at the ACS need RSA, X.509, and XML-DSig, which are not ported,
//! so they answer 501. Spec: `specs/auth/sso.md`.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use megabase_core::MegabaseNotImplemented;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::error::{unexpected, validation, GoTrueError};
use crate::factors::parse_gofrs_uuid;
use crate::gojson::{self, GoStruct, Kind, Pairs};
use crate::ratelimit::limit_by_header;
use crate::routes::{read_body, validate_pkce};
use crate::state::AuthState;
use crate::token::null_as_default;
use crate::COMPONENT;

const UNIT_SSO: &str = "auth:route:POST /auth/v1/sso";
const UNIT_METADATA: &str = "auth:route:GET /auth/v1/sso/saml/metadata";
const UNIT_ACS: &str = "auth:route:POST /auth/v1/sso/saml/acs";

/// `SingleSignOnParams` in `internal/api/sso.go`. `provider_id` is a
/// `uuid.UUID`, checked by [`unported_provider_id`] and decoded by
/// [`provider_id`].
const SINGLE_SIGN_ON_PARAMS: GoStruct = GoStruct {
    name: "SingleSignOnParams",
    fields: &[
        ("provider_id", Kind::Text),
        ("domain", Kind::Str),
        ("redirect_to", Kind::Str),
        ("skip_http_redirect", Kind::Bool),
        ("code_challenge", Kind::Str),
        ("code_challenge_method", Kind::Str),
    ],
};

pub(crate) fn router(state: AuthState) -> Router {
    Router::new()
        // chi serves `Post("/")` in `Route("/sso")` for `/sso` and `/sso/`.
        .route("/auth/v1/sso", any(single_sign_on))
        .route("/auth/v1/sso/", any(single_sign_on))
        .route("/auth/v1/sso/saml/metadata", any(metadata))
        .route("/auth/v1/sso/saml/acs", any(acs))
        .with_state(state)
}

/// Other paths under `/sso` reach the crate fallback; the gate still answers
/// them first. `Some` is the rejection to send.
pub(crate) fn fallback_gate(saml_enabled: bool, path: &str) -> Option<Response> {
    let under = path == "/auth/v1/sso" || path.starts_with("/auth/v1/sso/");
    if under && !saml_enabled {
        Some(disabled())
    } else {
        None
    }
}

fn disabled() -> Response {
    GoTrueError::new(
        StatusCode::NOT_FOUND,
        "saml_provider_disabled",
        "SAML 2.0 is disabled",
    )
    .into_response()
}

/// `requireSAMLEnabled`. It runs before chi matches a route or method.
fn gate(state: &AuthState) -> Result<(), Box<Response>> {
    if state.config.saml_enabled {
        Ok(())
    } else {
        Err(Box::new(disabled()))
    }
}

fn not_served(method: &Method, uri: &Uri) -> Response {
    MegabaseNotImplemented::new(COMPONENT, format!("{method} {}", uri.path())).into_response()
}

#[derive(Default, Deserialize)]
struct SingleSignOnBody {
    #[serde(default)]
    provider_id: Value,
    #[serde(default, deserialize_with = "null_as_default")]
    domain: String,
    #[serde(default, deserialize_with = "null_as_default")]
    code_challenge: String,
    #[serde(default, deserialize_with = "null_as_default")]
    code_challenge_method: String,
}

/// `SingleSignOn` up to the SAML `AuthnRequest`.
async fn single_sign_on(
    State(state): State<AuthState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if let Err(response) = gate(&state) {
        return *response;
    }
    if method != Method::POST {
        return not_served(&method, &uri);
    }
    if let Err(error) = limit_by_header(&state.sso_limiter, &state.rate_limit_header, &headers) {
        return error.into_response();
    }
    // `verifyCaptcha` is a no-op: captcha is not configurable in Megabase,
    // and the reference stack leaves it off.
    let bytes = match read_body(body).await {
        Ok(bytes) => bytes,
        Err(error) => return error.into_response(),
    };
    if unported_provider_id(&bytes) {
        return MegabaseNotImplemented::new(COMPONENT, UNIT_SSO).into_response();
    }
    let params = match gojson::decode::<Option<SingleSignOnBody>>(&bytes, &SINGLE_SIGN_ON_PARAMS) {
        Ok(params) => params.unwrap_or_default(),
        Err(error) => return error.into_response(),
    };
    let Some(provider_id) = provider_id(&params.provider_id) else {
        return MegabaseNotImplemented::new(COMPONENT, UNIT_SSO).into_response();
    };
    let lookup = match (provider_id, params.domain.is_empty()) {
        (Some(_), false) => {
            return validation("Only one of provider_id or domain supported").into_response()
        }
        (None, true) => {
            return validation("A provider_id or domain needs to be provided").into_response()
        }
        (Some(id), true) => find_provider_by_id(&state, id).await,
        (None, false) => find_provider_by_domain(&state, &params.domain).await,
    };
    let disabled = match lookup {
        Ok(disabled) => disabled,
        Err(error) => return error.into_response(),
    };
    if disabled == Some(true) {
        return GoTrueError::new(
            StatusCode::NOT_FOUND,
            "sso_provider_disabled",
            "SSO Provider is currently disabled",
        )
        .into_response();
    }
    if let Err(error) = validate_pkce(&params.code_challenge_method, &params.code_challenge) {
        return error.into_response();
    }
    // Creating the flow state and relay state, then signing the redirect
    // binding `AuthnRequest` (crewjam/saml, RSA-SHA256), is not ported.
    MegabaseNotImplemented::new(COMPONENT, UNIT_SSO).into_response()
}

/// A valid JSON object whose `provider_id` (any key case) is neither `null`
/// nor a UUID gofrs accepts. Go's `json.Unmarshal` then fails with gofrs
/// `UnmarshalText` wording, or a `uuid.UUID` type error whose precedence
/// over other field errors depends on key order; neither is ported.
fn unported_provider_id(bytes: &[u8]) -> bool {
    let Ok(Pairs(pairs)) = serde_json::from_slice::<Pairs>(bytes) else {
        return false;
    };
    pairs
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("provider_id"))
        .any(|(_, value)| provider_id(value).is_none())
}

/// `provider_id` as Go decodes it into a `uuid.UUID`: absent or `null` is
/// `uuid.Nil`, which `validate` treats as no provider id. `None` is a value
/// gofrs rejects.
fn provider_id(raw: &Value) -> Option<Option<Uuid>> {
    match raw {
        Value::Null => Some(None),
        Value::String(text) => {
            let id = parse_gofrs_uuid(text)?;
            Some((!id.is_nil()).then_some(id))
        }
        _ => None,
    }
}

/// `models.FindSSOProviderByID`; `Ok` is the provider's `disabled` column.
async fn find_provider_by_id(state: &AuthState, id: Uuid) -> Result<Option<bool>, GoTrueError> {
    const FAILED: &str = "Unable to find SSO provider by ID";
    let pool = state.backend.pg_pool().ok_or_else(|| unexpected(FAILED))?;
    let row: Option<(Option<bool>,)> =
        sqlx::query_as("SELECT disabled FROM auth.sso_providers WHERE id = $1 LIMIT 1")
            .bind(id)
            .fetch_optional(&pool)
            .await
            .map_err(|error| lookup_failed(&error, FAILED))?;
    row.map(|(disabled,)| disabled).ok_or_else(|| {
        GoTrueError::new(
            StatusCode::NOT_FOUND,
            "sso_provider_not_found",
            "No such SSO provider",
        )
    })
}

/// `models.FindSSOProviderByDomain`: the domain row, then its provider.
async fn find_provider_by_domain(
    state: &AuthState,
    domain: &str,
) -> Result<Option<bool>, GoTrueError> {
    const FAILED: &str = "Unable to find SSO provider by domain";
    let pool = state.backend.pg_pool().ok_or_else(|| unexpected(FAILED))?;
    let row: Option<(Option<bool>,)> = sqlx::query_as(
        "SELECT p.disabled FROM auth.sso_domains d \
         JOIN auth.sso_providers p ON p.id = d.sso_provider_id \
         WHERE d.domain = $1 LIMIT 1",
    )
    .bind(domain)
    .fetch_optional(&pool)
    .await
    .map_err(|error| lookup_failed(&error, FAILED))?;
    row.map(|(disabled,)| disabled).ok_or_else(|| {
        GoTrueError::new(
            StatusCode::NOT_FOUND,
            "sso_provider_not_found",
            "No SSO provider assigned for this domain",
        )
    })
}

fn lookup_failed(error: &sqlx::Error, message: &'static str) -> GoTrueError {
    tracing::error!(%error, "SSO provider lookup failed");
    unexpected(message)
}

/// `SAMLMetadata`: past the gate, building the SP metadata needs the X.509
/// certificate of `GOTRUE_SAML_PRIVATE_KEY`, which is not ported.
async fn metadata(State(state): State<AuthState>, method: Method, uri: Uri) -> Response {
    if let Err(response) = gate(&state) {
        return *response;
    }
    if method != Method::GET {
        return not_served(&method, &uri);
    }
    MegabaseNotImplemented::new(COMPONENT, UNIT_METADATA).into_response()
}

/// `SamlAcs`: past the gate and the assertion rate limit, parsing and
/// verifying the signed SAML response is not ported.
async fn acs(
    State(state): State<AuthState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = gate(&state) {
        return *response;
    }
    if method != Method::POST {
        return not_served(&method, &uri);
    }
    if let Err(error) = limit_by_header(
        &state.saml_assertion_limiter,
        &state.rate_limit_header,
        &headers,
    ) {
        return error.into_response();
    }
    MegabaseNotImplemented::new(COMPONENT, UNIT_ACS).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;
    use crate::store::Backend;
    use axum::body::to_bytes;
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;

    const RATE_HEADER: &str = "X-Megabase-Judge-Rate-Key";

    fn app(saml_enabled: bool) -> Router {
        let mut config = AuthConfig::reference_defaults();
        config.saml_enabled = saml_enabled;
        crate::router_with_state(AuthState::new(config, None, Backend::memory()))
    }

    async fn call(
        app: &Router,
        method: &str,
        path: &str,
        body: &str,
        rate_key: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(key) = rate_key {
            builder = builder.header(RATE_HEADER, key);
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

    fn assert_error(result: &(StatusCode, Value), status: StatusCode, code: &str, msg: &str) {
        assert_eq!(result.0, status, "{}", result.1);
        assert_eq!(result.1["code"], status.as_u16(), "{}", result.1);
        assert_eq!(result.1["error_code"], code, "{}", result.1);
        assert_eq!(result.1["msg"], msg, "{}", result.1);
    }

    fn assert_501(result: &(StatusCode, Value), unit: &str) {
        assert_eq!(result.0, StatusCode::NOT_IMPLEMENTED, "{}", result.1);
        assert_eq!(result.1["code"], "MEGABASE_NOT_IMPLEMENTED");
        assert_eq!(result.1["unit"], unit);
    }

    #[tokio::test]
    async fn disabled_gate_answers_every_method_and_path() {
        let app = app(false);
        let paths = [
            "/auth/v1/sso",
            "/auth/v1/sso/",
            "/auth/v1/sso/saml",
            "/auth/v1/sso/saml/metadata",
            "/auth/v1/sso/saml/acs",
            "/auth/v1/sso/unknown/deeper",
        ];
        for path in paths {
            for method in ["GET", "POST", "PUT", "DELETE"] {
                let result = call(&app, method, path, "not json", None).await;
                assert_error(
                    &result,
                    StatusCode::NOT_FOUND,
                    "saml_provider_disabled",
                    "SAML 2.0 is disabled",
                );
            }
        }
    }

    #[tokio::test]
    async fn gate_runs_before_the_rate_limit() {
        let app = app(false);
        for _ in 0..40 {
            let result = call(&app, "POST", "/auth/v1/sso", "{}", Some("gate")).await;
            assert_eq!(result.0, StatusCode::NOT_FOUND);
        }
    }

    #[tokio::test]
    async fn enabled_sso_checks_params_before_the_lookup() {
        let app = app(true);
        assert_error(
            &call(&app, "POST", "/auth/v1/sso", "{}", None).await,
            StatusCode::BAD_REQUEST,
            "validation_failed",
            "A provider_id or domain needs to be provided",
        );
        assert_error(
            &call(&app, "POST", "/auth/v1/sso/", "null", None).await,
            StatusCode::BAD_REQUEST,
            "validation_failed",
            "A provider_id or domain needs to be provided",
        );
        let nil = json!({ "provider_id": "00000000-0000-0000-0000-000000000000" });
        assert_error(
            &call(&app, "POST", "/auth/v1/sso", &nil.to_string(), None).await,
            StatusCode::BAD_REQUEST,
            "validation_failed",
            "A provider_id or domain needs to be provided",
        );
        let both = json!({
            "provider_id": "00000000-0000-0000-0000-000000000001",
            "domain": "example.com",
        });
        assert_error(
            &call(&app, "POST", "/auth/v1/sso", &both.to_string(), None).await,
            StatusCode::BAD_REQUEST,
            "validation_failed",
            "Only one of provider_id or domain supported",
        );
        let bad = call(&app, "POST", "/auth/v1/sso", "{", None).await;
        assert_eq!(bad.0, StatusCode::BAD_REQUEST);
        assert_eq!(bad.1["error_code"], "bad_json");
        let typed = json!({ "domain": 1 });
        assert_eq!(
            call(&app, "POST", "/auth/v1/sso", &typed.to_string(), None)
                .await
                .1["msg"],
            "Could not parse request body as JSON: json: cannot unmarshal number into Go \
             struct field SingleSignOnParams.domain of type string"
        );
    }

    #[tokio::test]
    async fn enabled_sso_unparsed_provider_id_is_501() {
        let app = app(true);
        for raw in [json!("not-a-uuid"), json!(""), json!(7)] {
            let body = json!({ "provider_id": raw }).to_string();
            assert_501(
                &call(&app, "POST", "/auth/v1/sso", &body, None).await,
                UNIT_SSO,
            );
        }
    }

    #[tokio::test]
    async fn enabled_sso_without_database_is_lookup_failure() {
        let app = app(true);
        let by_id = json!({ "provider_id": "00000000-0000-0000-0000-000000000001" });
        assert_error(
            &call(&app, "POST", "/auth/v1/sso", &by_id.to_string(), None).await,
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected_failure",
            "Unable to find SSO provider by ID",
        );
        let by_domain = json!({ "domain": "example.com" });
        assert_error(
            &call(&app, "POST", "/auth/v1/sso", &by_domain.to_string(), None).await,
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected_failure",
            "Unable to find SSO provider by domain",
        );
    }

    #[tokio::test]
    async fn enabled_sso_is_rate_limited_before_the_body() {
        let app = app(true);
        for _ in 0..30 {
            let result = call(&app, "POST", "/auth/v1/sso", "{", Some("sso")).await;
            assert_eq!(result.0, StatusCode::BAD_REQUEST);
        }
        assert_error(
            &call(&app, "POST", "/auth/v1/sso", "{", Some("sso")).await,
            StatusCode::TOO_MANY_REQUESTS,
            "over_request_rate_limit",
            "Request rate limit reached",
        );
    }

    #[tokio::test]
    async fn enabled_saml_routes_are_501() {
        let app = app(true);
        assert_501(
            &call(&app, "GET", "/auth/v1/sso/saml/metadata", "", None).await,
            UNIT_METADATA,
        );
        assert_501(
            &call(&app, "POST", "/auth/v1/sso/saml/acs", "", None).await,
            UNIT_ACS,
        );
        assert_501(
            &call(&app, "GET", "/auth/v1/sso", "", None).await,
            "GET /auth/v1/sso",
        );
        assert_501(
            &call(&app, "POST", "/auth/v1/sso/saml/metadata", "", None).await,
            "POST /auth/v1/sso/saml/metadata",
        );
        assert_501(
            &call(&app, "GET", "/auth/v1/sso/other", "", None).await,
            "GET /auth/v1/sso/other",
        );
    }

    #[tokio::test]
    async fn enabled_acs_is_rate_limited() {
        let app = app(true);
        for _ in 0..30 {
            assert_501(
                &call(&app, "POST", "/auth/v1/sso/saml/acs", "", Some("acs")).await,
                UNIT_ACS,
            );
        }
        assert_error(
            &call(&app, "POST", "/auth/v1/sso/saml/acs", "", Some("acs")).await,
            StatusCode::TOO_MANY_REQUESTS,
            "over_request_rate_limit",
            "Request rate limit reached",
        );
    }
}
