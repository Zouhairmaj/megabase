// Ported from supabase/auth internal/api/verify.go, internal/api/mail.go,
// internal/api/errors.go, internal/api/pkce.go, internal/utilities/request.go,
// and internal/crypto/crypto.go (MIT), pin v2.197.0.

//! `GET` and `POST /auth/v1/verify` for signup, invite, recovery, and email change.
//!
//! GET validation failures stay JSON. Later GET failures are 303 redirects
//! whose fragment matches GoTrue. POST failures stay JSON. A GET `pkce_`
//! token is the PKCE flow: success redirects with `?code=<auth_code>`
//! instead of a session, and errors also go in the query. Magic link, SMS,
//! phone change, and the `email` OTP type stay HTTP 501.

use std::time::SystemTime;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use megabase_core::MegabaseNotImplemented;
use serde::Deserialize;
use serde_json::json;

use crate::error::{bad_json, unexpected, validation, GoTrueError};
use crate::hooks::TokenHookCall;
use crate::jsonutil::{session_claims, session_json, unix_secs};
use crate::routes::{insert_header, query_param, read_body, request_aud, validate_email, JsonOk};
use crate::state::AuthState;
use crate::store::StoreError;
use crate::store::{email_otp_hash, VerifyKind, VerifyOutcome, VerifyRequest, SINGLE_CONFIRMATION};
use crate::token::{authentication_method, null_as_default, unhandled};
use crate::COMPONENT;

const UNIT_EMAIL: &str = "auth:verify-type:email";
const UNIT_SMS: &str = "auth:verify-type:sms";
const UNIT_PHONE_CHANGE: &str = "auth:verify-type:phone_change";

#[derive(Debug, Deserialize)]
struct VerifyBody {
    #[serde(default, rename = "type", deserialize_with = "null_as_default")]
    kind: String,
    #[serde(default, deserialize_with = "null_as_default")]
    token: String,
    #[serde(default, deserialize_with = "null_as_default")]
    token_hash: String,
    #[serde(default, deserialize_with = "null_as_default")]
    email: String,
    #[serde(default, deserialize_with = "null_as_default")]
    phone: String,
    #[serde(default, deserialize_with = "null_as_default")]
    redirect_to: String,
}

struct Params {
    kind: String,
    token: String,
    token_hash: String,
    email: String,
    phone: String,
    redirect_to: String,
}

