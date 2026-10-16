// Ported from supabase/auth internal/api/api.go, internal/api/middleware.go
// (`requirePasskeyEnabled`), internal/api/passkey_manage.go,
// internal/api/passkey_registration.go, internal/api/passkey_authentication.go
// and internal/api/passkey_webauthn.go (MIT), pin v2.197.0.

//! `/auth/v1/passkeys` routes.
//!
//! GoTrue mounts every passkey route in one chi sub-router whose first
//! middleware is `requirePasskeyEnabled`, so every method on every path under
//! `/passkeys` is 404 `passkey_disabled` while `GOTRUE_PASSKEY_ENABLED` is
//! off (the reference stack). With passkeys on, listing, renaming, and
//! deleting a passkey are ported. Registration and authentication run
//! GoTrue's checks up to the go-webauthn call; generating WebAuthn options
//! and verifying attestations and assertions are not ported, so they answer
//! 501. Spec: `specs/auth/passkeys.md`.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use megabase_core::MegabaseNotImplemented;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::admin::system_time_rfc3339;
use crate::error::{unexpected, validation, GoTrueError};
use crate::factors::{parse_gofrs_uuid, require_factor_user};
use crate::gojson::{self, GoStruct, Kind};
use crate::routes::{read_body, JsonOk};
use crate::state::AuthState;
use crate::store::{PasskeyRow, StoreError};
use crate::token::null_as_default;
use crate::user::{require_user, Authed};
use crate::COMPONENT;

const UNIT_AUTHENTICATION_OPTIONS: &str =
    "auth:route:POST /auth/v1/passkeys/authentication/options";
const UNIT_AUTHENTICATION_VERIFY: &str = "auth:route:POST /auth/v1/passkeys/authentication/verify";
const UNIT_REGISTRATION_OPTIONS: &str = "auth:route:POST /auth/v1/passkeys/registration/options";
const UNIT_REGISTRATION_VERIFY: &str = "auth:route:POST /auth/v1/passkeys/registration/verify";

/// `len(friendly_name)` limit in `PasskeyUpdate` (bytes, like Go).
const MAX_FRIENDLY_NAME: usize = 120;

/// `PasskeyUpdateParams` in `internal/api/passkey_manage.go`.
const PASSKEY_UPDATE_PARAMS: GoStruct = GoStruct {
    name: "PasskeyUpdateParams",
    fields: &[("friendly_name", Kind::Str)],
};

/// `PasskeyRegistrationVerifyParams.challenge_id`. `credential` is a
/// `json.RawMessage` that accepts any value, see [`has_credential`].
const REGISTRATION_VERIFY_PARAMS: GoStruct = GoStruct {
    name: "PasskeyRegistrationVerifyParams",
    fields: &[("challenge_id", Kind::Str)],
};

/// `PasskeyAuthenticationVerifyParams.challenge_id`, as above.
const AUTHENTICATION_VERIFY_PARAMS: GoStruct = GoStruct {
    name: "PasskeyAuthenticationVerifyParams",
    fields: &[("challenge_id", Kind::Str)],
};

pub(crate) fn router(state: AuthState) -> Router {
    Router::new()
        .route("/auth/v1/passkeys", any(list))
        .route(
            "/auth/v1/passkeys/authentication/options",
            any(authentication_options),
        )
        .route(
            "/auth/v1/passkeys/authentication/verify",
            any(authentication_verify),
        )
        .route(
            "/auth/v1/passkeys/registration/options",
            any(registration_options),
        )
        .route(
            "/auth/v1/passkeys/registration/verify",
            any(registration_verify),
        )
        // chi mounts `/authentication` and `/registration` as sub-routers,
        // so these never reach `/{passkey_id}`.
        .route("/auth/v1/passkeys/authentication", any(unrouted))
        .route("/auth/v1/passkeys/registration", any(unrouted))
        .route("/auth/v1/passkeys/:passkey_id", any(passkey))
        .with_state(state)
}

/// Deeper paths under `/passkeys` reach the crate fallback (a catch-all
/// here would conflict with `/{passkey_id}` in the router); the gate still
/// answers them first. `Some` is the rejection to send.
pub(crate) fn fallback_gate(passkeys_enabled: bool, path: &str) -> Option<Response> {
    let under = path == "/auth/v1/passkeys" || path.starts_with("/auth/v1/passkeys/");
    if under && !passkeys_enabled {
        Some(disabled())
    } else {
        None
    }
}

