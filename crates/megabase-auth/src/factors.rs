// Ported from supabase/auth internal/api/mfa.go, internal/api/recovery_codes.go,
// internal/api/api.go, internal/api/auth.go, internal/api/admin.go (`loadFactor`),
// and internal/crypto/recovery_codes.go (MIT), pin v2.197.0.

//! `/auth/v1/factors` routes.
//!
//! Every route runs GoTrue's `requireAuthentication` and `requireNotAnonymous`
//! first, and the `{factor_id}` routes then `loadFactor`. TOTP and phone
//! enrollment, challenge, and verification are ported, as are recovery-code
//! generation, rotation, and redemption. WebAuthn challenge and verification
//! stay 501 when those switches are on: credential ceremonies are not ported.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::Router;
use megabase_core::MegabaseNotImplemented;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use std::net::IpAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{unexpected, validation, GoTrueError};
use crate::gojson::{self, GoStruct, Kind};
use crate::hooks::TokenHookCall;
use crate::jsonutil::{mfa_access_claims, session_json};
use crate::routes::{read_body, JsonOk, INVALID_CHANNEL};
use crate::state::AuthState;
use crate::store::{
    EnrolledFactor, FactorRow, FactorVerify, IssuedSession, MfaError, RecoveryIssue,
    UpgradedSession,
};
use crate::token::null_as_default;
use crate::totp::issuer_host;
use crate::user::{require_user, validate_phone, Authed};
use crate::COMPONENT;

const UNIT_CHALLENGE: &str = "auth:route:POST /auth/v1/factors/{factor_id}/challenge";
const UNIT_VERIFY: &str = "auth:route:POST /auth/v1/factors/{factor_id}/verify";

const NOT_ENROLLED: &str = "The user has not enrolled recovery codes";
const BAD_FACTOR_TYPE: &str = "factor_type needs to be totp, phone, or webauthn";

/// `EnrollFactorParams` in `internal/api/mfa.go`.
const ENROLL_FACTOR_PARAMS: GoStruct = GoStruct {
    name: "EnrollFactorParams",
    fields: &[
        ("friendly_name", Kind::Str),
        ("factor_type", Kind::Str),
        ("issuer", Kind::Str),
        ("phone", Kind::Str),
    ],
};

/// `VerifyFactorParams.code`. `challenge_id` and `webauthn` are checked
/// separately, see [`verify_factor`].
const VERIFY_FACTOR_PARAMS: GoStruct = GoStruct {
    name: "VerifyFactorParams",
    fields: &[("code", Kind::Str)],
};

/// `RecoveryCodesGenerateParams` in `internal/api/recovery_codes.go`.
const RECOVERY_CODES_GENERATE_PARAMS: GoStruct = GoStruct {
    name: "RecoveryCodesGenerateParams",
    fields: &[("friendly_name", Kind::Str)],
};

/// `RecoveryCodesVerifyParams` in `internal/api/recovery_codes.go`.
const RECOVERY_CODES_VERIFY_PARAMS: GoStruct = GoStruct {
    name: "RecoveryCodesVerifyParams",
    fields: &[("code", Kind::Str)],
};

pub(crate) fn router(state: AuthState) -> Router {
    Router::new()
        .route("/auth/v1/factors", post(enroll_factor))
        .route(
            "/auth/v1/factors/recovery-codes",
            get(recovery_codes_status)
                .post(recovery_codes_generate)
                .delete(recovery_codes_delete),
        )
        .route(
            "/auth/v1/factors/recovery-codes/verify",
            post(recovery_codes_verify),
        )
        .route(
            "/auth/v1/factors/recovery-codes/regenerate",
            post(recovery_codes_regenerate),
        )
        .route("/auth/v1/factors/:factor_id", delete(unenroll_factor))
        .route("/auth/v1/factors/:factor_id/verify", post(verify_factor))
        .route(
            "/auth/v1/factors/:factor_id/challenge",
            post(challenge_factor),
        )
        .with_state(state)
}

#[derive(Default, Deserialize)]
struct EnrollBody {
    #[serde(default, deserialize_with = "null_as_default")]
    factor_type: String,
    #[serde(default, deserialize_with = "null_as_default")]
    friendly_name: String,
    #[serde(default, deserialize_with = "null_as_default")]
    issuer: String,
    #[serde(default, deserialize_with = "null_as_default")]
    phone: String,
}

#[derive(Default, Deserialize)]
struct ChallengeBody {
    #[serde(default, deserialize_with = "null_as_default")]
    channel: String,
}

#[derive(Default, Deserialize)]
struct CodeBody {
    #[serde(default, deserialize_with = "null_as_default")]
    code: String,
}

#[derive(Default, Deserialize)]
struct FriendlyNameBody {
    #[serde(default, deserialize_with = "null_as_default")]
    friendly_name: String,
}

/// `ChallengeFactorParams.channel`. `webauthn` is ignored for phone and TOTP.
const CHALLENGE_PARAMS: GoStruct = GoStruct {
    name: "ChallengeFactorParams",
    fields: &[("channel", Kind::Str)],
};

// megabase:unit auth:route:POST /auth/v1/factors
/// `POST /factors` (`EnrollFactor`).
async fn enroll_factor(State(state): State<AuthState>, headers: HeaderMap, body: Body) -> Response {
    let authed = match require_factor_user(&state, &headers).await {
        Ok(authed) => authed,
        Err(response) => return *response,
    };
    let Some(session_id) = authed.session_id else {
        return unexpected("A valid session and a registered user are required to enroll a factor")
            .into_response();
    };
    let params: EnrollBody = match decode(body, &ENROLL_FACTOR_PARAMS).await {
        Ok(params) => params,
        Err(response) => return *response,
    };
    let mfa = &state.config.mfa;
    let (enabled, error_code, label) = match params.factor_type.as_str() {
        "phone" => (mfa.phone_enroll, "mfa_phone_enroll_not_enabled", "Phone"),
        "totp" => (mfa.totp_enroll, "mfa_totp_enroll_not_enabled", "TOTP"),
        "webauthn" => (
            mfa.webauthn_enroll,
            "mfa_webauthn_enroll_not_enabled",
            "WebAuthn",
        ),
        _ => return validation(BAD_FACTOR_TYPE).into_response(),
    };
    if !enabled {
        return GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            error_code,
            format!("MFA enroll is disabled for {label}"),
        )
        .into_response();
    }
    let enrolled = match params.factor_type.as_str() {
        "totp" => {
            let issuer = if params.issuer.is_empty() {
                match issuer_host(&state.config.site_url) {
                    Some(host) => host,
                    None => {
                        return unexpected("site url is improperly formatted").into_response();
                    }
                }
            } else {
                params.issuer
            };
            state
                .backend
                .enroll_totp(
                    &authed.user,
                    session_id,
                    &params.friendly_name,
                    &issuer,
                    mfa,
                )
                .await
        }
        "phone" => {
            if params.phone.is_empty() {
                return validation("Phone number required to enroll Phone factor").into_response();
            }
            let phone = match validate_phone(&params.phone) {
                Ok(phone) => phone,
                Err(error) => return error.into_response(),
            };
            state
                .backend
                .enroll_phone(
                    &authed.user,
                    session_id,
                    &params.friendly_name,
                    &phone,
                    &params.phone,
                    mfa,
                )
                .await
        }
        "webauthn" => {
            state
                .backend
                .enroll_webauthn(&authed.user, session_id, &params.friendly_name, mfa)
                .await
        }
        _ => return validation(BAD_FACTOR_TYPE).into_response(),
    };
    match enrolled {
        Ok(factor) => JsonOk(enroll_json(&factor)).into_response(),
        Err(error) => mfa_error(error),
    }
}