// megabase:unit auth:route:GET /auth/v1/verify
pub(crate) async fn verify_get(
    State(state): State<AuthState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Response {
    let query = uri.query();
    let params = Params {
        kind: query_param(query, "type").unwrap_or_default(),
        token: query_param(query, "token").unwrap_or_default(),
        token_hash: String::new(),
        email: String::new(),
        phone: String::new(),
        redirect_to: String::new(),
    };
    if let Err(error) = validate_get(&params) {
        return error.into_response();
    }
    let redirect = match redirect_target(
        &state.config.site_url,
        &state.uri_allow_list,
        query_param(query, "redirect_to").as_deref(),
        headers
            .get(header::REFERER)
            .and_then(|value| value.to_str().ok()),
    ) {
        Ok(url) => url,
        Err(()) => {
            return MegabaseNotImplemented::new(COMPONENT, "GOTRUE_URI_ALLOW_LIST").into_response();
        }
    };
    // A `pkce_` token is the PKCE flow: the type must parse as an
    // `AuthenticationMethod` (a plain error there is GoTrue's JSON 500), and
    // a successful verify issues an auth code instead of a session.
    let pkce_method = if params.token.starts_with("pkce_") {
        match authentication_method(&params.kind) {
            Some(method) => Some(method),
            None => return unhandled(),
        }
    } else {
        None
    };
    let pkce = pkce_method.is_some();
    if let Some(unit) = out_of_scope(&params.kind) {
        return MegabaseNotImplemented::new(COMPONENT, unit).into_response();
    }
    let fail = |status: u16, error_code: &str, message: &str| {
        see_other(&error_redirect(
            &redirect, pkce, status, error_code, message,
        ))
    };
    let Some(kind) = known_kind(&params.kind) else {
        return fail(400, "validation_failed", "Invalid email verification type");
    };
    let request = VerifyRequest {
        kind,
        token_hash: params.token,
        hash_path: true,
        email: String::new(),
        aud: request_aud(&headers, &state.config),
        autoconfirm: state.config.mailer_autoconfirm,
        secure_email_change: state.config.secure_email_change,
        otp_exp_seconds: state.config.mailer_otp_exp_seconds,
        pkce_method,
    };
    // Signing happens after the one-time token is cleared. GoTrue rolls that
    // write back when the signer is missing; reject here so the token stays.
    // The PKCE flow signs nothing.
    if state.jwt.is_none() && !pkce {
        let error = unexpected("Server lacks JWT secret");
        return fail(error.status.as_u16(), error.error_code, &error.message);
    }
    let now = unix_secs(SystemTime::now());
    let hook = otp_hook(&state, &headers, now);
    match state.backend.verify(&request, hook.as_ref()).await {
        Ok(VerifyOutcome::Rejected {
            status,
            error_code,
            message,
        }) => fail(status, error_code, message),
        Ok(VerifyOutcome::SingleConfirmation) => {
            see_other(&message_redirect(&redirect, pkce, SINGLE_CONFIRMATION))
        }
        Err(StoreError::Hook(error)) => match error.redirect_parts() {
            Some((status, error_code, message)) => fail(status, error_code, &message),
            None => (*error).into_response(),
        },
        // `prepPKCERedirectURL`.
        Ok(VerifyOutcome::AuthCode(code)) => see_other(&with_query(&redirect, &[("code", &code)])),
        Ok(VerifyOutcome::Session(issued)) => match sign_session(&state, &issued, now) {
            Ok(signed) => see_other(&success_redirect(&redirect, &params.kind, &signed)),
            Err(error) => fail(error.status.as_u16(), error.error_code, &error.message),
        },
        Err(error) => {
            let failure = store_failure(true, &error);
            fail(
                failure.status.as_u16(),
                failure.error_code,
                &failure.message,
            )
        }
    }
}

// megabase:unit auth:route:POST /auth/v1/verify
pub(crate) async fn verify_post(
    State(state): State<AuthState>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let bytes = match read_body(body).await {
        Ok(bytes) => bytes,
        Err(error) => return error.into_response(),
    };
    let body: VerifyBody = match serde_json::from_slice(&bytes) {
        Ok(body) => body,
        Err(error) => return bad_json(error).into_response(),
    };
    let params = Params {
        kind: body.kind,
        token: body.token,
        token_hash: body.token_hash,
        email: body.email,
        phone: body.phone,
        redirect_to: body.redirect_to,
    };
    let prepared = match prepare_post(&params) {
        Ok(PostPrep::Phone) => {
            let unit = if params.kind == "phone_change" {
                UNIT_PHONE_CHANGE
            } else {
                UNIT_SMS
            };
            return MegabaseNotImplemented::new(COMPONENT, unit).into_response();
        }
        Ok(PostPrep::Ready(prepared)) => prepared,
        Err(error) => return error.into_response(),
    };
    if let Some(unit) = out_of_scope(&params.kind) {
        return MegabaseNotImplemented::new(COMPONENT, unit).into_response();
    }
    let kind = if let Some(kind) = known_kind(&params.kind) {
        kind
    } else if prepared.hash_path {
        return validation("Invalid email verification type").into_response();
    } else {
        VerifyKind::Other
    };
    let request = VerifyRequest {
        kind,
        token_hash: prepared.token_hash,
        hash_path: prepared.hash_path,
        email: prepared.email,
        aud: request_aud(&headers, &state.config),
        autoconfirm: state.config.mailer_autoconfirm,
        secure_email_change: state.config.secure_email_change,
        otp_exp_seconds: state.config.mailer_otp_exp_seconds,
        // `verifyPost` always issues a session, whatever the token prefix.
        pkce_method: None,
    };
    if state.jwt.is_none() {
        return unexpected("Server lacks JWT secret").into_response();
    }
    let now = unix_secs(SystemTime::now());
    let hook = otp_hook(&state, &headers, now);
    match state.backend.verify(&request, hook.as_ref()).await {
        Ok(VerifyOutcome::Rejected {
            status,
            error_code,
            message,
        }) => GoTrueError::new(
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
            error_code,
            message,
        )
        .into_response(),
        Ok(VerifyOutcome::SingleConfirmation) => JsonOk(json!({
            "msg": SINGLE_CONFIRMATION,
            "code": "200",
        }))
        .into_response(),
        Ok(VerifyOutcome::Session(issued)) => match sign_session(&state, &issued, now) {
            Ok(signed) => signed.response,
            Err(error) => error.into_response(),
        },
        // Not produced without `pkce_method`.
        Ok(VerifyOutcome::AuthCode(_)) => unhandled(),
        Err(StoreError::Hook(error)) => (*error).into_response(),
        Err(error) => store_failure(prepared.hash_path, &error).into_response(),
    }
}

/// Verify issues its session with `models.OTP`.
fn otp_hook<'a>(state: &'a AuthState, headers: &HeaderMap, now: i64) -> Option<TokenHookCall<'a>> {
    TokenHookCall::new(
        state.config.custom_access_token_hook.as_ref(),
        headers,
        "otp",
        &state.config.jwt_issuer,
        now,
        state.config.jwt_exp_seconds,
    )
}