fn disabled() -> Response {
    GoTrueError::new(
        StatusCode::NOT_FOUND,
        "passkey_disabled",
        "Passkeys are disabled",
    )
    .into_response()
}

#[derive(Default, Deserialize)]
struct UpdateBody {
    #[serde(default, deserialize_with = "null_as_default")]
    friendly_name: String,
}

#[derive(Default, Deserialize)]
struct VerifyBody {
    #[serde(default, deserialize_with = "null_as_default")]
    challenge_id: String,
}

/// `requirePasskeyEnabled`. It runs before chi matches a route or method.
fn gate(state: &AuthState) -> Result<(), Box<Response>> {
    if state.config.passkeys_enabled {
        Ok(())
    } else {
        Err(Box::new(disabled()))
    }
}

/// A path or method under `/passkeys` with no GoTrue handler, past the gate.
async fn unrouted(State(state): State<AuthState>, method: Method, uri: Uri) -> Response {
    if let Err(response) = gate(&state) {
        return *response;
    }
    not_served(&method, &uri)
}

fn not_served(method: &Method, uri: &Uri) -> Response {
    MegabaseNotImplemented::new(COMPONENT, format!("{method} {}", uri.path())).into_response()
}

// megabase:unit auth:route:GET /auth/v1/passkeys
async fn list(
    State(state): State<AuthState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = gate(&state) {
        return *response;
    }
    if method != Method::GET {
        return not_served(&method, &uri);
    }
    let authed = match require_user(&state, &headers).await {
        Ok(authed) => authed,
        Err(response) => return *response,
    };
    match state.backend.list_passkeys(authed.user.id).await {
        Ok(rows) => JsonOk(Value::Array(rows.iter().map(list_item).collect())).into_response(),
        Err(error) => internal(&error, "Database error loading passkeys"),
    }
}