// megabase:unit auth:route:POST /auth/v1/factors/{factor_id}/challenge
/// `POST /factors/{factor_id}/challenge` (`ChallengeFactor`).
async fn challenge_factor(
    State(state): State<AuthState>,
    Path(factor_id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let (authed, factor) = match load_factor(&state, &headers, &factor_id).await {
        Ok(loaded) => loaded,
        Err(response) => return *response,
    };
    if let Err(response) = verify_gate(&state, &factor, "mfa_webauthn_verify_not_enabled") {
        return *response;
    }
    let ip = client_ip(&headers);
    let created = match factor.factor_type.as_str() {
        "totp" => {
            state
                .backend
                .challenge_totp(&authed.user, &factor, &ip, &state.config.mfa)
                .await
        }
        "phone" => {
            let bytes = match read_body(body).await {
                Ok(bytes) => bytes,
                Err(error) => return error.into_response(),
            };
            let params = match gojson::decode::<Option<ChallengeBody>>(&bytes, &CHALLENGE_PARAMS) {
                Ok(params) => params.unwrap_or_default(),
                Err(error) => return error.into_response(),
            };
            let channel = if params.channel.is_empty() {
                "sms"
            } else {
                params.channel.as_str()
            };
            if !crate::routes::valid_channel(channel, &state.config.sms_provider) {
                return validation(INVALID_CHANNEL).into_response();
            }
            if let Err(error) = state
                .backend
                .phone_challenge_allowed(&factor, &state.config.mfa)
                .await
            {
                return mfa_error(error);
            }
            let otp = match crate::store::phone_otp(state.config.mfa.phone_otp_length) {
                Ok(otp) => otp,
                Err(error) => return mfa_error(error),
            };
            if let Err(response) = send_mfa_sms(&state, &otp) {
                return *response;
            }
            state
                .backend
                .challenge_phone(&authed.user, &factor, &ip, &otp, &state.config.mfa)
                .await
        }
        "webauthn" => {
            return MegabaseNotImplemented::new(COMPONENT, UNIT_CHALLENGE).into_response();
        }
        _ => return validation(BAD_FACTOR_TYPE).into_response(),
    };
    match created {
        Ok(challenge) => JsonOk(json!({
            "id": challenge.id,
            "type": challenge.factor_type,
            "expires_at": challenge.expires_at,
        }))
        .into_response(),
        Err(error) => mfa_error(error),
    }
}

// megabase:unit auth:route:POST /auth/v1/factors/{factor_id}/verify
/// `POST /factors/{factor_id}/verify` (`VerifyFactor`).
///
/// `challenge_id` is a gofrs `uuid.UUID` and `webauthn` a struct in GoTrue,
/// so their decode errors carry text this crate does not reproduce; a body
/// that would hit one returns 501 instead of a guessed `bad_json` message.
async fn verify_factor(
    State(state): State<AuthState>,
    Path(factor_id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let (authed, factor) = match load_factor(&state, &headers, &factor_id).await {
        Ok(loaded) => loaded,
        Err(response) => return *response,
    };
    let bytes = match read_body(body).await {
        Ok(bytes) => bytes,
        Err(error) => return error.into_response(),
    };
    if !verify_fields_supported(&bytes) {
        return MegabaseNotImplemented::new(COMPONENT, UNIT_VERIFY).into_response();
    }
    let params = match gojson::decode::<Option<CodeBody>>(&bytes, &VERIFY_FACTOR_PARAMS) {
        Ok(params) => params.unwrap_or_default(),
        Err(error) => return error.into_response(),
    };
    if params.code.is_empty() && factor.factor_type != "webauthn" {
        return validation("Code needs to be non-empty").into_response();
    }
    // GoTrue reports a disabled WebAuthn verify with the enroll error code.
    if let Err(response) = verify_gate(&state, &factor, "mfa_webauthn_enroll_not_enabled") {
        return *response;
    }
    let Some(session_id) = authed.session_id else {
        return unexpected("Cannot read SessionId claim as UUID").into_response();
    };
    if factor.factor_type == "webauthn" {
        return MegabaseNotImplemented::new(COMPONENT, UNIT_VERIFY).into_response();
    }
    let challenge_id = challenge_id_from(&bytes);
    let ip = client_ip(&headers);
    let verified = match factor.factor_type.as_str() {
        "totp" => {
            let hook = mfa_hook(&state, &headers, "totp");
            state
                .backend
                .verify_totp(
                    &authed.user,
                    &factor,
                    FactorVerify {
                        session_id,
                        challenge_id,
                        code: &params.code,
                        ip: &ip,
                    },
                    &state.config.mfa,
                    hook.as_ref(),
                )
                .await
        }
        "phone" => {
            let hook = mfa_hook(&state, &headers, "mfa/phone");
            state
                .backend
                .verify_phone(
                    &authed.user,
                    &factor,
                    FactorVerify {
                        session_id,
                        challenge_id,
                        code: &params.code,
                        ip: &ip,
                    },
                    &state.config.mfa,
                    hook.as_ref(),
                )
                .await
        }
        _ => return validation(BAD_FACTOR_TYPE).into_response(),
    };
    match verified {
        Ok(upgraded) => session_response(&state, session_id, upgraded),
        Err(error) => mfa_error(error),
    }
}

// megabase:unit auth:route:DELETE /auth/v1/factors/{factor_id}
async fn unenroll_factor(
    State(state): State<AuthState>,
    Path(factor_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let (authed, factor) = match load_factor(&state, &headers, &factor_id).await {
        Ok(loaded) => loaded,
        Err(response) => return *response,
    };
    let Some(session_id) = authed.session_id else {
        return unexpected("A valid session and factor are required to unenroll a factor")
            .into_response();
    };
    if factor.is_verified() {
        match state.backend.session_is_aal2(session_id).await {
            Ok(true) => {}
            Ok(false) => {
                return GoTrueError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "insufficient_aal",
                    "AAL2 required to unenroll verified factor",
                )
                .into_response();
            }
            Err(error) => return internal(&error),
        }
    }
    let traits = json!({
        "factor_id": factor.id.to_string(),
        "factor_status": factor.status,
        "session_id": session_id.to_string(),
    });
    if let Err(error) = state
        .backend
        .unenroll_factor(&authed.user, &factor, "factor_unenrolled", traits)
        .await
    {
        return internal(&error);
    }
    JsonOk(json!({ "id": factor.id })).into_response()
}

// megabase:unit auth:route:GET /auth/v1/factors/recovery-codes
async fn recovery_codes_status(State(state): State<AuthState>, headers: HeaderMap) -> Response {
    let authed = match require_factor_user(&state, &headers).await {
        Ok(authed) => authed,
        Err(response) => return *response,
    };
    match state.backend.recovery_code_status(authed.user.id).await {
        Ok(Some(status)) => JsonOk(json!({
            "id": status.factor_id,
            "type": "recovery_code",
            "total": status.total,
            "remaining": status.remaining,
        }))
        .into_response(),
        Ok(None) => not_enrolled(),
        Err(error) => {
            tracing::error!(%error, "recovery code set lookup failed");
            unexpected("Database error finding recovery code set").into_response()
        }
    }
}

// megabase:unit auth:route:POST /auth/v1/factors/recovery-codes
async fn recovery_codes_generate(
    State(state): State<AuthState>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let authed = match require_factor_user(&state, &headers).await {
        Ok(authed) => authed,
        Err(response) => return *response,
    };
    let Some(session_id) = authed.session_id else {
        return unexpected(
            "A valid session and a registered user are required to generate recovery codes",
        )
        .into_response();
    };
    if !state.config.mfa.recovery_codes_enroll {
        return recovery_enroll_disabled();
    }
    // The body is optional; GoTrue parses it only when one was sent.
    let bytes = match read_body(body).await {
        Ok(bytes) => bytes,
        Err(error) => return error.into_response(),
    };
    let friendly_name = if bytes.is_empty() {
        String::new()
    } else {
        match gojson::decode::<Option<FriendlyNameBody>>(&bytes, &RECOVERY_CODES_GENERATE_PARAMS) {
            Ok(params) => params.unwrap_or_default().friendly_name,
            Err(error) => return error.into_response(),
        }
    };
    if let Err(response) = require_aal2(&state, session_id, "generate recovery codes").await {
        return *response;
    }
    match state.backend.recovery_code_status(authed.user.id).await {
        Ok(Some(_)) => {
            return GoTrueError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "mfa_verified_factor_exists",
                "Recovery codes are already enrolled for this user, use regenerate to rotate them",
            )
            .into_response();
        }
        Ok(None) => {}
        Err(error) => {
            tracing::error!(%error, "recovery code set lookup failed");
            return unexpected("Database error finding recovery code set").into_response();
        }
    }
    match state
        .backend
        .generate_recovery_codes(&authed.user, session_id, &friendly_name, &state.config.mfa)
        .await
    {
        Ok(issued) => JsonOk(recovery_json(&issued)).into_response(),
        Err(error) => mfa_error(error),
    }
}

// megabase:unit auth:route:POST /auth/v1/factors/recovery-codes/regenerate
async fn recovery_codes_regenerate(State(state): State<AuthState>, headers: HeaderMap) -> Response {
    let authed = match require_factor_user(&state, &headers).await {
        Ok(authed) => authed,
        Err(response) => return *response,
    };
    let Some(session_id) = authed.session_id else {
        return unexpected(
            "A valid session and a registered user are required to regenerate recovery codes",
        )
        .into_response();
    };
    if !state.config.mfa.recovery_codes_enroll {
        return recovery_enroll_disabled();
    }
    if let Err(response) = require_aal2(&state, session_id, "regenerate recovery codes").await {
        return *response;
    }
    match state.backend.recovery_code_status(authed.user.id).await {
        Ok(Some(_)) => {}
        Ok(None) => return not_enrolled(),
        Err(error) => {
            tracing::error!(%error, "recovery code set lookup failed");
            return unexpected("Database error finding recovery code set").into_response();
        }
    }
    match state
        .backend
        .regenerate_recovery_codes(&authed.user, &state.config.mfa)
        .await
    {
        Ok(issued) => JsonOk(recovery_json(&issued)).into_response(),
        Err(error) => mfa_error(error),
    }
}

// megabase:unit auth:route:POST /auth/v1/factors/recovery-codes/verify
async fn recovery_codes_verify(
    State(state): State<AuthState>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let authed = match require_factor_user(&state, &headers).await {
        Ok(authed) => authed,
        Err(response) => return *response,
    };
    let Some(session_id) = authed.session_id else {
        return unexpected(
            "A valid session and a registered user are required to verify recovery codes",
        )
        .into_response();
    };
    if !state.config.mfa.recovery_codes_verify {
        return GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "mfa_recovery_codes_verify_not_enabled",
            "MFA verification is disabled for recovery codes",
        )
        .into_response();
    }
    let params: CodeBody = match decode(body, &RECOVERY_CODES_VERIFY_PARAMS).await {
        Ok(params) => params,
        Err(response) => return *response,
    };
    if normalize_recovery_code(&params.code).is_empty() {
        return validation("Code needs to be non-empty").into_response();
    }
    let hook = mfa_hook(&state, &headers, "mfa/recovery_code");
    match state
        .backend
        .verify_recovery_code(
            &authed.user,
            session_id,
            &params.code,
            &state.config.mfa,
            hook.as_ref(),
        )
        .await
    {
        Ok(upgraded) => session_response(&state, session_id, upgraded),
        Err(error) => mfa_error(error),
    }
}