struct Prepared {
    token_hash: String,
    hash_path: bool,
    email: String,
}

enum PostPrep {
    Ready(Prepared),
    Phone,
}

fn validate_get(params: &Params) -> Result<(), GoTrueError> {
    if params.kind.is_empty() {
        return Err(validation("Verify requires a verification type"));
    }
    if params.token.is_empty() {
        return Err(validation("Verify requires a token or a token hash"));
    }
    Ok(())
}

fn prepare_post(params: &Params) -> Result<PostPrep, GoTrueError> {
    if params.kind.is_empty() {
        return Err(validation("Verify requires a verification type"));
    }
    let has_token = !params.token.is_empty();
    let has_hash = !params.token_hash.is_empty();
    if has_token == has_hash {
        return Err(validation("Verify requires either a token or a token hash"));
    }
    if has_token {
        if !params.phone.is_empty() && params.email.is_empty() {
            return Ok(PostPrep::Phone);
        }
        if params.phone.is_empty() && !params.email.is_empty() {
            let email = validate_email(&params.email).map_err(|_| {
                GoTrueError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "validation_failed",
                    "Invalid email format",
                )
            })?;
            return Ok(PostPrep::Ready(Prepared {
                token_hash: email_otp_hash(&email, &params.token),
                hash_path: false,
                email,
            }));
        }
        return Err(validation(
            "Only an email address or phone number should be provided on verify",
        ));
    }
    if !params.email.is_empty() || !params.phone.is_empty() || !params.redirect_to.is_empty() {
        return Err(validation(
            "Only the token_hash and type should be provided",
        ));
    }
    Ok(PostPrep::Ready(Prepared {
        token_hash: params.token_hash.clone(),
        hash_path: true,
        email: String::new(),
    }))
}

fn known_kind(kind: &str) -> Option<VerifyKind> {
    match kind {
        // megabase:unit auth:verify-type:signup
        "signup" => Some(VerifyKind::Signup),
        // megabase:unit auth:verify-type:invite
        "invite" => Some(VerifyKind::Invite),
        // megabase:unit auth:verify-type:recovery
        "recovery" => Some(VerifyKind::Recovery),
        // `MagicLinkVerification` shares every step with recovery: the
        // `recovery_token`, its expiry, and `recoverVerify`. Only the `type`
        // in the success redirect differs.
        // megabase:unit auth:verify-type:magiclink
        "magiclink" => Some(VerifyKind::Recovery),
        // megabase:unit auth:verify-type:email_change
        "email_change" => Some(VerifyKind::EmailChange),
        _ => None,
    }
}

fn out_of_scope(kind: &str) -> Option<&'static str> {
    match kind {
        "email" => Some(UNIT_EMAIL),
        "sms" => Some(UNIT_SMS),
        "phone_change" => Some(UNIT_PHONE_CHANGE),
        _ => None,
    }
}

struct SignedSession {
    response: Response,
    access_token: String,
    expires_in: i64,
    expires_at: i64,
    refresh_token: String,
}

fn sign_session(
    state: &AuthState,
    issued: &crate::store::IssuedSession,
    now: i64,
) -> Result<SignedSession, GoTrueError> {
    let Some(jwt) = state.jwt.clone() else {
        return Err(unexpected("Server lacks JWT secret"));
    };
    let expires_in = state.config.jwt_exp_seconds;
    let claims = session_claims(issued, &state.config.jwt_issuer, now, expires_in);
    let access_token = jwt.sign(&claims).map_err(|error| {
        tracing::error!(%error, "signing verify access token failed");
        unexpected("error generating jwt token")
    })?;
    let body = session_json(issued, &access_token, expires_in, now);
    let mut response = JsonOk(body).into_response();
    insert_header(
        &mut response,
        "sb-auth-user-id",
        &issued.user.id.to_string(),
    );
    insert_header(
        &mut response,
        "sb-auth-session-id",
        &issued.session_id.to_string(),
    );
    let prefix: String = issued.refresh_token.chars().take(5).collect();
    insert_header(&mut response, "sb-auth-refresh-token-prefix", &prefix);
    Ok(SignedSession {
        response,
        access_token: access_token.as_str().to_string(),
        expires_in,
        expires_at: now + expires_in,
        refresh_token: issued.refresh_token.clone(),
    })
}