/// `PATCH` and `DELETE /passkeys/{passkey_id}`.
async fn passkey(
    State(state): State<AuthState>,
    method: Method,
    uri: Uri,
    Path(passkey_id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if let Err(response) = gate(&state) {
        return *response;
    }
    if method == Method::PATCH {
        update(&state, &passkey_id, &headers, body).await
    } else if method == Method::DELETE {
        delete(&state, &passkey_id, &headers).await
    } else {
        not_served(&method, &uri)
    }
}

// megabase:unit auth:route:PATCH /auth/v1/passkeys/{passkey_id}
async fn update(state: &AuthState, raw_id: &str, headers: &HeaderMap, body: Body) -> Response {
    let authed = match require_user(state, headers).await {
        Ok(authed) => authed,
        Err(response) => return *response,
    };
    let Some(passkey_id) = parse_gofrs_uuid(raw_id) else {
        return not_found();
    };
    let bytes = match read_body(body).await {
        Ok(bytes) => bytes,
        Err(error) => return error.into_response(),
    };
    let params = match gojson::decode::<Option<UpdateBody>>(&bytes, &PASSKEY_UPDATE_PARAMS) {
        Ok(params) => params.unwrap_or_default(),
        Err(error) => return error.into_response(),
    };
    if params.friendly_name.is_empty() {
        return validation("friendly_name is required").into_response();
    }
    if params.friendly_name.len() > MAX_FRIENDLY_NAME {
        return validation("friendly_name must be 120 characters or less").into_response();
    }
    let mut row = match find_owned(state, &authed, passkey_id).await {
        Ok(row) => row,
        Err(response) => return *response,
    };
    if let Err(error) = state
        .backend
        .rename_passkey(&authed.user, row.id, &params.friendly_name)
        .await
    {
        return internal(&error, "Database error updating passkey");
    }
    row.friendly_name = params.friendly_name;
    JsonOk(list_item(&row)).into_response()
}

// megabase:unit auth:route:DELETE /auth/v1/passkeys/{passkey_id}
async fn delete(state: &AuthState, raw_id: &str, headers: &HeaderMap) -> Response {
    let authed = match require_user(state, headers).await {
        Ok(authed) => authed,
        Err(response) => return *response,
    };
    if let Err(response) = require_management_aal(state, &authed).await {
        return *response;
    }
    let Some(passkey_id) = parse_gofrs_uuid(raw_id) else {
        return not_found();
    };
    let row = match find_owned(state, &authed, passkey_id).await {
        Ok(row) => row,
        Err(response) => return *response,
    };
    if let Err(error) = state.backend.delete_passkey(&authed.user, row.id).await {
        return internal(&error, "Database error deleting passkey");
    }
    StatusCode::NO_CONTENT.into_response()
}

/// `PasskeyRegistrationOptions` up to `webAuthn.BeginRegistration`.
async fn registration_options(
    State(state): State<AuthState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let authed = match registration_user(&state, &method, &uri, &headers).await {
        Ok(authed) => authed,
        Err(response) => return *response,
    };
    let rows = match state.backend.list_passkeys(authed.user.id).await {
        Ok(rows) => rows,
        Err(error) => return internal(&error, "Database error counting passkeys"),
    };
    let count = i64::try_from(rows.len()).unwrap_or(i64::MAX);
    if count >= state.config.passkey_max_per_user {
        return too_many_passkeys();
    }
    // Building the go-webauthn creation options and storing the challenge
    // session data is not ported.
    MegabaseNotImplemented::new(COMPONENT, UNIT_REGISTRATION_OPTIONS).into_response()
}

/// `PasskeyRegistrationVerify` up to the challenge lookup.
async fn registration_verify(
    State(state): State<AuthState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if let Err(response) = registration_user(&state, &method, &uri, &headers).await {
        return *response;
    }
    if let Err(response) = verify_params(body, &REGISTRATION_VERIFY_PARAMS).await {
        return *response;
    }
    // Consuming the challenge and verifying the attestation is not ported.
    MegabaseNotImplemented::new(COMPONENT, UNIT_REGISTRATION_VERIFY).into_response()
}

/// `PasskeyAuthenticationOptions`: past the gate, the whole handler is the
/// go-webauthn discoverable login options, which are not ported.
async fn authentication_options(
    State(state): State<AuthState>,
    method: Method,
    uri: Uri,
) -> Response {
    if let Err(response) = gate(&state) {
        return *response;
    }
    if method != Method::POST {
        return not_served(&method, &uri);
    }
    MegabaseNotImplemented::new(COMPONENT, UNIT_AUTHENTICATION_OPTIONS).into_response()
}

/// `PasskeyAuthenticationVerify` up to the challenge lookup.
async fn authentication_verify(
    State(state): State<AuthState>,
    method: Method,
    uri: Uri,
    body: Body,
) -> Response {
    if let Err(response) = gate(&state) {
        return *response;
    }
    if method != Method::POST {
        return not_served(&method, &uri);
    }
    if let Err(response) = verify_params(body, &AUTHENTICATION_VERIFY_PARAMS).await {
        return *response;
    }
    // Consuming the challenge and verifying the assertion is not ported.
    MegabaseNotImplemented::new(COMPONENT, UNIT_AUTHENTICATION_VERIFY).into_response()
}

/// The gate, then `requireAuthentication` and `requireNotAnonymous` on the
/// `/registration` sub-router (they run before its method match), then the
/// SSO and AAL checks both registration handlers start with.
async fn registration_user(
    state: &AuthState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
) -> Result<Authed, Box<Response>> {
    gate(state)?;
    let authed = require_factor_user(state, headers).await?;
    if method != Method::POST {
        return Err(Box::new(not_served(method, uri)));
    }
    if authed.user.is_sso_user {
        return Err(reject(GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "SSO users cannot register passkeys",
        )));
    }
    require_management_aal(state, &authed).await?;
    Ok(authed)
}

/// Body checks shared by both verify handlers, up to the challenge id parse.
async fn verify_params(body: Body, target: &GoStruct) -> Result<Uuid, Box<Response>> {
    let bytes = read_body(body).await.map_err(reject)?;
    let params = gojson::decode::<Option<VerifyBody>>(&bytes, target)
        .map_err(reject)?
        .unwrap_or_default();
    if params.challenge_id.is_empty() {
        return Err(reject(validation("challenge_id is required")));
    }
    if !has_credential(&bytes) {
        return Err(reject(validation("credential is required")));
    }
    parse_gofrs_uuid(&params.challenge_id)
        .ok_or_else(|| reject(validation("challenge_id must be a valid UUID")))
}