// megabase:unit auth:route:DELETE /auth/v1/factors/recovery-codes
async fn recovery_codes_delete(State(state): State<AuthState>, headers: HeaderMap) -> Response {
    let authed = match require_factor_user(&state, &headers).await {
        Ok(authed) => authed,
        Err(response) => return *response,
    };
    let Some(session_id) = authed.session_id else {
        return unexpected(
            "A valid session and a registered user are required to delete recovery codes",
        )
        .into_response();
    };
    if let Err(response) = require_aal2(&state, session_id, "delete recovery codes").await {
        return *response;
    }
    let factor = match state.backend.recovery_code_factor(authed.user.id).await {
        Ok(Some(factor)) => factor,
        Ok(None) => return not_enrolled(),
        Err(error) => {
            tracing::error!(%error, "recovery code factor lookup failed");
            return unexpected("Database error finding recovery code factor").into_response();
        }
    };
    let traits = json!({ "factor_id": factor.id.to_string() });
    if let Err(error) = state
        .backend
        .unenroll_factor(&authed.user, &factor, "recovery_codes_deleted", traits)
        .await
    {
        tracing::error!(%error, "recovery code factor delete failed");
        return unexpected("Database error deleting recovery code factor").into_response();
    }
    JsonOk(json!({ "id": factor.id })).into_response()
}

/// `requireAuthentication` then `requireNotAnonymous`.
pub(crate) async fn require_factor_user(
    state: &AuthState,
    headers: &HeaderMap,
) -> Result<Authed, Box<Response>> {
    let authed = require_user(state, headers).await?;
    if authed.claims.raw.get("is_anonymous") == Some(&Value::Bool(true)) {
        return Err(reject(GoTrueError::new(
            StatusCode::FORBIDDEN,
            "no_authorization",
            "Anonymous user not allowed to perform these actions",
        )));
    }
    Ok(authed)
}

/// `require_factor_user` then `loadFactor`.
async fn load_factor(
    state: &AuthState,
    headers: &HeaderMap,
    raw_id: &str,
) -> Result<(Authed, FactorRow), Box<Response>> {
    let authed = require_factor_user(state, headers).await?;
    let Some(factor_id) = parse_gofrs_uuid(raw_id) else {
        return Err(reject(GoTrueError::new(
            StatusCode::NOT_FOUND,
            "validation_failed",
            "factor_id must be an UUID",
        )));
    };
    match state
        .backend
        .find_owned_factor(authed.user.id, factor_id)
        .await
    {
        Ok(Some(factor)) => Ok((authed, factor)),
        Ok(None) => Err(reject(GoTrueError::new(
            StatusCode::NOT_FOUND,
            "mfa_factor_not_found",
            "Factor not found",
        ))),
        Err(error) => {
            tracing::error!(%error, "factor lookup failed");
            Err(reject(unexpected("Database error loading factor")))
        }
    }
}

fn reject(response: impl IntoResponse) -> Box<Response> {
    Box::new(response.into_response())
}