fn store_failure(hash_path: bool, error: &StoreError) -> GoTrueError {
    let message = match error {
        StoreError::Hash => "Error storing password",
        _ if hash_path => "Database error finding user from email link",
        _ => "Database error finding user",
    };
    unexpected(message)
}

fn see_other(url: &str) -> Response {
    let body = format!("<a href=\"{}\">See Other</a>.\n\n", html_escape(url));
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, url)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// `prepErrorRedirectURL`: the error goes in the fragment, and for the PKCE
/// flow also in the query.
fn error_redirect(
    base: &str,
    pkce: bool,
    status: u16,
    error_code: &str,
    description: &str,
) -> String {
    let oauth = match status {
        400 => "invalid_request",
        401 => "unauthorized_client",
        403 => "access_denied",
        500 => "server_error",
        503 => "temporarily_unavailable",
        _ => "",
    };
    let mut pairs = Vec::with_capacity(3);
    if !oauth.is_empty() {
        pairs.push(("error", oauth));
    }
    pairs.push(("error_code", error_code));
    pairs.push(("error_description", description));
    let base = if pkce {
        with_query(base, &pairs)
    } else {
        base.to_string()
    };
    pairs.push(("sb", ""));
    set_fragment(&base, &form_encode(&pairs))
}

/// `prepRedirectURL`: the message goes in the fragment, and for the PKCE
/// flow also in the query.
fn message_redirect(base: &str, pkce: bool, message: &str) -> String {
    let base = if pkce {
        with_query(base, &[("message", message)])
    } else {
        base.to_string()
    };
    set_fragment(&base, &form_encode(&[("message", message), ("sb", "")]))
}