/// A `json.RawMessage` field is non-nil whenever its key is present, even
/// for a JSON `null`. Keys match case-insensitively, like Go's decoder.
fn has_credential(bytes: &[u8]) -> bool {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(object)) => object
            .keys()
            .any(|key| key.eq_ignore_ascii_case("credential")),
        _ => false,
    }
}

/// `requirePasskeyManagementAAL`.
async fn require_management_aal(state: &AuthState, authed: &Authed) -> Result<(), Box<Response>> {
    match state
        .backend
        .passkey_management_needs_aal2(authed.user.id, authed.session_id)
        .await
    {
        Ok(false) => Ok(()),
        Ok(true) => Err(reject(GoTrueError::new(
            StatusCode::FORBIDDEN,
            "insufficient_aal",
            "AAL2 session is required to manage passkeys when MFA is enabled",
        ))),
        Err(error) => Err(Box::new(internal(
            &error,
            "Unexpected failure, please check server logs for more information",
        ))),
    }
}

/// `FindWebAuthnCredentialByIDAndUserID` with GoTrue's error mapping.
async fn find_owned(
    state: &AuthState,
    authed: &Authed,
    passkey_id: Uuid,
) -> Result<PasskeyRow, Box<Response>> {
    match state
        .backend
        .find_owned_passkey(authed.user.id, passkey_id)
        .await
    {
        Ok(Some(row)) => Ok(row),
        Ok(None) => Err(Box::new(not_found())),
        Err(error) => Err(Box::new(internal(&error, "Database error loading passkey"))),
    }
}

/// `toPasskeyListItem`: `friendly_name` and `last_used_at` are `omitempty`.
fn list_item(row: &PasskeyRow) -> Value {
    let mut item = Map::new();
    item.insert("id".into(), json!(row.id.to_string()));
    if !row.friendly_name.is_empty() {
        item.insert("friendly_name".into(), json!(row.friendly_name));
    }
    item.insert(
        "created_at".into(),
        json!(system_time_rfc3339(row.created_at)),
    );
    if let Some(used) = row.last_used_at {
        item.insert("last_used_at".into(), json!(system_time_rfc3339(used)));
    }
    Value::Object(item)
}

fn not_found() -> Response {
    GoTrueError::new(
        StatusCode::NOT_FOUND,
        "validation_failed",
        "Passkey not found",
    )
    .into_response()
}

fn too_many_passkeys() -> Response {
    GoTrueError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "too_many_passkeys",
        "Maximum number of passkeys reached",
    )
    .into_response()
}

/// GoTrue's `NewInternalServerError(message)`; the store error is logged only.
fn internal(error: &StoreError, message: &'static str) -> Response {
    tracing::error!(%error, "passkey store operation failed");
    unexpected(message).into_response()
}