/// gofrs `uuid.FromString`: canonical, braced, `urn:uuid:` prefixed, or
/// 32 hex digits without hyphens.
pub(crate) fn parse_gofrs_uuid(raw: &str) -> Option<Uuid> {
    let canonical = |text: &str| -> Option<Uuid> {
        let bytes = text.as_bytes();
        let hyphenated =
            text.len() == 36 && [8, 13, 18, 23].iter().all(|&index| bytes[index] == b'-');
        let plain = text.len() == 32;
        if !(hyphenated || plain) || !text.bytes().all(|b| b == b'-' || b.is_ascii_hexdigit()) {
            return None;
        }
        Uuid::parse_str(text).ok()
    };
    match raw.len() {
        32 | 36 => canonical(raw),
        38 if raw.starts_with('{') && raw.ends_with('}') => canonical(&raw[1..37]),
        45 if raw.starts_with("urn:uuid:") => canonical(&raw[9..]),
        _ => None,
    }
}

/// Verify switch shared by challenge and verify. `webauthn_code` is the
/// error code GoTrue uses for a disabled WebAuthn verify on that route.
fn verify_gate(
    state: &AuthState,
    factor: &FactorRow,
    webauthn_code: &'static str,
) -> Result<(), Box<Response>> {
    let mfa = &state.config.mfa;
    let (enabled, error_code, label) = match factor.factor_type.as_str() {
        "phone" => (mfa.phone_verify, "mfa_phone_verify_not_enabled", "Phone"),
        "totp" => (mfa.totp_verify, "mfa_totp_verify_not_enabled", "TOTP"),
        "webauthn" => (mfa.webauthn_verify, webauthn_code, "WebAuthn"),
        _ => return Err(reject(validation(BAD_FACTOR_TYPE))),
    };
    if enabled {
        Ok(())
    } else {
        Err(reject(GoTrueError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            error_code,
            format!("MFA verification is disabled for {label}"),
        )))
    }
}

/// `challenge_id` must be absent, `null`, or a canonical UUID string, and
/// `webauthn` absent, `null`, or an object whose `type` is a string or
/// `null`. Field names match case-insensitively, like Go's decoder.
fn verify_fields_supported(bytes: &[u8]) -> bool {
    let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(bytes) else {
        // Syntax errors and non-object bodies go through `gojson::decode`.
        return true;
    };
    object.iter().all(|(key, value)| {
        if key.eq_ignore_ascii_case("challenge_id") {
            match value {
                Value::Null => true,
                Value::String(text) => parse_gofrs_uuid(text).is_some(),
                _ => false,
            }
        } else if key.eq_ignore_ascii_case("webauthn") {
            match value {
                Value::Null => true,
                Value::Object(inner) => inner.iter().all(|(key, value)| {
                    !key.eq_ignore_ascii_case("type")
                        || matches!(value, Value::Null | Value::String(_))
                }),
                _ => false,
            }
        } else {
            true
        }
    })
}

async fn require_aal2(
    state: &AuthState,
    session_id: Uuid,
    action: &str,
) -> Result<(), Box<Response>> {
    match state.backend.session_is_aal2(session_id).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(reject(GoTrueError::new(
            StatusCode::FORBIDDEN,
            "insufficient_aal",
            format!("AAL2 required to {action}"),
        ))),
        Err(error) => Err(reject(internal(&error))),
    }
}

/// Read and decode a required JSON body; a top-level `null` is the zero value.
async fn decode<T: Default + serde::de::DeserializeOwned>(
    body: Body,
    target: &GoStruct,
) -> Result<T, Box<Response>> {
    let bytes = read_body(body).await.map_err(reject)?;
    gojson::decode::<Option<T>>(&bytes, target)
        .map(Option::unwrap_or_default)
        .map_err(reject)
}

/// `crypto.NormalizeRecoveryCode`: drop hyphens and white space, lowercase.
fn normalize_recovery_code(input: &str) -> String {
    input
        .chars()
        .filter(|ch| *ch != '-' && !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn not_enrolled() -> Response {
    GoTrueError::new(StatusCode::NOT_FOUND, "mfa_factor_not_found", NOT_ENROLLED).into_response()
}

fn recovery_enroll_disabled() -> Response {
    GoTrueError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "mfa_recovery_codes_enroll_not_enabled",
        "MFA enroll is disabled for recovery codes",
    )
    .into_response()
}

/// A raw store error. GoTrue's handler answers any non-HTTP error this way.
fn internal(error: &crate::store::StoreError) -> Response {
    tracing::error!(%error, "factor store operation failed");
    unexpected("Unexpected failure, please check server logs for more information").into_response()
}

fn mfa_error(error: MfaError) -> Response {
    match error {
        MfaError::Store(crate::store::StoreError::Hook(error)) => (*error).into_response(),
        MfaError::Store(error) => internal(&error),
        MfaError::Status {
            status,
            code,
            message,
        } => {
            let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            GoTrueError::new(status, code, message).into_response()
        }
    }
}

fn enroll_json(factor: &EnrolledFactor) -> Value {
    let mut body = json!({
        "id": factor.id,
        "type": factor.factor_type,
        "friendly_name": factor.friendly_name,
    });
    if factor.factor_type == "totp" {
        body["totp"] = json!({
            "qr_code": factor.qr_code,
            "secret": factor.secret,
            "uri": factor.uri,
        });
    }
    if factor.factor_type == "phone" {
        body["phone"] = json!(factor.phone);
    }
    body
}

fn recovery_json(issued: &RecoveryIssue) -> Value {
    json!({
        "id": issued.id,
        "type": "recovery_code",
        "friendly_name": issued.friendly_name,
        "total": issued.codes.len(),
        "codes": issued.codes,
    })
}

fn session_response(state: &AuthState, session_id: Uuid, upgraded: UpgradedSession) -> Response {
    let Some(jwt) = state.jwt.clone() else {
        return unexpected("Server lacks JWT secret").into_response();
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let expires_in = state.config.jwt_exp_seconds;
    let claims = match &upgraded.hook_claims {
        Some(claims) => claims.clone(),
        None => mfa_access_claims(
            &upgraded.user,
            session_id,
            &upgraded.amr,
            &state.config.jwt_issuer,
            now,
            expires_in,
        ),
    };
    let access_token = match jwt.sign(&claims) {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "signing access token failed");
            return unexpected("error generating jwt token").into_response();
        }
    };
    let issued = IssuedSession {
        user: upgraded.user,
        refresh_token: upgraded.refresh_token,
        session_id,
        amr_at: SystemTime::now(),
        amr_method: "totp".into(),
        hook_claims: None,
    };
    JsonOk(session_json(&issued, &access_token, expires_in, now)).into_response()
}

/// `GetIPAddress` without a socket address: `X-Forwarded-For`, else
/// `0.0.0.0` so `mfa_challenges.ip_address` (inet) accepts the value.
/// Challenge and verify use the same address, so they match.
fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .into_iter()
        .flat_map(|header| header.split(','))
        .map(str::trim)
        .find_map(|entry| entry.parse::<IpAddr>().ok())
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "0.0.0.0".to_string())
}

fn challenge_id_from(bytes: &[u8]) -> Uuid {
    let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(bytes) else {
        return Uuid::nil();
    };
    object
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("challenge_id"))
        .and_then(|(_, value)| value.as_str().and_then(parse_gofrs_uuid))
        .unwrap_or(Uuid::nil())
}