/// Go `q := u.Query(); q.Set(...); u.RawQuery = q.Encode()`: the existing
/// query is parsed (pairs Go rejects are dropped), `pairs` replace their
/// keys, and the result is re-encoded with sorted keys. The fragment stays.
fn with_query(url: &str, pairs: &[(&str, &str)]) -> String {
    let (head, fragment) = match url.split_once('#') {
        Some((head, fragment)) => (head, Some(fragment)),
        None => (url, None),
    };
    let (path, query) = head.split_once('?').unwrap_or((head, ""));
    let mut values: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for pair in query.split('&') {
        if pair.is_empty() || pair.contains(';') {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if let (Some(key), Some(value)) = (query_unescape(key), query_unescape(value)) {
            values.entry(key).or_default().push(value);
        }
    }
    for (key, value) in pairs {
        values.insert((*key).to_string(), vec![(*value).to_string()]);
    }
    let encoded = values
        .iter()
        .flat_map(|(key, list)| {
            list.iter()
                .map(move |value| format!("{}={}", query_escape(key), query_escape(value)))
        })
        .collect::<Vec<_>>()
        .join("&");
    let mut out = path.to_string();
    if !encoded.is_empty() {
        out.push('?');
        out.push_str(&encoded);
    }
    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

/// Go `url.QueryUnescape`: `+` is a space and a bad `%` escape fails. A
/// value that decodes to invalid UTF-8 (which Go would keep as raw bytes)
/// is dropped as well.
fn query_unescape(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' => {
                let hex = bytes.get(index + 1..index + 3)?;
                if !hex.iter().all(u8::is_ascii_hexdigit) {
                    return None;
                }
                let text = std::str::from_utf8(hex).ok()?;
                out.push(u8::from_str_radix(text, 16).ok()?);
                index += 3;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn success_redirect(base: &str, kind: &str, signed: &SignedSession) -> String {
    let expires_in = signed.expires_in.to_string();
    let expires_at = signed.expires_at.to_string();
    let fragment = form_encode(&[
        ("access_token", &signed.access_token),
        ("expires_at", &expires_at),
        ("expires_in", &expires_in),
        ("refresh_token", &signed.refresh_token),
        ("sb", ""),
        ("token_type", "bearer"),
        ("type", kind),
    ]);
    set_fragment(base, &fragment)
}

fn set_fragment(url: &str, fragment: &str) -> String {
    let base = url.split('#').next().unwrap_or(url);
    format!("{base}#{fragment}")
}

fn form_encode(pairs: &[(&str, &str)]) -> String {
    let mut pairs: Vec<(&str, &str)> = pairs.to_vec();
    pairs.sort_by(|left, right| left.0.cmp(right.0));
    pairs
        .into_iter()
        .map(|(key, value)| format!("{}={}", query_escape(key), query_escape(value)))
        .collect::<Vec<_>>()
        .join("&")
}

pub(crate) fn query_escape(input: &str) -> String {
    const HEX: &[u8] = b"0123456789ABCDEF";
    let mut out = String::new();
    for &byte in input.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else if byte == b' ' {
            out.push('+');
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

pub(crate) fn html_escape(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '\'' => out.push_str("&#39;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&#34;"),
            ch => out.push(ch),
        }
    }
    out
}

fn redirect_target(
    site_url: &str,
    allow: &[String],
    requested: Option<&str>,
    referer: Option<&str>,
) -> Result<String, ()> {
    if let Some(url) = take_allowed(site_url, allow, requested)? {
        return Ok(url);
    }
    if let Some(url) = take_allowed(site_url, allow, referer)? {
        return Ok(url);
    }
    Ok(site_url.to_string())
}

fn take_allowed(
    site: &str,
    allow: &[String],
    candidate: Option<&str>,
) -> Result<Option<String>, ()> {
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    match crate::admin_batch2::redirect_ok(site, candidate, allow) {
        Ok(true) => Ok(Some(candidate.to_string())),
        Ok(false) => Ok(None),
        // Same 501 as admin generate-link: the pattern is configured, and
        // guessing that it does not match would drop a redirect GoTrue accepts.
        Err(crate::admin_batch2::RedirectAllowListUnsupported) => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;
    use crate::state::AuthState;
    use crate::store::{Backend, PlantFlags};
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::Router;
    use megabase_core::Hs256;
    use serde_json::Value;
    use tower::ServiceExt;

    const SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";
    const LINK_EXPIRED: &str = "Email link is invalid or has expired";

    fn app(autoconfirm: bool, backend: Backend) -> Router {
        let mut config = AuthConfig::reference_defaults();
        config.mailer_autoconfirm = autoconfirm;
        let jwt = Hs256::new(SECRET.as_bytes()).unwrap();
        crate::routes::router(AuthState::new(config, Some(jwt), backend))
    }

    fn app_without_jwt(backend: Backend) -> Router {
        crate::routes::router(AuthState::new(
            AuthConfig::reference_defaults(),
            None,
            backend,
        ))
    }

    async fn call(
        app: Router,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> (StatusCode, Value, HeaderMap, String) {
        let mut builder = Request::builder().method(method).uri(path);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let request = builder
            .body(Body::from(body.unwrap_or("").to_string()))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json, headers, text)
    }

    fn expired_location() -> String {
        "http://localhost:3000#error=access_denied&error_code=otp_expired&error_description=Email+link+is+invalid+or+has+expired&sb=".into()
    }

    #[test]
    fn redirect_fragment_matches_gotrue() {
        let url = error_redirect(
            "http://localhost:3000",
            false,
            403,
            "otp_expired",
            LINK_EXPIRED,
        );
        assert_eq!(url, expired_location());
        let html = format!("<a href=\"{}\">See Other</a>.\n\n", html_escape(&url));
        assert!(html.contains("&amp;"));
        assert!(html.ends_with(".\n\n"));
    }

    #[test]
    fn pkce_redirects_rewrite_the_query_like_go() {
        assert_eq!(
            with_query(
                "http://localhost:3000/cb?z=1&a=x+y&bad=%zz#keep",
                &[("code", "c-1")]
            ),
            "http://localhost:3000/cb?a=x+y&code=c-1&z=1#keep"
        );
        assert_eq!(
            with_query("http://localhost:3000?code=old", &[("code", "new")]),
            "http://localhost:3000?code=new"
        );
        assert_eq!(
            error_redirect("http://localhost:3000", true, 403, "otp_expired", LINK_EXPIRED),
            "http://localhost:3000?error=access_denied&error_code=otp_expired&error_description=Email+link+is+invalid+or+has+expired#error=access_denied&error_code=otp_expired&error_description=Email+link+is+invalid+or+has+expired&sb="
        );
        assert_eq!(
            message_redirect("http://localhost:3000", true, "hi there"),
            "http://localhost:3000?message=hi+there#message=hi+there&sb="
        );
    }

    #[tokio::test]
    async fn pkce_verify_without_a_flow_state_rolls_back() {
        let backend = Backend::memory();
        backend
            .plant_verification_for_test("pkce@example.com", "pkce_abc", PlantFlags::default())
            .await;
        let router = app(false, backend);
        let expected = "http://localhost:3000?error=invalid_request&error_code=flow_state_not_found&error_description=No+associated+flow+state+found.+422%3A+No+valid+flow+state+found+for+user.#error=invalid_request&error_code=flow_state_not_found&error_description=No+associated+flow+state+found.+422%3A+No+valid+flow+state+found+for+user.&sb=";
        // Twice: the rejected auth code leaves the token in place.
        for _ in 0..2 {
            let (status, _, headers, _) = call(
                router.clone(),
                "GET",
                "/auth/v1/verify?type=signup&token=pkce_abc",
                None,
            )
            .await;
            assert_eq!(status, StatusCode::SEE_OTHER);
            assert_eq!(headers[header::LOCATION], expected);
        }
    }

    #[test]
    fn success_redirect_replaces_an_existing_fragment() {
        let signed = SignedSession {
            response: Response::new(Body::empty()),
            access_token: "tok".into(),
            expires_in: 3600,
            expires_at: 1,
            refresh_token: "ref".into(),
        };
        let url = success_redirect("http://localhost:3000/welcome#section", "signup", &signed);
        assert_eq!(url.matches('#').count(), 1);
        assert!(url.starts_with("http://localhost:3000/welcome#"));
        assert!(url.contains("access_token=tok"));
        assert!(!url.contains("section"));
    }

    #[test]
    fn userinfo_redirect_falls_back_and_allow_list_is_honored() {
        let site = "http://localhost:3000";
        assert_eq!(
            redirect_target(site, &[], Some("http://127.0.0.1:1@evil.com/"), None).unwrap(),
            site
        );
        let allow = vec!["https://app.example.com/*".to_string()];
        assert_eq!(
            redirect_target(site, &allow, Some("https://app.example.com/welcome"), None).unwrap(),
            "https://app.example.com/welcome"
        );
        assert!(redirect_target(
            site,
            &["https://app.example/[id]".to_string()],
            Some("https://app.example/cb"),
            None,
        )
        .is_err());
    }

    #[tokio::test]
    async fn validation_errors_match_gotrue() {
        let router = app(true, Backend::memory());
        let (status, body, _, _) = call(router.clone(), "GET", "/auth/v1/verify", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "validation_failed");
        assert_eq!(body["msg"], "Verify requires a verification type");

        let (status, body, _, _) =
            call(router.clone(), "GET", "/auth/v1/verify?type=signup", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["msg"], "Verify requires a token or a token hash");

        let (status, body, _, _) =
            call(router.clone(), "POST", "/auth/v1/verify", Some("{}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["msg"], "Verify requires a verification type");

        // Go reads `null` as the zero value, so null fields are absent fields.
        let nulls = r#"{"type":null,"token":null,"token_hash":null,"email":null,"phone":null,"redirect_to":null}"#;
        let (status, body, _, _) =
            call(router.clone(), "POST", "/auth/v1/verify", Some(nulls)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error_code"], "validation_failed");
        assert_eq!(body["msg"], "Verify requires a verification type");

        let (status, body, _, _) = call(
            router.clone(),
            "POST",
            "/auth/v1/verify",
            Some(r#"{"type":"signup"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["msg"],
            "Verify requires either a token or a token hash"
        );

        let (status, body, _, _) = call(
            router.clone(),
            "POST",
            "/auth/v1/verify",
            Some(r#"{"type":"signup","token":"1","token_hash":"2"}"#),
        )
        .await;
        assert_eq!(
            body["msg"],
            "Verify requires either a token or a token hash"
        );
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, body, _, _) = call(
            router.clone(),
            "POST",
            "/auth/v1/verify",
            Some(r#"{"type":"signup","token":"123456"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["msg"],
            "Only an email address or phone number should be provided on verify"
        );

        let (status, body, _, _) = call(
            router.clone(),
            "POST",
            "/auth/v1/verify",
            Some(r#"{"type":"signup","token_hash":"abc","email":"a@b.com"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["msg"],
            "Only the token_hash and type should be provided"
        );

        let (status, body, _, _) = call(
            router,
            "POST",
            "/auth/v1/verify",
            Some(r#"{"type":"signup","token":"123456","email":"not-an-email"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error_code"], "validation_failed");
        assert_eq!(body["msg"], "Invalid email format");
    }

    #[tokio::test]
    async fn missing_hash_is_forbidden_or_redirect() {
        let router = app(true, Backend::memory());
        for kind in ["signup", "invite", "recovery", "email_change"] {
            let (status, _, headers, text) = call(
                router.clone(),
                "GET",
                &format!("/auth/v1/verify?type={kind}&token=deadbeef"),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::SEE_OTHER, "{kind}");
            assert_eq!(
                headers.get(header::LOCATION).unwrap().to_str().unwrap(),
                expired_location()
            );
            assert!(text.contains("See Other"));
            assert!(text.contains("&amp;"));

            let (status, body, _, _) = call(
                router.clone(),
                "POST",
                "/auth/v1/verify",
                Some(&format!(r#"{{"type":"{kind}","token_hash":"deadbeef"}}"#)),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{kind} {body}");
            assert_eq!(body["error_code"], "otp_expired");
            assert_eq!(body["msg"], LINK_EXPIRED);
        }

        let (status, body, _, _) = call(
            router,
            "POST",
            "/auth/v1/verify",
            Some(r#"{"type":"nope","token_hash":"x"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["msg"], "Invalid email verification type");
    }

    #[tokio::test]
    async fn one_token_redeems_once_when_two_requests_race() {
        // Invite hashing yields after the first lookup and before the recheck,
        // so both requests still see the token. PostgreSQL takes the same
        // order: locate, `SELECT … FOR UPDATE` on `auth.users`, locate again.
        let backend = Backend::memory();
        backend
            .plant_verification_for_test(
                "race@example.com",
                "racehash",
                PlantFlags {
                    kind: VerifyKind::Invite,
                    invited: true,
                    ..PlantFlags::default()
                },
            )
            .await;
        let router = app(true, backend);
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let body = r#"{"type":"invite","token_hash":"racehash"}"#;
        let once = |router: Router, barrier: std::sync::Arc<tokio::sync::Barrier>| async move {
            barrier.wait().await;
            call(router, "POST", "/auth/v1/verify", Some(body)).await
        };
        let (left, right) = tokio::join!(
            once(router.clone(), std::sync::Arc::clone(&barrier)),
            once(router, barrier),
        );
        let outcomes = [left.0, right.0];
        let wins = outcomes
            .iter()
            .filter(|status| **status == StatusCode::OK)
            .count();
        assert_eq!(wins, 1, "left {} right {}", left.0, right.0);
        assert!(
            outcomes.contains(&StatusCode::FORBIDDEN),
            "left {} {:?} right {} {:?}",
            left.0,
            left.1,
            right.0,
            right.1
        );
    }

    #[tokio::test]
    async fn signup_invite_recovery_and_email_change_issue_otp_sessions() {
        let backend = Backend::memory();
        let email = "person@example.com";
        let hash = email_otp_hash(email, "123456");
        backend
            .plant_verification_for_test(email, &hash, PlantFlags::default())
            .await;
        let router = app(true, backend.clone());
        let (status, body, _, _) = call(
            router,
            "POST",
            "/auth/v1/verify",
            Some(&format!(
                r#"{{"type":"signup","token":"123456","email":"{email}"}}"#
            )),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["user"]["email"], email);
        assert!(body["user"]["email_confirmed_at"].is_string());
        let access = body["access_token"].as_str().unwrap();
        let claims = Hs256::new(SECRET.as_bytes())
            .unwrap()
            .verify(access)
            .unwrap();
        assert_eq!(claims.raw["amr"][0]["method"], "otp");

        let backend = Backend::memory();
        backend
            .plant_verification_for_test(
                "invited@example.com",
                "invitehash",
                PlantFlags {
                    kind: VerifyKind::Invite,
                    invited: true,
                    ..PlantFlags::default()
                },
            )
            .await;
        let router = app(true, backend.clone());
        let (status, body, _, _) = call(
            router,
            "POST",
            "/auth/v1/verify",
            Some(r#"{"type":"invite","token_hash":"invitehash"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let stored = backend
            .password_hash_for_test("invited@example.com")
            .await
            .unwrap();
        assert!(!stored.is_empty());

        let backend = Backend::memory();
        backend
            .plant_verification_for_test(
                "back@example.com",
                "recoverhash",
                PlantFlags {
                    kind: VerifyKind::Recovery,
                    confirmed: true,
                    password_hash: "hash".into(),
                    ..PlantFlags::default()
                },
            )
            .await;
        let router = app(true, backend);
        let (status, body, _, _) = call(
            router,
            "POST",
            "/auth/v1/verify",
            Some(r#"{"type":"recovery","token_hash":"recoverhash"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["user"]["email"], "back@example.com");
        assert!(body["user"]["email_confirmed_at"].is_string());

        let backend = Backend::memory();
        backend
            .plant_verification_for_test(
                "old@example.com",
                "changehash",
                PlantFlags {
                    kind: VerifyKind::EmailChange,
                    confirmed: true,
                    password_hash: "hash".into(),
                    email_change: "new@example.com".into(),
                    ..PlantFlags::default()
                },
            )
            .await;
        let router = app(true, backend);
        let (status, body, _, _) = call(
            router,
            "POST",
            "/auth/v1/verify",
            Some(r#"{"type":"email_change","token_hash":"changehash"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["user"]["email"], "new@example.com");
        assert_eq!(
            body["user"]["identities"][0]["identity_data"]["email"],
            "new@example.com"
        );
        assert_eq!(
            body["user"]["identities"][0]["identity_data"]["email_verified"],
            true
        );
    }

    #[tokio::test]
    async fn secure_email_change_without_autoconfirm_is_partial() {
        let backend = Backend::memory();
        backend
            .plant_verification_for_test(
                "old@example.com",
                "onehash",
                PlantFlags {
                    kind: VerifyKind::EmailChange,
                    confirmed: true,
                    password_hash: "hash".into(),
                    email_change: "new@example.com".into(),
                    email_change_current: true,
                    ..PlantFlags::default()
                },
            )
            .await;
        let router = app(false, backend);
        let (status, body, _, _) = call(
            router,
            "POST",
            "/auth/v1/verify",
            Some(r#"{"type":"email_change","token_hash":"onehash"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["msg"], SINGLE_CONFIRMATION);
        assert_eq!(body["code"], "200");
        assert!(body.get("access_token").is_none());
    }

    #[tokio::test]
    async fn banned_and_expired_tokens_are_forbidden() {
        let backend = Backend::memory();
        backend
            .plant_verification_for_test(
                "banned@example.com",
                "bannedhash",
                PlantFlags {
                    banned: true,
                    ..PlantFlags::default()
                },
            )
            .await;
        let router = app(true, backend);
        let (status, body, _, _) = call(
            router,
            "POST",
            "/auth/v1/verify",
            Some(r#"{"type":"signup","token_hash":"bannedhash"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error_code"], "user_banned");
        assert_eq!(body["msg"], "User is banned");

        let backend = Backend::memory();
        backend
            .plant_verification_for_test(
                "old@example.com",
                "oldhash",
                PlantFlags {
                    sent_at: SystemTime::now() - std::time::Duration::from_secs(90_000),
                    ..PlantFlags::default()
                },
            )
            .await;
        let router = app(true, backend);
        let (status, body, _, _) = call(
            router,
            "POST",
            "/auth/v1/verify",
            Some(r#"{"type":"signup","token_hash":"oldhash"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["msg"], LINK_EXPIRED);
    }

    #[tokio::test]
    async fn out_of_scope_types_and_missing_database() {
        let router = app(true, Backend::memory());
        let (status, body, _, _) = call(
            router.clone(),
            "POST",
            "/auth/v1/verify",
            Some(r#"{"type":"email","token_hash":"x"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], UNIT_EMAIL);

        // `magiclink` is served through the recovery path.
        let (status, body, _, _) = call(
            router.clone(),
            "POST",
            "/auth/v1/verify",
            Some(r#"{"type":"magiclink","token_hash":"x"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["msg"], LINK_EXPIRED);

        let (status, body, _, _) = call(
            router.clone(),
            "GET",
            "/auth/v1/verify?type=sms&token=pkce_abc",
            None,
        )
        .await;
        // `ParseAuthenticationMethod("sms")` fails before the 501 checks.
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error_code"], "unexpected_failure");

        let router = app(true, Backend::none());
        let (status, body, _, _) = call(
            router,
            "POST",
            "/auth/v1/verify",
            Some(r#"{"type":"signup","token_hash":"deadbeef"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["msg"], "Database error finding user from email link");
    }

    #[tokio::test]
    async fn missing_jwt_does_not_consume_the_token() {
        let backend = Backend::memory();
        backend
            .plant_verification_for_test("keep@example.com", "keephash", PlantFlags::default())
            .await;
        let (status, _, headers, _) = call(
            app_without_jwt(backend.clone()),
            "GET",
            "/auth/v1/verify?type=signup&token=keephash",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let location = headers.get(header::LOCATION).unwrap().to_str().unwrap();
        assert!(location.contains("error=server_error"));
        assert!(location.contains("error_code=unexpected_failure"));
        assert!(location.contains("Server+lacks+JWT+secret"));

        let (status, body, _, _) = call(
            app_without_jwt(backend.clone()),
            "POST",
            "/auth/v1/verify",
            Some(r#"{"type":"signup","token_hash":"keephash"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error_code"], "unexpected_failure");
        assert_eq!(body["msg"], "Server lacks JWT secret");

        let (status, body, _, _) = call(
            app(true, backend),
            "POST",
            "/auth/v1/verify",
            Some(r#"{"type":"signup","token_hash":"keephash"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["user"]["email"], "keep@example.com");
    }
}