fn reject(response: impl IntoResponse) -> Box<Response> {
    Box::new(response.into_response())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::config::AuthConfig;
    use crate::store::Backend;
    use axum::body::to_bytes;
    use axum::http::Request;
    use megabase_core::Hs256;
    use tower::ServiceExt;

    const SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";
    const EMAIL: &str = "passkey@example.com";
    const SOME_ID: &str = "00000000-0000-0000-0000-000000000001";

    struct Fixture {
        app: Router,
        backend: Backend,
        token: String,
        session_id: Uuid,
    }

    async fn fixture(configure: impl FnOnce(&mut AuthConfig)) -> Fixture {
        let backend = Backend::memory();
        let mut config = AuthConfig::reference_defaults();
        config.passkeys_enabled = true;
        configure(&mut config);
        let jwt = Hs256::new(SECRET.as_bytes()).unwrap();
        let app = crate::router_with_state(AuthState::new(config, Some(jwt), backend.clone()));
        let body = json!({ "email": EMAIL, "password": "secret-pass" }).to_string();
        let (status, body) = call(&app, "POST", "/auth/v1/signup", Some(&body), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let token = body["access_token"].as_str().unwrap().to_string();
        let claims = Hs256::new(SECRET.as_bytes())
            .unwrap()
            .verify(&token)
            .unwrap();
        let session_id = Uuid::parse_str(claims.raw["session_id"].as_str().unwrap()).unwrap();
        Fixture {
            app,
            backend,
            token,
            session_id,
        }
    }

    fn resign(token: &str, edit: impl FnOnce(&mut Map<String, Value>)) -> String {
        let jwt = Hs256::new(SECRET.as_bytes()).unwrap();
        let mut claims = jwt.verify(token).unwrap().raw;
        edit(&mut claims);
        jwt.sign(&Value::Object(claims)).unwrap().to_string()
    }

    async fn call(
        app: &Router,
        method: &str,
        path: &str,
        body: Option<&str>,
        token: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let request = builder
            .body(Body::from(body.unwrap_or("").to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
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

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[tokio::test]
    async fn disabled_gate_answers_every_method_and_path() {
        let fx = fixture(|config| config.passkeys_enabled = false).await;
        let paths = [
            "/auth/v1/passkeys",
            "/auth/v1/passkeys/authentication/options",
            "/auth/v1/passkeys/authentication/verify",
            "/auth/v1/passkeys/registration/options",
            "/auth/v1/passkeys/registration/verify",
            "/auth/v1/passkeys/registration",
            "/auth/v1/passkeys/not-a-uuid",
            "/auth/v1/passkeys/00000000-0000-0000-0000-000000000001",
            "/auth/v1/passkeys/a/b/c",
        ];
        for path in paths {
            for method in ["GET", "POST", "PATCH", "DELETE", "PUT"] {
                // The gate runs before authentication.
                for token in [None, Some(fx.token.as_str())] {
                    let result = call(&fx.app, method, path, Some("{}"), token).await;
                    assert_error(
                        &result,
                        StatusCode::NOT_FOUND,
                        "passkey_disabled",
                        "Passkeys are disabled",
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn management_routes_require_authentication() {
        let fx = fixture(|_| {}).await;
        let item = format!("/auth/v1/passkeys/{SOME_ID}");
        for (method, path) in [
            ("GET", "/auth/v1/passkeys"),
            ("PATCH", item.as_str()),
            ("DELETE", item.as_str()),
            ("POST", "/auth/v1/passkeys/registration/options"),
            ("POST", "/auth/v1/passkeys/registration/verify"),
        ] {
            let result = call(&fx.app, method, path, Some("{}"), None).await;
            assert_error(
                &result,
                StatusCode::UNAUTHORIZED,
                "no_authorization",
                "This endpoint requires a valid Bearer token",
            );
        }
    }

    #[tokio::test]
    async fn list_returns_the_users_passkeys_oldest_first() {
        let fx = fixture(|_| {}).await;
        let result = call(&fx.app, "GET", "/auth/v1/passkeys", None, Some(&fx.token)).await;
        assert_eq!(result, (StatusCode::OK, json!([])));

        let newer = fx
            .backend
            .add_passkey_for_test(EMAIL, "", at(2_000_000_000), None)
            .await;
        let older = fx
            .backend
            .add_passkey_for_test(EMAIL, "Laptop", at(1_700_000_000), Some(at(1_800_000_000)))
            .await;
        let result = call(&fx.app, "GET", "/auth/v1/passkeys", None, Some(&fx.token)).await;
        assert_eq!(
            result,
            (
                StatusCode::OK,
                json!([
                    {
                        "id": older.to_string(),
                        "friendly_name": "Laptop",
                        "created_at": "2023-11-14T22:13:20Z",
                        "last_used_at": "2027-01-15T08:00:00Z",
                    },
                    { "id": newer.to_string(), "created_at": "2033-05-18T03:33:20Z" },
                ])
            )
        );
    }

    #[tokio::test]
    async fn update_validates_then_renames() {
        let fx = fixture(|_| {}).await;
        let id = fx
            .backend
            .add_passkey_for_test(EMAIL, "Old", at(1_700_000_000), None)
            .await;
        let path = format!("/auth/v1/passkeys/{id}");
        let token = Some(fx.token.as_str());

        let result = call(
            &fx.app,
            "PATCH",
            "/auth/v1/passkeys/nope",
            Some("{}"),
            token,
        )
        .await;
        assert_error(
            &result,
            StatusCode::NOT_FOUND,
            "validation_failed",
            "Passkey not found",
        );
        let result = call(
            &fx.app,
            "PATCH",
            &path,
            Some(r#"{"friendly_name":1}"#),
            token,
        )
        .await;
        assert_error(
            &result,
            StatusCode::BAD_REQUEST,
            "bad_json",
            "Could not parse request body as JSON: json: cannot unmarshal number into Go struct field PasskeyUpdateParams.friendly_name of type string",
        );
        for body in ["{}", "null", r#"{"friendly_name":null}"#] {
            let result = call(&fx.app, "PATCH", &path, Some(body), token).await;
            assert_error(
                &result,
                StatusCode::BAD_REQUEST,
                "validation_failed",
                "friendly_name is required",
            );
        }
        // 61 two-byte runes: 122 bytes, over Go's `len` limit.
        let long = json!({ "friendly_name": "é".repeat(61) }).to_string();
        let result = call(&fx.app, "PATCH", &path, Some(&long), token).await;
        assert_error(
            &result,
            StatusCode::BAD_REQUEST,
            "validation_failed",
            "friendly_name must be 120 characters or less",
        );
        let foreign = format!("/auth/v1/passkeys/{SOME_ID}");
        let result = call(
            &fx.app,
            "PATCH",
            &foreign,
            Some(r#"{"friendly_name":"x"}"#),
            token,
        )
        .await;
        assert_error(
            &result,
            StatusCode::NOT_FOUND,
            "validation_failed",
            "Passkey not found",
        );

        let result = call(
            &fx.app,
            "PATCH",
            &path,
            Some(r#"{"Friendly_Name":"Phone"}"#),
            token,
        )
        .await;
        assert_eq!(
            result,
            (
                StatusCode::OK,
                json!({
                    "id": id.to_string(),
                    "friendly_name": "Phone",
                    "created_at": "2023-11-14T22:13:20Z",
                })
            )
        );
        let result = call(&fx.app, "GET", "/auth/v1/passkeys", None, token).await;
        assert_eq!(result.1[0]["friendly_name"], "Phone");
    }

    #[tokio::test]
    async fn delete_removes_an_owned_passkey() {
        let fx = fixture(|_| {}).await;
        let id = fx
            .backend
            .add_passkey_for_test(EMAIL, "Key", at(1_700_000_000), None)
            .await;
        let token = Some(fx.token.as_str());
        let result = call(&fx.app, "DELETE", "/auth/v1/passkeys/nope", None, token).await;
        assert_error(
            &result,
            StatusCode::NOT_FOUND,
            "validation_failed",
            "Passkey not found",
        );
        let foreign = format!("/auth/v1/passkeys/{SOME_ID}");
        let result = call(&fx.app, "DELETE", &foreign, None, token).await;
        assert_error(
            &result,
            StatusCode::NOT_FOUND,
            "validation_failed",
            "Passkey not found",
        );
        // gofrs accepts the braced form.
        let path = format!("/auth/v1/passkeys/%7B{id}%7D");
        let result = call(&fx.app, "DELETE", &path, None, token).await;
        assert_eq!(result, (StatusCode::NO_CONTENT, Value::Null));
        let result = call(&fx.app, "GET", "/auth/v1/passkeys", None, token).await;
        assert_eq!(result.1, json!([]));
    }

    #[tokio::test]
    async fn verified_factor_needs_aal2_to_delete_or_register() {
        let fx = fixture(|_| {}).await;
        let factor = fx
            .backend
            .add_factor_for_test(EMAIL, "totp", "verified")
            .await;
        let token = Some(fx.token.as_str());
        let item = format!("/auth/v1/passkeys/{SOME_ID}");
        for (method, path) in [
            ("DELETE", item.as_str()),
            ("POST", "/auth/v1/passkeys/registration/options"),
            ("POST", "/auth/v1/passkeys/registration/verify"),
        ] {
            let result = call(&fx.app, method, path, Some("{}"), token).await;
            assert_error(
                &result,
                StatusCode::FORBIDDEN,
                "insufficient_aal",
                "AAL2 session is required to manage passkeys when MFA is enabled",
            );
        }
        // Rename has no AAL check.
        let body = r#"{"friendly_name":"x"}"#;
        let result = call(&fx.app, "PATCH", &item, Some(body), token).await;
        assert_error(
            &result,
            StatusCode::NOT_FOUND,
            "validation_failed",
            "Passkey not found",
        );
        fx.backend.set_aal2_for_test(fx.session_id, factor).await;
        let result = call(&fx.app, "DELETE", &item, None, token).await;
        assert_error(
            &result,
            StatusCode::NOT_FOUND,
            "validation_failed",
            "Passkey not found",
        );
    }

    #[tokio::test]
    async fn registration_checks_then_501() {
        let fx = fixture(|config| config.passkey_max_per_user = 1).await;
        let token = Some(fx.token.as_str());
        let options = "/auth/v1/passkeys/registration/options";
        let anonymous = resign(&fx.token, |claims| {
            claims.insert("is_anonymous".into(), Value::Bool(true));
        });
        let result = call(&fx.app, "POST", options, None, Some(&anonymous)).await;
        assert_error(
            &result,
            StatusCode::FORBIDDEN,
            "no_authorization",
            "Anonymous user not allowed to perform these actions",
        );

        let result = call(&fx.app, "POST", options, None, token).await;
        assert_501(&result, UNIT_REGISTRATION_OPTIONS);
        fx.backend
            .add_passkey_for_test(EMAIL, "", at(1_700_000_000), None)
            .await;
        let result = call(&fx.app, "POST", options, None, token).await;
        assert_error(
            &result,
            StatusCode::UNPROCESSABLE_ENTITY,
            "too_many_passkeys",
            "Maximum number of passkeys reached",
        );

        let verify = "/auth/v1/passkeys/registration/verify";
        let result = call(&fx.app, "POST", verify, Some("{}"), token).await;
        assert_error(
            &result,
            StatusCode::BAD_REQUEST,
            "validation_failed",
            "challenge_id is required",
        );
        // A `null` credential is still a non-nil `json.RawMessage`.
        let body = format!(r#"{{"challenge_id":"{SOME_ID}","credential":null}}"#);
        let result = call(&fx.app, "POST", verify, Some(&body), token).await;
        assert_501(&result, UNIT_REGISTRATION_VERIFY);

        fx.backend.set_sso_for_test(EMAIL).await;
        let result = call(&fx.app, "POST", verify, Some("{}"), token).await;
        assert_error(
            &result,
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "SSO users cannot register passkeys",
        );
    }

    #[tokio::test]
    async fn authentication_checks_then_501() {
        let fx = fixture(|_| {}).await;
        let options = "/auth/v1/passkeys/authentication/options";
        let result = call(&fx.app, "POST", options, None, None).await;
        assert_501(&result, UNIT_AUTHENTICATION_OPTIONS);

        let verify = "/auth/v1/passkeys/authentication/verify";
        let result = call(
            &fx.app,
            "POST",
            verify,
            Some(r#"{"challenge_id":"x"}"#),
            None,
        )
        .await;
        assert_error(
            &result,
            StatusCode::BAD_REQUEST,
            "validation_failed",
            "credential is required",
        );
        let body = r#"{"challenge_id":"x","credential":{}}"#;
        let result = call(&fx.app, "POST", verify, Some(body), None).await;
        assert_error(
            &result,
            StatusCode::BAD_REQUEST,
            "validation_failed",
            "challenge_id must be a valid UUID",
        );
        let body = format!(r#"{{"challenge_id":"{SOME_ID}","credential":{{}}}}"#);
        let result = call(&fx.app, "POST", verify, Some(&body), None).await;
        assert_501(&result, UNIT_AUTHENTICATION_VERIFY);
    }

    #[tokio::test]
    async fn unrouted_methods_and_paths_are_501_when_enabled() {
        let fx = fixture(|_| {}).await;
        let token = Some(fx.token.as_str());
        for (method, path) in [
            ("POST", "/auth/v1/passkeys"),
            ("GET", "/auth/v1/passkeys/authentication/options"),
            (
                "PUT",
                "/auth/v1/passkeys/00000000-0000-0000-0000-000000000001",
            ),
            ("PATCH", "/auth/v1/passkeys/registration"),
            ("GET", "/auth/v1/passkeys/a/b"),
        ] {
            let result = call(&fx.app, method, path, None, token).await;
            assert_501(&result, &format!("{method} {path}"));
        }
    }
}