/// `generateSMSFromTemplate`, then `GetSmsProvider`.
///
/// No SMS client is ported. A missing provider is GoTrue's 500. A configured
/// provider would send the message, so that step is 501.
fn send_mfa_sms(state: &AuthState, otp: &str) -> Result<(), Box<Response>> {
    let template = &state.config.mfa.phone_sms_template;
    let message = template
        .replace("{{ .Code }}", otp)
        .replace("{{.Code}}", otp)
        .replace("{{ .Code}}", otp)
        .replace("{{.Code }}", otp);
    if message.contains("{{") {
        return Err(Box::new(
            unexpected("error generating sms template").into_response(),
        ));
    }
    let _ = message;
    if state.config.sms_provider.is_empty() {
        return Err(Box::new(
            unexpected("Failed to get SMS provider").into_response(),
        ));
    }
    Err(Box::new(
        MegabaseNotImplemented::new(COMPONENT, UNIT_CHALLENGE).into_response(),
    ))
}

fn mfa_hook<'a>(
    state: &'a AuthState,
    headers: &HeaderMap,
    method: &'static str,
) -> Option<TokenHookCall<'a>> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    TokenHookCall::new(
        state.config.custom_access_token_hook.as_ref(),
        headers,
        method,
        &state.config.jwt_issuer,
        now,
        state.config.jwt_exp_seconds,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;
    use crate::store::Backend;
    use axum::body::to_bytes;
    use axum::http::Request;
    use megabase_core::Hs256;
    use tower::ServiceExt;

    const SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";
    const EMAIL: &str = "mfa@example.com";

    struct Fixture {
        app: Router,
        backend: Backend,
        token: String,
        session_id: Uuid,
    }

    async fn fixture(configure: impl FnOnce(&mut AuthConfig)) -> Fixture {
        fixture_hook(configure, None).await
    }

    async fn fixture_hook(
        configure: impl FnOnce(&mut AuthConfig),
        hook: Option<std::sync::Arc<crate::hooks::MemoryHook>>,
    ) -> Fixture {
        let backend = Backend::memory();
        if let Some(function) = hook {
            backend.set_access_token_hook_for_test(function).await;
        }
        let mut config = AuthConfig::reference_defaults();
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

    /// Re-sign the fixture token with `edit` applied to its claims.
    fn resign(token: &str, edit: impl FnOnce(&mut serde_json::Map<String, Value>)) -> String {
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

    const ROUTES: &[(&str, &str)] = &[
        ("POST", "/auth/v1/factors"),
        ("GET", "/auth/v1/factors/recovery-codes"),
        ("POST", "/auth/v1/factors/recovery-codes"),
        ("DELETE", "/auth/v1/factors/recovery-codes"),
        ("POST", "/auth/v1/factors/recovery-codes/verify"),
        ("POST", "/auth/v1/factors/recovery-codes/regenerate"),
        (
            "DELETE",
            "/auth/v1/factors/00000000-0000-0000-0000-000000000001",
        ),
        (
            "POST",
            "/auth/v1/factors/00000000-0000-0000-0000-000000000001/verify",
        ),
        (
            "POST",
            "/auth/v1/factors/00000000-0000-0000-0000-000000000001/challenge",
        ),
    ];

    #[tokio::test]
    async fn every_route_requires_a_non_anonymous_user() {
        let fx = fixture(|_| {}).await;
        let anonymous = resign(&fx.token, |claims| {
            claims.insert("is_anonymous".into(), Value::Bool(true));
        });
        for (method, path) in ROUTES {
            let result = call(&fx.app, method, path, Some("{}"), None).await;
            assert_error(
                &result,
                StatusCode::UNAUTHORIZED,
                "no_authorization",
                "This endpoint requires a valid Bearer token",
            );
            let result = call(&fx.app, method, path, Some("{}"), Some(&anonymous)).await;
            assert_error(
                &result,
                StatusCode::FORBIDDEN,
                "no_authorization",
                "Anonymous user not allowed to perform these actions",
            );
        }
    }

    #[tokio::test]
    async fn load_factor_rejects_bad_and_foreign_ids() {
        let fx = fixture(|_| {}).await;
        for (method, suffix) in [("DELETE", ""), ("POST", "/verify"), ("POST", "/challenge")] {
            let path = format!("/auth/v1/factors/not-a-uuid{suffix}");
            let result = call(&fx.app, method, &path, Some("{}"), Some(&fx.token)).await;
            assert_error(
                &result,
                StatusCode::NOT_FOUND,
                "validation_failed",
                "factor_id must be an UUID",
            );
            let path = format!("/auth/v1/factors/{}{suffix}", Uuid::new_v4());
            let result = call(&fx.app, method, &path, Some("{}"), Some(&fx.token)).await;
            assert_error(
                &result,
                StatusCode::NOT_FOUND,
                "mfa_factor_not_found",
                "Factor not found",
            );
        }
    }

    #[test]
    fn gofrs_uuid_forms() {
        let id = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";
        assert!(parse_gofrs_uuid(id).is_some());
        assert!(parse_gofrs_uuid(&id.to_uppercase()).is_some());
        assert!(parse_gofrs_uuid(&format!("{{{id}}}")).is_some());
        assert!(parse_gofrs_uuid(&format!("urn:uuid:{id}")).is_some());
        assert!(parse_gofrs_uuid(&id.replace('-', "")).is_some());
        assert!(parse_gofrs_uuid("6ba7b8109dad-11d1-80b4-00c04fd430c8-").is_none());
        assert!(parse_gofrs_uuid(&"é".repeat(18)).is_none());
        assert!(parse_gofrs_uuid(&format!("{{{}}}", "é".repeat(18))).is_none());
        assert!(parse_gofrs_uuid("").is_none());
    }

    #[tokio::test]
    async fn enroll_follows_the_factor_type_gates() {
        let fx = fixture(|_| {}).await;
        let enroll = |body: &'static str| {
            let app = fx.app.clone();
            let token = fx.token.clone();
            async move { call(&app, "POST", "/auth/v1/factors", Some(body), Some(&token)).await }
        };
        assert_error(
            &enroll(r#"{"factor_type":"sms"}"#).await,
            StatusCode::BAD_REQUEST,
            "validation_failed",
            BAD_FACTOR_TYPE,
        );
        assert_error(
            &enroll("null").await,
            StatusCode::BAD_REQUEST,
            "validation_failed",
            BAD_FACTOR_TYPE,
        );
        assert_error(
            &enroll(r#"{"factor_type":"phone","phone":"+15555550100"}"#).await,
            StatusCode::UNPROCESSABLE_ENTITY,
            "mfa_phone_enroll_not_enabled",
            "MFA enroll is disabled for Phone",
        );
        assert_error(
            &enroll(r#"{"factor_type":"webauthn"}"#).await,
            StatusCode::UNPROCESSABLE_ENTITY,
            "mfa_webauthn_enroll_not_enabled",
            "MFA enroll is disabled for WebAuthn",
        );
        let enrolled = enroll(r#"{"FACTOR_TYPE":"totp"}"#).await;
        assert_eq!(enrolled.0, StatusCode::OK, "{}", enrolled.1);
        assert_eq!(enrolled.1["type"], "totp");
        assert!(enrolled.1["totp"]["secret"].as_str().unwrap().len() >= 16);
        assert!(enrolled.1["totp"]["uri"]
            .as_str()
            .unwrap()
            .starts_with("otpauth://totp/"));
        let result = enroll(r#"{"factor_type":1}"#).await;
        assert_eq!(result.0, StatusCode::BAD_REQUEST);
        assert_eq!(result.1["error_code"], "bad_json");

        let off = fixture(|config| config.mfa.totp_enroll = false).await;
        let result = call(
            &off.app,
            "POST",
            "/auth/v1/factors",
            Some(r#"{"factor_type":"totp"}"#),
            Some(&off.token),
        )
        .await;
        assert_error(
            &result,
            StatusCode::UNPROCESSABLE_ENTITY,
            "mfa_totp_enroll_not_enabled",
            "MFA enroll is disabled for TOTP",
        );

        let sessionless = resign(&fx.token, |claims| {
            claims.remove("session_id");
        });
        let result = call(
            &fx.app,
            "POST",
            "/auth/v1/factors",
            Some(r#"{"factor_type":"totp"}"#),
            Some(&sessionless),
        )
        .await;
        assert_error(
            &result,
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected_failure",
            "A valid session and a registered user are required to enroll a factor",
        );
    }

    #[tokio::test]
    async fn challenge_and_verify_check_the_verify_gates() {
        let fx = fixture(|_| {}).await;
        let backend = &fx.backend;
        let totp = backend
            .add_factor_for_test(EMAIL, "totp", "unverified")
            .await;
        let phone = backend
            .add_factor_for_test(EMAIL, "phone", "unverified")
            .await;
        let webauthn = backend
            .add_factor_for_test(EMAIL, "webauthn", "unverified")
            .await;
        let recovery = backend
            .add_factor_for_test(EMAIL, "recovery_code", "verified")
            .await;
        let post = |id: Uuid, action: &'static str, body: &'static str| {
            let path = format!("/auth/v1/factors/{id}/{action}");
            let app = fx.app.clone();
            let token = fx.token.clone();
            async move { call(&app, "POST", &path, Some(body), Some(&token)).await }
        };

        let challenged = post(totp, "challenge", "").await;
        assert_eq!(challenged.0, StatusCode::OK, "{}", challenged.1);
        assert_eq!(challenged.1["type"], "totp");
        assert!(challenged.1["expires_at"].is_i64());
        assert_error(
            &post(phone, "challenge", "").await,
            StatusCode::UNPROCESSABLE_ENTITY,
            "mfa_phone_verify_not_enabled",
            "MFA verification is disabled for Phone",
        );
        assert_error(
            &post(webauthn, "challenge", "").await,
            StatusCode::UNPROCESSABLE_ENTITY,
            "mfa_webauthn_verify_not_enabled",
            "MFA verification is disabled for WebAuthn",
        );
        assert_error(
            &post(recovery, "challenge", "").await,
            StatusCode::BAD_REQUEST,
            "validation_failed",
            BAD_FACTOR_TYPE,
        );

        assert_error(
            &post(totp, "verify", r#"{"code":""}"#).await,
            StatusCode::BAD_REQUEST,
            "validation_failed",
            "Code needs to be non-empty",
        );
        let result = post(totp, "verify", "").await;
        assert_eq!(result.1["error_code"], "bad_json", "{}", result.1);
        assert_error(
            &post(totp, "verify", r#"{"code":"123456"}"#).await,
            StatusCode::UNPROCESSABLE_ENTITY,
            "mfa_factor_not_found",
            "MFA factor with the provided challenge ID not found",
        );
        assert_501(
            &post(totp, "verify", r#"{"challenge_id":"nope","code":"123456"}"#).await,
            UNIT_VERIFY,
        );
        assert_error(
            &post(phone, "verify", r#"{"code":"123456"}"#).await,
            StatusCode::UNPROCESSABLE_ENTITY,
            "mfa_phone_verify_not_enabled",
            "MFA verification is disabled for Phone",
        );
        // A WebAuthn factor needs no code; GoTrue reports the enroll error code.
        assert_error(
            &post(webauthn, "verify", "{}").await,
            StatusCode::UNPROCESSABLE_ENTITY,
            "mfa_webauthn_enroll_not_enabled",
            "MFA verification is disabled for WebAuthn",
        );
        assert_error(
            &post(recovery, "verify", r#"{"code":"x"}"#).await,
            StatusCode::BAD_REQUEST,
            "validation_failed",
            BAD_FACTOR_TYPE,
        );
    }

    #[tokio::test]
    async fn totp_enroll_challenge_verify_upgrades_aal() {
        let fx = fixture(|_| {}).await;
        let enrolled = call(
            &fx.app,
            "POST",
            "/auth/v1/factors",
            Some(r#"{"factor_type":"totp","friendly_name":"phone-app","issuer":"megabase"}"#),
            Some(&fx.token),
        )
        .await;
        assert_eq!(enrolled.0, StatusCode::OK, "{}", enrolled.1);
        let factor_id = enrolled.1["id"].as_str().unwrap();
        let secret = enrolled.1["totp"]["secret"].as_str().unwrap();
        assert!(enrolled.1["totp"]["uri"]
            .as_str()
            .unwrap()
            .contains("issuer=megabase"));
        let challenged = call(
            &fx.app,
            "POST",
            &format!("/auth/v1/factors/{factor_id}/challenge"),
            Some(""),
            Some(&fx.token),
        )
        .await;
        assert_eq!(challenged.0, StatusCode::OK, "{}", challenged.1);
        let challenge_id = challenged.1["id"].as_str().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let code = crate::totp::current_code(secret, now);
        let verified = call(
            &fx.app,
            "POST",
            &format!("/auth/v1/factors/{factor_id}/verify"),
            Some(&format!(
                r#"{{"challenge_id":"{challenge_id}","code":"{code}"}}"#
            )),
            Some(&fx.token),
        )
        .await;
        assert_eq!(verified.0, StatusCode::OK, "{}", verified.1);
        assert!(verified.1["access_token"].is_string());
        assert!(verified.1["refresh_token"].is_string());
        let claims = megabase_core::Hs256::new(SECRET.as_bytes())
            .unwrap()
            .verify(verified.1["access_token"].as_str().unwrap())
            .unwrap();
        assert_eq!(claims.raw["aal"], "aal2");
        assert_eq!(claims.raw["amr"][0]["method"], "totp");
        assert_eq!(claims.raw["amr"][1]["method"], "password");
        assert!(fx.backend.session_is_aal2(fx.session_id).await.unwrap());

        let wrong = call(
            &fx.app,
            "POST",
            &format!("/auth/v1/factors/{factor_id}/verify"),
            Some(&format!(
                r#"{{"challenge_id":"{challenge_id}","code":"000000"}}"#
            )),
            Some(&fx.token),
        )
        .await;
        assert_error(
            &wrong,
            StatusCode::UNPROCESSABLE_ENTITY,
            "mfa_ip_address_mismatch",
            "Challenge and verify IP addresses mismatch.",
        );
    }

    #[tokio::test]
    async fn totp_verify_signs_custom_access_token_hook_claims() {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = std::sync::Arc::clone(&events);
        let function: std::sync::Arc<crate::hooks::MemoryHook> =
            std::sync::Arc::new(move |event: &Value| {
                seen.lock().expect("hook events").push(event.clone());
                let mut claims = event["claims"].clone();
                claims["user_role"] = json!("moderator");
                Some(json!({ "claims": claims }).to_string())
            });
        let fx = fixture_hook(
            |config| {
                config.custom_access_token_hook =
                    Some(crate::hooks::AccessTokenHook::pg_function_for_test(
                        "public",
                        "custom_access_token_hook",
                    ));
            },
            Some(function),
        )
        .await;
        events.lock().expect("hook events").clear();
        let enrolled = call(
            &fx.app,
            "POST",
            "/auth/v1/factors",
            Some(r#"{"factor_type":"totp","friendly_name":"hook","issuer":"megabase"}"#),
            Some(&fx.token),
        )
        .await;
        assert_eq!(enrolled.0, StatusCode::OK, "{}", enrolled.1);
        let factor_id = enrolled.1["id"].as_str().unwrap();
        let secret = enrolled.1["totp"]["secret"].as_str().unwrap();
        let challenged = call(
            &fx.app,
            "POST",
            &format!("/auth/v1/factors/{factor_id}/challenge"),
            Some(""),
            Some(&fx.token),
        )
        .await;
        assert_eq!(challenged.0, StatusCode::OK, "{}", challenged.1);
        let challenge_id = challenged.1["id"].as_str().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let code = crate::totp::current_code(secret, now);
        let verified = call(
            &fx.app,
            "POST",
            &format!("/auth/v1/factors/{factor_id}/verify"),
            Some(&format!(
                r#"{{"challenge_id":"{challenge_id}","code":"{code}"}}"#
            )),
            Some(&fx.token),
        )
        .await;
        assert_eq!(verified.0, StatusCode::OK, "{}", verified.1);
        let claims = megabase_core::Hs256::new(SECRET.as_bytes())
            .unwrap()
            .verify(verified.1["access_token"].as_str().unwrap())
            .unwrap();
        assert_eq!(claims.raw["aal"], "aal2");
        assert_eq!(claims.raw["user_role"], "moderator");
        let events = events.lock().expect("hook events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["authentication_method"], "totp");
        assert_eq!(events[0]["claims"]["aal"], "aal2");
        assert_eq!(events[0]["claims"]["amr"][0]["method"], "totp");
    }

    #[tokio::test]
    async fn phone_challenge_reports_missing_provider_and_unported_send() {
        let body = r#"{"factor_type":"phone","phone":"+15555550100"}"#;
        let missing = fixture(|config| {
            config.mfa.phone_enroll = true;
            config.mfa.phone_verify = true;
        })
        .await;
        let enrolled = call(
            &missing.app,
            "POST",
            "/auth/v1/factors",
            Some(body),
            Some(&missing.token),
        )
        .await;
        assert_eq!(enrolled.0, StatusCode::OK, "{}", enrolled.1);
        let factor_id = enrolled.1["id"].as_str().unwrap();
        let challenged = call(
            &missing.app,
            "POST",
            &format!("/auth/v1/factors/{factor_id}/challenge"),
            Some("{}"),
            Some(&missing.token),
        )
        .await;
        assert_error(
            &challenged,
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected_failure",
            "Failed to get SMS provider",
        );

        let configured = fixture(|config| {
            config.mfa.phone_enroll = true;
            config.mfa.phone_verify = true;
            config.sms_provider = "twilio".into();
        })
        .await;
        let enrolled = call(
            &configured.app,
            "POST",
            "/auth/v1/factors",
            Some(body),
            Some(&configured.token),
        )
        .await;
        assert_eq!(enrolled.0, StatusCode::OK, "{}", enrolled.1);
        let factor_id = enrolled.1["id"].as_str().unwrap();
        let challenged = call(
            &configured.app,
            "POST",
            &format!("/auth/v1/factors/{factor_id}/challenge"),
            Some("{}"),
            Some(&configured.token),
        )
        .await;
        assert_501(&challenged, UNIT_CHALLENGE);
    }

    #[tokio::test]
    async fn unenroll_requires_aal2_for_verified_factors() {
        let fx = fixture(|_| {}).await;
        let unverified = fx
            .backend
            .add_factor_for_test(EMAIL, "totp", "unverified")
            .await;
        let path = format!("/auth/v1/factors/{unverified}");
        let (status, body) = call(&fx.app, "DELETE", &path, None, Some(&fx.token)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body, json!({ "id": unverified }));
        assert!(!fx.backend.has_factor_for_test(unverified).await);

        let verified = fx
            .backend
            .add_factor_for_test(EMAIL, "totp", "verified")
            .await;
        let path = format!("/auth/v1/factors/{verified}");
        let result = call(&fx.app, "DELETE", &path, None, Some(&fx.token)).await;
        assert_error(
            &result,
            StatusCode::UNPROCESSABLE_ENTITY,
            "insufficient_aal",
            "AAL2 required to unenroll verified factor",
        );
        assert!(fx.backend.has_factor_for_test(verified).await);

        fx.backend.set_aal2_for_test(fx.session_id, verified).await;
        let (status, body) = call(&fx.app, "DELETE", &path, None, Some(&fx.token)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["id"], verified.to_string());
        assert!(!fx.backend.has_factor_for_test(verified).await);
        // Removing the factor downgraded the session it raised.
        assert!(!fx.backend.session_is_aal2(fx.session_id).await.unwrap());
    }

    #[tokio::test]
    async fn recovery_code_status_reports_counts_only() {
        let fx = fixture(|_| {}).await;
        let path = "/auth/v1/factors/recovery-codes";
        assert_error(
            &call(&fx.app, "GET", path, None, Some(&fx.token)).await,
            StatusCode::NOT_FOUND,
            "mfa_factor_not_found",
            NOT_ENROLLED,
        );
        let factor = fx
            .backend
            .add_factor_for_test(EMAIL, "recovery_code", "verified")
            .await;
        fx.backend
            .add_recovery_set_for_test(EMAIL, factor, 10, 7)
            .await;
        let (status, body) = call(&fx.app, "GET", path, None, Some(&fx.token)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body,
            json!({ "id": factor, "type": "recovery_code", "total": 10, "remaining": 7 })
        );
    }

    #[tokio::test]
    async fn recovery_code_writes_are_gated_by_default() {
        let fx = fixture(|_| {}).await;
        for path in [
            "/auth/v1/factors/recovery-codes",
            "/auth/v1/factors/recovery-codes/regenerate",
        ] {
            assert_error(
                &call(&fx.app, "POST", path, Some("{}"), Some(&fx.token)).await,
                StatusCode::UNPROCESSABLE_ENTITY,
                "mfa_recovery_codes_enroll_not_enabled",
                "MFA enroll is disabled for recovery codes",
            );
        }
        assert_error(
            &call(
                &fx.app,
                "POST",
                "/auth/v1/factors/recovery-codes/verify",
                Some(r#"{"code":"abc"}"#),
                Some(&fx.token),
            )
            .await,
            StatusCode::UNPROCESSABLE_ENTITY,
            "mfa_recovery_codes_verify_not_enabled",
            "MFA verification is disabled for recovery codes",
        );
        assert_error(
            &call(
                &fx.app,
                "DELETE",
                "/auth/v1/factors/recovery-codes",
                None,
                Some(&fx.token),
            )
            .await,
            StatusCode::FORBIDDEN,
            "insufficient_aal",
            "AAL2 required to delete recovery codes",
        );
    }

    #[tokio::test]
    async fn enabled_recovery_codes_check_aal_and_enrollment() {
        let fx = fixture(|config| {
            config.mfa.recovery_codes_enroll = true;
            config.mfa.recovery_codes_verify = true;
        })
        .await;
        let token = Some(fx.token.as_str());
        let generate = "/auth/v1/factors/recovery-codes";
        let regenerate = "/auth/v1/factors/recovery-codes/regenerate";
        let verify = "/auth/v1/factors/recovery-codes/verify";
        let result = call(
            &fx.app,
            "POST",
            generate,
            Some(r#"{"friendly_name":1}"#),
            token,
        )
        .await;
        assert_eq!(result.1["error_code"], "bad_json", "{}", result.1);
        assert_error(
            &call(&fx.app, "POST", generate, None, token).await,
            StatusCode::FORBIDDEN,
            "insufficient_aal",
            "AAL2 required to generate recovery codes",
        );
        assert_error(
            &call(&fx.app, "POST", regenerate, None, token).await,
            StatusCode::FORBIDDEN,
            "insufficient_aal",
            "AAL2 required to regenerate recovery codes",
        );
        assert_error(
            &call(&fx.app, "POST", verify, Some(r#"{"code":" - \n"}"#), token).await,
            StatusCode::BAD_REQUEST,
            "validation_failed",
            "Code needs to be non-empty",
        );
        assert_error(
            &call(
                &fx.app,
                "POST",
                verify,
                Some(r#"{"code":"abcd-efgh"}"#),
                token,
            )
            .await,
            StatusCode::UNPROCESSABLE_ENTITY,
            "mfa_verification_failed",
            "Invalid recovery code entered",
        );

        let totp = fx
            .backend
            .add_factor_for_test(EMAIL, "totp", "verified")
            .await;
        fx.backend.set_aal2_for_test(fx.session_id, totp).await;
        assert_error(
            &call(&fx.app, "POST", regenerate, None, token).await,
            StatusCode::NOT_FOUND,
            "mfa_factor_not_found",
            NOT_ENROLLED,
        );
        assert_error(
            &call(&fx.app, "DELETE", generate, None, token).await,
            StatusCode::NOT_FOUND,
            "mfa_factor_not_found",
            NOT_ENROLLED,
        );
        let generated = call(&fx.app, "POST", generate, None, token).await;
        assert_eq!(generated.0, StatusCode::OK, "{}", generated.1);
        assert_eq!(generated.1["type"], "recovery_code");
        assert_eq!(generated.1["friendly_name"], "Recovery codes");
        assert_eq!(generated.1["codes"].as_array().map(Vec::len), Some(10));
        let recovery = generated.1["id"].as_str().unwrap().to_string();
        assert_error(
            &call(&fx.app, "POST", generate, None, token).await,
            StatusCode::UNPROCESSABLE_ENTITY,
            "mfa_verified_factor_exists",
            "Recovery codes are already enrolled for this user, use regenerate to rotate them",
        );
        let rotated = call(&fx.app, "POST", regenerate, None, token).await;
        assert_eq!(rotated.0, StatusCode::OK, "{}", rotated.1);
        assert_eq!(rotated.1["total"], 10);
        assert_eq!(rotated.1["id"], recovery);
        let code = rotated.1["codes"][0].as_str().unwrap();
        // A second AAL1 session redeems the code. The original session stays
        // bound to the TOTP factor, so deleting the recovery factor later
        // does not drop it from AAL2.
        let login = call(
            &fx.app,
            "POST",
            "/auth/v1/token?grant_type=password",
            Some(&json!({ "email": EMAIL, "password": "secret-pass" }).to_string()),
            None,
        )
        .await;
        assert_eq!(login.0, StatusCode::OK, "{}", login.1);
        let aal1 = login.1["access_token"].as_str().unwrap();
        let verified = call(
            &fx.app,
            "POST",
            verify,
            Some(&format!(r#"{{"code":"{code}"}}"#)),
            Some(aal1),
        )
        .await;
        assert_eq!(verified.0, StatusCode::OK, "{}", verified.1);
        let claims = megabase_core::Hs256::new(SECRET.as_bytes())
            .unwrap()
            .verify(verified.1["access_token"].as_str().unwrap())
            .unwrap();
        assert_eq!(claims.raw["aal"], "aal2");
        assert_eq!(claims.raw["amr"][0]["method"], "mfa/recovery_code");
        let (status, body) = call(&fx.app, "DELETE", generate, None, token).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["id"], recovery);
        assert_error(
            &call(&fx.app, "GET", generate, None, token).await,
            StatusCode::NOT_FOUND,
            "mfa_factor_not_found",
            NOT_ENROLLED,
        );
        // The TOTP factor still holds the session at AAL2.
        assert!(fx.backend.session_is_aal2(fx.session_id).await.unwrap());
    }

    #[tokio::test]
    async fn totp_round_trip_persists_on_postgres() {
        let Ok(url) = std::env::var("MEGABASE_AUTH_TEST_DATABASE") else {
            eprintln!("MEGABASE_AUTH_TEST_DATABASE unset; postgres MFA integration not run");
            return;
        };
        crate::install_schema(&url)
            .await
            .expect("install auth schema");
        let backend = crate::store::Backend::connect(&url).await.expect("connect");
        let jwt = megabase_core::Hs256::new(SECRET.as_bytes()).unwrap();
        let app = crate::router_with_state(AuthState::new(
            crate::AuthConfig::reference_defaults(),
            Some(jwt.clone()),
            backend,
        ));
        let email = format!(
            "mfa-{}@example.com",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(1)
        );
        let signup = call(
            &app,
            "POST",
            "/auth/v1/signup",
            Some(&format!(
                r#"{{"email":"{email}","password":"secret-pass"}}"#
            )),
            None,
        )
        .await;
        assert_eq!(signup.0, StatusCode::OK, "{}", signup.1);
        let token = signup.1["access_token"].as_str().unwrap().to_string();
        let enrolled = call(
            &app,
            "POST",
            "/auth/v1/factors",
            Some(r#"{"factor_type":"totp","friendly_name":"app","issuer":"megabase"}"#),
            Some(&token),
        )
        .await;
        assert_eq!(enrolled.0, StatusCode::OK, "{}", enrolled.1);
        let factor_id = enrolled.1["id"].as_str().unwrap();
        let secret = enrolled.1["totp"]["secret"].as_str().unwrap();
        let challenged = call(
            &app,
            "POST",
            &format!("/auth/v1/factors/{factor_id}/challenge"),
            Some("{}"),
            Some(&token),
        )
        .await;
        assert_eq!(challenged.0, StatusCode::OK, "{}", challenged.1);
        let challenge_id = challenged.1["id"].as_str().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let code = crate::totp::current_code(secret, now);
        let verified = call(
            &app,
            "POST",
            &format!("/auth/v1/factors/{factor_id}/verify"),
            Some(&format!(
                r#"{{"challenge_id":"{challenge_id}","code":"{code}"}}"#
            )),
            Some(&token),
        )
        .await;
        assert_eq!(verified.0, StatusCode::OK, "{}", verified.1);
        let claims = jwt
            .verify(verified.1["access_token"].as_str().unwrap())
            .unwrap();
        assert_eq!(claims.raw["aal"], "aal2");
        assert_eq!(claims.raw["amr"][0]["method"], "totp");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect(&url)
            .await
            .unwrap();
        let aal: String = sqlx::query_scalar(
            "SELECT s.aal::text FROM auth.sessions s
             JOIN auth.users u ON u.id = s.user_id
             WHERE u.email = $1",
        )
        .bind(&email)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(aal, "aal2");
        let stored: String =
            sqlx::query_scalar("SELECT secret FROM auth.mfa_factors WHERE id = $1::uuid")
                .bind(factor_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored, secret);
    }

    #[test]
    fn recovery_code_normalization_matches_go() {
        assert_eq!(normalize_recovery_code(" AbCd-EF\tgh\u{a0}"), "abcdefgh");
        assert_eq!(normalize_recovery_code("- \n"), "");
    }
}
