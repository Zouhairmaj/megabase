// Ported from supabase/auth internal/hooks/hookspgfunc/hookspgfunc.go,
// internal/hooks/hookserrors/hookserrors.go, internal/hooks/v0hooks/v0hooks.go,
// internal/hooks/v0hooks/manager.go, internal/conf/configuration.go
// (`ExtensibilityPointConfiguration`), internal/tokens/service.go
// (`GenerateAccessToken`, `validateTokenClaims`, `MinimumViableTokenSchema`),
// internal/utilities/postgres.go and internal/utilities/request.go (MIT),
// pin v2.197.0.

//! Custom Access Token Hook (`GOTRUE_HOOK_CUSTOM_ACCESS_TOKEN_*`).
//!
//! When the hook is enabled, every access token is built from the claims a
//! Postgres function returns. The store calls [`run_pg`] inside the
//! transaction that writes the session, so a hook failure rolls the session
//! back (GoTrue passes the session `tx` to `InvokeHook`). Only
//! `pg-functions://` URIs are served; an HTTP(S) hook is HTTP 501.

use std::fmt;
use std::net::IpAddr;
use std::time::SystemTime;

use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use megabase_core::MegabaseNotImplemented;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::error::ConfigError;
use crate::jsonutil::access_claims;
use crate::store::IssuedSession;
use crate::COMPONENT;

/// Unit served by the Postgres dispatcher. HTTP(S) hooks return 501 with it.
pub(crate) const UNIT: &str = "auth:hook:custom_access_token";

/// `hookspgfunc.defaultTimeout`, applied as `statement_timeout` in milliseconds.
const STATEMENT_TIMEOUT_MS: u32 = 2000;

/// Event name in the hook input metadata (`v0hooks.CustomizeAccessToken`).
const HOOK_EVENT_NAME: &str = "customize-access-token";

/// Role GoTrue connects as on the reference stack (`GOTRUE_DB_DATABASE_URL`).
const AUTH_ADMIN_ROLE: &str = "supabase_auth_admin";

const ENV_ENABLED: &str = "GOTRUE_HOOK_CUSTOM_ACCESS_TOKEN_ENABLED";
const ENV_URI: &str = "GOTRUE_HOOK_CUSTOM_ACCESS_TOKEN_URI";

/// Enabled custom access token hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessTokenHook {
    uri: String,
    target: HookTarget,
}

/// Dispatcher chosen from the URI prefix (`Manager.dispatch`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum HookTarget {
    /// `pg-functions:` with the quoted `"schema"."function"` name.
    PgFunction(PgFunctionName),
    /// `http:` or `https:`. Not served yet.
    Http,
    /// Any other prefix. GoTrue fails token generation with a plain error.
    Unsupported,
}

/// Quoted `"schema"."function"` from a validated `pg-functions://` path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgFunctionName(String);

impl PgFunctionName {
    /// `PopulateExtensibilityPoint`: `fmt.Sprintf("%q.%q", parts[1], parts[2])`.
    ///
    /// Returns `None` when either part is not a Postgres name
    /// (`postgresNamesRegexp`), so the result is always safe to splice into SQL.
    fn new(schema: &str, function: &str) -> Option<Self> {
        (postgres_name(schema) && postgres_name(function))
            .then(|| Self(format!("\"{schema}\".\"{function}\"")))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl AccessTokenHook {
    /// `GOTRUE_HOOK_CUSTOM_ACCESS_TOKEN_ENABLED` and `_URI`.
    ///
    /// GoTrue validates a non-empty URI whether or not the hook is enabled,
    /// and refuses to start when it is invalid.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Invalid`] for a bad boolean or an invalid URI.
    pub(crate) fn from_lookup(
        lookup: &impl Fn(&str) -> Option<String>,
    ) -> Result<Option<Self>, ConfigError> {
        let enabled = crate::config::optional_bool(lookup, ENV_ENABLED)?.unwrap_or(false);
        let uri = lookup(ENV_URI).unwrap_or_default();
        if !uri.is_empty() && !valid_uri(&uri) {
            return Err(ConfigError::Invalid {
                key: ENV_URI.into(),
                value: uri,
            });
        }
        if !enabled {
            return Ok(None);
        }
        let target = if uri.starts_with("pg-functions:") {
            // `valid_uri` already checked the two path parts.
            match pg_function(&uri) {
                Some(name) => HookTarget::PgFunction(name),
                None => HookTarget::Unsupported,
            }
        } else if uri.starts_with("http:") || uri.starts_with("https:") {
            HookTarget::Http
        } else {
            HookTarget::Unsupported
        };
        Ok(Some(Self { uri, target }))
    }

    /// Hook for tests: `pg-functions://postgres/<schema>/<function>`.
    #[cfg(test)]
    pub(crate) fn pg_function_for_test(schema: &str, function: &str) -> Self {
        let uri = format!("pg-functions://postgres/{schema}/{function}");
        let name = PgFunctionName::new(schema, function).expect("valid test hook name");
        Self {
            uri,
            target: HookTarget::PgFunction(name),
        }
    }
}

/// `ValidateExtensibilityPoint` for the URI schemes GoTrue accepts.
fn valid_uri(uri: &str) -> bool {
    let Some((scheme, _)) = split_scheme(uri) else {
        return false;
    };
    match scheme.to_ascii_lowercase().as_str() {
        "pg-functions" => {
            let parts: Vec<&str> = url_path(uri).split('/').collect();
            parts.len() >= 3 && postgres_name(parts[1]) && postgres_name(parts[2])
        }
        "http" => matches!(
            url_host(uri).as_str(),
            "localhost" | "127.0.0.1" | "::1" | "host.docker.internal"
        ),
        "https" => true,
        _ => false,
    }
}

fn pg_function(uri: &str) -> Option<PgFunctionName> {
    let parts: Vec<&str> = url_path(uri).split('/').collect();
    if parts.len() < 3 {
        return None;
    }
    PgFunctionName::new(parts[1], parts[2])
}

/// RFC 3986 scheme before the first `:`.
fn split_scheme(uri: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = uri.split_once(':')?;
    let mut chars = scheme.chars();
    let first = chars.next()?;
    let valid = first.is_ascii_alphabetic()
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'));
    valid.then_some((scheme, rest))
}

/// Go `url.URL.Path`: the part after the authority, before `?` and `#`.
/// An opaque URI (`scheme:path` without a leading `/`) has an empty path.
fn url_path(uri: &str) -> &str {
    let Some((_, rest)) = split_scheme(uri) else {
        return "";
    };
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let rest = match rest.strip_prefix("//") {
        Some(authority_and_path) => match authority_and_path.find('/') {
            Some(index) => &authority_and_path[index..],
            None => "",
        },
        None => rest,
    };
    if rest.starts_with('/') {
        rest
    } else {
        ""
    }
}

/// Go `url.URL.Hostname()`.
fn url_host(uri: &str) -> String {
    let Some((_, rest)) = split_scheme(uri) else {
        return String::new();
    };
    let Some(authority) = rest.strip_prefix("//") else {
        return String::new();
    };
    let authority = authority.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if let Some(bracketed) = host.strip_prefix('[') {
        return bracketed.split(']').next().unwrap_or("").to_string();
    }
    host.split(':').next().unwrap_or("").to_string()
}

/// `postgresNamesRegexp`: `^[a-zA-Z_][a-zA-Z0-9_]{0,62}$`.
fn postgres_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && name.len() <= 63
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// One token issuance that must go through the hook.
pub struct TokenHookCall<'a> {
    hook: &'a AccessTokenHook,
    /// `models.AuthenticationMethod.String()` for this grant.
    authentication_method: &'static str,
    ip_address: Option<String>,
    issuer: &'a str,
    now: i64,
    expires_in: i64,
}

impl<'a> TokenHookCall<'a> {
    /// The call for one request, or `None` when the hook is disabled.
    pub(crate) fn new(
        hook: Option<&'a AccessTokenHook>,
        headers: &HeaderMap,
        authentication_method: &'static str,
        issuer: &'a str,
        now: i64,
        expires_in: i64,
    ) -> Option<Self> {
        hook.map(|hook| Self {
            hook,
            authentication_method,
            ip_address: forwarded_ip(headers),
            issuer,
            now,
            expires_in,
        })
    }

    /// `CustomAccessTokenInput` for the session the store just wrote.
    pub(crate) fn input(&self, issued: &IssuedSession) -> Value {
        let claims = access_claims(
            &issued.user,
            issued.session_id,
            issued.amr_at,
            &issued.amr_method,
            self.issuer,
            self.now,
            self.expires_in,
        );
        let mut metadata = Map::new();
        metadata.insert("uuid".into(), json!(Uuid::new_v4().to_string()));
        metadata.insert("time".into(), json!(rfc3339_nano(SystemTime::now())));
        metadata.insert("name".into(), json!(HOOK_EVENT_NAME));
        if let Some(ip) = &self.ip_address {
            metadata.insert("ip_address".into(), json!(ip));
        }
        json!({
            "metadata": Value::Object(metadata),
            "user_id": issued.user.id.to_string(),
            "claims": claims,
            "authentication_method": self.authentication_method,
        })
    }
}

/// Go `time.Time` JSON (`RFC3339Nano`, trailing zeros trimmed).
fn rfc3339_nano(time: SystemTime) -> String {
    let time = DateTime::<Utc>::from(time);
    let base = time.format("%Y-%m-%dT%H:%M:%S").to_string();
    let nanos = time.timestamp_subsec_nanos();
    if nanos == 0 {
        return format!("{base}Z");
    }
    let fraction = format!("{nanos:09}");
    format!("{base}.{}Z", fraction.trim_end_matches('0'))
}

/// `getIPAddressWithXFF`: the first parseable `X-Forwarded-For` entry.
///
/// Megabase handlers do not see the socket address, so GoTrue's
/// `RemoteAddr` fallback leaves `ip_address` out (`omitempty`).
fn forwarded_ip(headers: &HeaderMap) -> Option<String> {
    let header = headers.get("x-forwarded-for")?.to_str().ok()?;
    header
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .find_map(|entry| entry.parse::<IpAddr>().ok())
        .map(|ip| ip.to_string())
}

/// Why the hook did not produce claims. Each variant is the GoTrue response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookError {
    /// `*apierrors.HTTPError`. `error_code` is `None` when GoTrue builds the
    /// error as a literal (hook `error` object, schema failure, missing
    /// claims): the body then falls back to `unexpected_failure` / `unknown`
    /// and `x-sb-error-code` is not sent.
    Http {
        status: StatusCode,
        error_code: Option<&'static str>,
        message: String,
    },
    /// A publicly visible Postgres error raised by the hook function
    /// (`utilities.NewPostgresError`). The redirect form of the error is the
    /// wrapping `Error running hook URI` failure.
    Postgres {
        status: StatusCode,
        code: String,
        message: String,
        hint: String,
        detail: String,
        uri: String,
    },
    /// HTTP(S) hooks are not served yet.
    NotImplemented,
}

impl fmt::Display for HookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http {
                status, message, ..
            } => write!(f, "{}: {message}", status.as_u16()),
            Self::Postgres { code, message, .. } => write!(f, "{code}: {message}"),
            Self::NotImplemented => write!(f, "{UNIT} over HTTP is not implemented"),
        }
    }
}

impl std::error::Error for HookError {}

impl HookError {
    fn coded(message: impl Into<String>) -> Self {
        Self::Http {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error_code: Some("unexpected_failure"),
            message: message.into(),
        }
    }

    fn uncoded(status: StatusCode, message: impl Into<String>) -> Self {
        Self::Http {
            status,
            error_code: None,
            message: message.into(),
        }
    }

    /// Status, `error_code`, and message for the `GET /verify` error redirect
    /// (`prepErrorRedirectURL` reads the raw `ErrorCode`, so it may be empty).
    ///
    /// `None` for [`HookError::NotImplemented`], which is answered with 501.
    pub(crate) fn redirect_parts(&self) -> Option<(u16, &'static str, String)> {
        match self {
            Self::Http {
                status,
                error_code,
                message,
            } => Some((status.as_u16(), error_code.unwrap_or(""), message.clone())),
            Self::Postgres { uri, .. } => Some((
                500,
                "unexpected_failure",
                format!("Error running hook URI: {uri}"),
            )),
            Self::NotImplemented => None,
        }
    }
}

impl IntoResponse for HookError {
    fn into_response(self) -> Response {
        match self {
            Self::Http {
                status,
                error_code,
                message,
            } => {
                let code = error_code.unwrap_or(if status == StatusCode::INTERNAL_SERVER_ERROR {
                    "unexpected_failure"
                } else {
                    "unknown"
                });
                let body = json!({
                    "code": status.as_u16(),
                    "error_code": code,
                    "msg": message,
                });
                let mut response = (status, Json(body)).into_response();
                if let Some(code) = error_code {
                    set_error_code(&mut response, code);
                }
                response
            }
            Self::Postgres {
                status,
                code,
                message,
                hint,
                detail,
                ..
            } => {
                let mut body = Map::new();
                body.insert("code".into(), json!(code));
                body.insert("message".into(), json!(message));
                if !hint.is_empty() {
                    body.insert("hint".into(), json!(hint));
                }
                if !detail.is_empty() {
                    body.insert("detail".into(), json!(detail));
                }
                let mut response = (status, Json(Value::Object(body))).into_response();
                set_error_code(&mut response, "unexpected_failure");
                response
            }
            Self::NotImplemented => MegabaseNotImplemented::new(COMPONENT, UNIT).into_response(),
        }
    }
}

fn set_error_code(response: &mut Response, code: &'static str) {
    response.headers_mut().insert(
        HeaderName::from_static("x-sb-error-code"),
        HeaderValue::from_static(code),
    );
}

/// What the hook function returned, or the error it raised.
pub(crate) enum PgOutcome {
    /// `select fn(event)::text`. `None` is SQL `NULL`.
    Returned(Option<String>),
    Failed(sqlx::Error),
}

/// Run the hook inside the caller's session transaction (`runPostgresHook`).
///
/// GoTrue's connection is `supabase_auth_admin`. Megabase may connect as a
/// superuser, so the call switches to that role when it exists and the
/// session may assume it.
///
/// # Errors
///
/// A [`HookError`] with GoTrue's status and message. Every database error,
/// including the ones around the call, is GoTrue's hook failure.
// megabase:unit auth:hook:custom_access_token
pub(crate) async fn run_pg(
    conn: &mut sqlx::PgConnection,
    call: &TokenHookCall<'_>,
    issued: &IssuedSession,
) -> Result<Value, HookError> {
    let function = pg_target(call)?;
    let input = call.input(issued);
    let outcome = invoke_pg(conn, function, &input).await;
    finish(outcome, &call.hook.uri)
}

/// In-memory stand-in for the hook function: event in, `jsonb` text out
/// (`None` is SQL `NULL`).
pub type MemoryHook = dyn Fn(&Value) -> Option<String> + Send + Sync;

/// [`run_pg`] for the in-memory backend. Without a test function the call
/// fails like a missing Postgres function.
pub(crate) fn run_memory(
    call: &TokenHookCall<'_>,
    issued: &IssuedSession,
    function: Option<&MemoryHook>,
) -> Result<Value, HookError> {
    pg_target(call)?;
    let Some(function) = function else {
        return Err(HookError::coded(format!(
            "Error running hook URI: {}",
            call.hook.uri
        )));
    };
    let input = call.input(issued);
    finish(PgOutcome::Returned(function(&input)), &call.hook.uri)
}

fn pg_target<'a>(call: &TokenHookCall<'a>) -> Result<&'a PgFunctionName, HookError> {
    match &call.hook.target {
        HookTarget::PgFunction(function) => Ok(function),
        HookTarget::Http => Err(HookError::NotImplemented),
        // `Manager.dispatch` returns a plain error; `IssueRefreshToken`
        // wraps it as `error generating jwt token`.
        HookTarget::Unsupported => Err(HookError::coded("error generating jwt token")),
    }
}

async fn invoke_pg(
    conn: &mut sqlx::PgConnection,
    function: &PgFunctionName,
    input: &Value,
) -> PgOutcome {
    let timeout = format!("set local statement_timeout TO '{STATEMENT_TIMEOUT_MS}'");
    if let Err(error) = sqlx::query(&timeout).execute(&mut *conn).await {
        return PgOutcome::Failed(error);
    }
    let can_switch: Result<bool, sqlx::Error> = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)
                AND pg_has_role($1, 'MEMBER')",
    )
    .bind(AUTH_ADMIN_ROLE)
    .fetch_one(&mut *conn)
    .await;
    match can_switch {
        Ok(true) => {
            let set_role = format!("set local role {AUTH_ADMIN_ROLE}");
            if let Err(error) = sqlx::query(&set_role).execute(&mut *conn).await {
                return PgOutcome::Failed(error);
            }
        }
        Ok(false) => {}
        Err(error) => return PgOutcome::Failed(error),
    }
    // The name is two validated Postgres identifiers, quoted.
    let call = format!("select ({}($1::jsonb))::text", function.as_str());
    let returned: Result<Option<String>, sqlx::Error> = sqlx::query_scalar(&call)
        .bind(input.to_string())
        .fetch_one(&mut *conn)
        .await;
    let returned = match returned {
        Ok(returned) => returned,
        Err(error) => return PgOutcome::Failed(error),
    };
    for reset in ["reset role", "set local statement_timeout TO default"] {
        if let Err(error) = sqlx::query(reset).execute(&mut *conn).await {
            return PgOutcome::Failed(error);
        }
    }
    PgOutcome::Returned(returned)
}

/// Map a hook result to claims or GoTrue's error.
pub(crate) fn finish(outcome: PgOutcome, uri: &str) -> Result<Value, HookError> {
    match outcome {
        PgOutcome::Returned(text) => output_claims(text.as_deref()),
        PgOutcome::Failed(error) => Err(postgres_failure(&error, uri)),
    }
}

/// `Manager.dispatch` wraps a non-HTTP error as `Error running hook URI`.
/// `handleResponseErrorHTTP` then shows a publicly visible Postgres error
/// instead.
fn postgres_failure(error: &sqlx::Error, uri: &str) -> HookError {
    let wrapped = HookError::coded(format!("Error running hook URI: {uri}"));
    let Some(database) = error.as_database_error() else {
        return wrapped;
    };
    let Some(pg) = database.try_downcast_ref::<sqlx::postgres::PgDatabaseError>() else {
        return wrapped;
    };
    let Some(status) = public_postgres_status(pg.code()) else {
        return wrapped;
    };
    HookError::Postgres {
        status,
        code: pg.code().to_string(),
        message: pg.message().to_string(),
        hint: pg.hint().unwrap_or("").to_string(),
        detail: pg.detail().unwrap_or("").to_string(),
        uri: uri.to_string(),
    }
}

/// `getHttpStatusCodeFromPostgresErrorCode`.
fn public_postgres_status(code: &str) -> Option<StatusCode> {
    if code.len() != 5 {
        return None;
    }
    if matches!(
        code,
        "P0001" | "23000" | "23001" | "23502" | "23503" | "23505" | "23514" | "23P01"
    ) {
        return Some(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let digits = code.strip_prefix("PT")?;
    let status: i64 = go_parse_int(digits)?;
    // Go writes whatever `strconv.ParseInt` returned; 0 means "not public".
    if status == 0 {
        return None;
    }
    u16::try_from(status)
        .ok()
        .and_then(|status| StatusCode::from_u16(status).ok())
        .or(Some(StatusCode::INTERNAL_SERVER_ERROR))
}

/// `strconv.ParseInt(s, 10, 0)` for the three characters after `PT`.
fn go_parse_int(digits: &str) -> Option<i64> {
    let unsigned = digits
        .strip_prefix('+')
        .or_else(|| digits.strip_prefix('-'))
        .unwrap_or(digits);
    if unsigned.is_empty() || !unsigned.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// `hookserrors.Check`, then `CustomAccessTokenOutput.UnmarshalJSON`, then
/// `validateTokenClaims`.
pub(crate) fn output_claims(text: Option<&str>) -> Result<Value, HookError> {
    let parsed: Option<Value> = text.and_then(|text| serde_json::from_str(text).ok());
    if let Some(error) = parsed.as_ref().and_then(hook_error) {
        return Err(error);
    }
    // SQL NULL leaves the output untouched: claims stay nil.
    let claims = match (text, parsed) {
        (None, _) => None,
        (Some(_), Some(Value::Object(object))) => match object.get("claims") {
            None => {
                return Err(HookError::uncoded(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "output claims field is missing",
                ))
            }
            Some(Value::Object(claims)) => Some(claims.clone()),
            Some(Value::Null) => None,
            Some(_) => return Err(HookError::coded("Error unmarshaling JSON output.")),
        },
        (Some(_), Some(Value::Null)) => {
            return Err(HookError::uncoded(
                StatusCode::INTERNAL_SERVER_ERROR,
                "output claims field is missing",
            ))
        }
        (Some(_), _) => return Err(HookError::coded("Error unmarshaling JSON output.")),
    };
    let errors = schema_errors(claims.as_ref());
    if !errors.is_empty() {
        let mut message = String::from("output claims do not conform to the expected schema: \n");
        for error in errors {
            message.push_str("- ");
            message.push_str(&error);
            message.push('\n');
        }
        return Err(HookError::uncoded(
            StatusCode::INTERNAL_SERVER_ERROR,
            message,
        ));
    }
    Ok(claims.map_or(Value::Null, Value::Object))
}

/// `hookserrors.fromBytes` + `check`: an `error` object with a non-empty
/// `message`. `http_code` 0 (or absent) is 500.
fn hook_error(output: &Value) -> Option<HookError> {
    let object = output.as_object()?;
    let error = match go_field(object, "error")? {
        Value::Null => return None,
        Value::Object(error) => error,
        _ => return None,
    };
    let http_code = match go_field(error, "http_code") {
        None | Some(Value::Null) => 0,
        Some(value) => value.as_i64()?,
    };
    let message = match go_field(error, "message") {
        None | Some(Value::Null) => "",
        Some(Value::String(message)) => message.as_str(),
        Some(_) => return None,
    };
    if message.is_empty() {
        return None;
    }
    let status = if http_code == 0 {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        u16::try_from(http_code)
            .ok()
            .and_then(|code| StatusCode::from_u16(code).ok())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    };
    Some(HookError::uncoded(status, message))
}

/// Go `encoding/json` struct field lookup: case-insensitive, and the key
/// decoded last wins. Postgres `jsonb` text orders keys by length, then bytes.
fn go_field<'a>(object: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    object
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case(name))
        .max_by(|(left, _), (right, _)| {
            left.len()
                .cmp(&right.len())
                .then_with(|| left.as_bytes().cmp(right.as_bytes()))
        })
        .map(|(_, value)| value)
}

/// `MinimumViableTokenSchema` required claims, in schema order.
const REQUIRED_CLAIMS: [&str; 10] = [
    "aud",
    "exp",
    "iat",
    "sub",
    "email",
    "phone",
    "role",
    "aal",
    "session_id",
    "is_anonymous",
];

/// `MinimumViableTokenSchema` property types, in schema order.
const CLAIM_TYPES: [(&str, &[&str]); 16] = [
    ("aud", &["string", "array"]),
    ("exp", &["integer"]),
    ("jti", &["string"]),
    ("iat", &["integer"]),
    ("iss", &["string"]),
    ("nbf", &["integer"]),
    ("sub", &["string"]),
    ("email", &["string"]),
    ("phone", &["string"]),
    ("app_metadata", &["object"]),
    ("user_metadata", &["object"]),
    ("role", &["string"]),
    ("aal", &["string"]),
    ("amr", &["array"]),
    ("session_id", &["string"]),
    ("client_id", &["string"]),
];

/// gojsonschema `ResultError.String()` lines (`field: description`).
///
/// Required failures come first, then property types. gojsonschema walks
/// properties in Go map order, which is not stable; this uses schema order.
fn schema_errors(claims: Option<&Map<String, Value>>) -> Vec<String> {
    let Some(claims) = claims else {
        return vec!["(root): Invalid type. Expected: object, given: null".into()];
    };
    let mut errors = Vec::new();
    for name in REQUIRED_CLAIMS {
        if !claims.contains_key(name) {
            errors.push(format!("(root): {name} is required"));
        }
    }
    for (name, types) in CLAIM_TYPES {
        let Some(value) = claims.get(name) else {
            continue;
        };
        if !type_matches(value, types) {
            errors.push(invalid_type(name, types, value));
            continue;
        }
        if name == "amr" {
            for (index, item) in value.as_array().into_iter().flatten().enumerate() {
                if !type_matches(item, &["string"]) && !type_matches(item, &["object"]) {
                    let field = format!("amr.{index}");
                    errors.push(format!(
                        "{field}: Must validate at least one schema (anyOf)"
                    ));
                    errors.push(invalid_type(&field, &["string"], item));
                }
            }
        }
    }
    errors
}

fn invalid_type(field: &str, types: &[&str], value: &Value) -> String {
    let expected = if types.len() == 1 {
        types[0].to_string()
    } else {
        format!("[{}]", types.join(","))
    };
    format!(
        "{field}: Invalid type. Expected: {expected}, given: {}",
        json_type(value)
    )
}

fn type_matches(value: &Value, types: &[&str]) -> bool {
    let given = json_type(value);
    types
        .iter()
        .any(|ty| *ty == given || (*ty == "number" && given == "integer"))
}

/// gojsonschema type names. A whole JSON number is `integer`.
fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) => {
            let whole = number.is_i64()
                || number.is_u64()
                || number.as_f64().is_some_and(|float| float.fract() == 0.0);
            if whole {
                "integer"
            } else {
                "number"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_string())
        }
    }

    fn valid_claims() -> Value {
        json!({
            "aud": "authenticated",
            "exp": 1_700_003_600,
            "iat": 1_700_000_000,
            "sub": "00000000-0000-0000-0000-000000000001",
            "email": "ada@example.com",
            "phone": "",
            "role": "authenticated",
            "aal": "aal1",
            "session_id": "00000000-0000-0000-0000-000000000002",
            "is_anonymous": false,
            "amr": [{"method": "password", "timestamp": 1_700_000_000}],
            "user_role": "admin",
        })
    }

    #[test]
    fn config_reads_the_pg_function_uri() {
        let hook = AccessTokenHook::from_lookup(&lookup(&[
            (ENV_ENABLED, "true"),
            (
                ENV_URI,
                "pg-functions://postgres/public/custom_access_token_hook",
            ),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(
            hook.target,
            HookTarget::PgFunction(PgFunctionName(
                "\"public\".\"custom_access_token_hook\"".into()
            ))
        );
        assert!(AccessTokenHook::from_lookup(&lookup(&[]))
            .unwrap()
            .is_none());
        // A valid URI with the hook disabled is accepted and ignored.
        assert!(AccessTokenHook::from_lookup(&lookup(&[(
            ENV_URI,
            "pg-functions://postgres/public/hook"
        )]))
        .unwrap()
        .is_none());
    }

    #[test]
    fn config_rejects_invalid_uris_even_when_disabled() {
        for uri in [
            "pg-functions://postgres/public",
            "pg-functions://postgres/public/bad-name",
            "pg-functions://postgres/1public/hook",
            "http://example.com/hook",
            "ftp://example.com/hook",
            "no-scheme",
        ] {
            let error = AccessTokenHook::from_lookup(&lookup(&[(ENV_URI, uri)]));
            assert!(error.is_err(), "{uri}");
        }
        assert!(AccessTokenHook::from_lookup(&lookup(&[(ENV_ENABLED, "maybe")])).is_err());
    }

    #[test]
    fn config_targets_http_and_mixed_case_schemes() {
        let http = AccessTokenHook::from_lookup(&lookup(&[
            (ENV_ENABLED, "1"),
            (ENV_URI, "https://hooks.example.com/token"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(http.target, HookTarget::Http);
        let local = AccessTokenHook::from_lookup(&lookup(&[
            (ENV_ENABLED, "1"),
            (ENV_URI, "http://localhost:54321/functions/v1/hook"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(local.target, HookTarget::Http);
        // `Validate` lowercases the scheme; the dispatcher prefix match does not.
        let upper = AccessTokenHook::from_lookup(&lookup(&[
            (ENV_ENABLED, "true"),
            (ENV_URI, "PG-FUNCTIONS://postgres/public/hook"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(upper.target, HookTarget::Unsupported);
    }

    #[test]
    fn valid_claims_pass_through() {
        let output = json!({ "claims": valid_claims() }).to_string();
        let claims = output_claims(Some(&output)).unwrap();
        assert_eq!(claims["user_role"], "admin");
        assert_eq!(claims["aud"], "authenticated");
    }

    #[test]
    fn hook_error_object_sets_status_and_message() {
        let output = r#"{"error":{"http_code":403,"message":"nope"}}"#;
        let error = output_claims(Some(output)).unwrap_err();
        assert_eq!(error, HookError::uncoded(StatusCode::FORBIDDEN, "nope"),);
        let output = r#"{"error":{"message":"boom"},"claims":{}}"#;
        assert_eq!(
            output_claims(Some(output)).unwrap_err(),
            HookError::uncoded(StatusCode::INTERNAL_SERVER_ERROR, "boom"),
        );
        // An empty message is not an error; the missing claims are.
        let output = r#"{"error":{"http_code":403}}"#;
        assert_eq!(
            output_claims(Some(output)).unwrap_err(),
            HookError::uncoded(
                StatusCode::INTERNAL_SERVER_ERROR,
                "output claims field is missing"
            ),
        );
        // Go matches struct fields case-insensitively.
        let output = r#"{"Error":{"Message":"upper"}}"#;
        assert_eq!(
            output_claims(Some(output)).unwrap_err(),
            HookError::uncoded(StatusCode::INTERNAL_SERVER_ERROR, "upper"),
        );
    }

    #[test]
    fn malformed_outputs_match_gotrue() {
        assert_eq!(
            output_claims(Some("[1]")).unwrap_err(),
            HookError::coded("Error unmarshaling JSON output."),
        );
        assert_eq!(
            output_claims(Some(r#"{"claims":"x"}"#)).unwrap_err(),
            HookError::coded("Error unmarshaling JSON output."),
        );
        assert_eq!(
            output_claims(Some("null")).unwrap_err(),
            HookError::uncoded(
                StatusCode::INTERNAL_SERVER_ERROR,
                "output claims field is missing"
            ),
        );
        let null_claims = HookError::uncoded(
            StatusCode::INTERNAL_SERVER_ERROR,
            "output claims do not conform to the expected schema: \n- (root): Invalid type. Expected: object, given: null\n",
        );
        assert_eq!(output_claims(None).unwrap_err(), null_claims);
        assert_eq!(
            output_claims(Some(r#"{"claims":null}"#)).unwrap_err(),
            null_claims
        );
    }

    #[test]
    fn schema_failures_list_required_then_types() {
        let mut claims = valid_claims();
        let object = claims.as_object_mut().unwrap();
        object.remove("aal");
        object.remove("is_anonymous");
        object.insert("exp".into(), json!("soon"));
        object.insert("aud".into(), json!(1));
        object.insert("iat".into(), json!(1.5));
        object.insert("amr".into(), json!(["password", 7]));
        let output = json!({ "claims": claims }).to_string();
        let HookError::Http {
            status,
            error_code,
            message,
        } = output_claims(Some(&output)).unwrap_err()
        else {
            panic!("expected an HTTP error");
        };
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error_code, None);
        assert_eq!(
            message,
            "output claims do not conform to the expected schema: \n\
             - (root): aal is required\n\
             - (root): is_anonymous is required\n\
             - aud: Invalid type. Expected: [string,array], given: integer\n\
             - exp: Invalid type. Expected: integer, given: string\n\
             - iat: Invalid type. Expected: integer, given: number\n\
             - amr.1: Must validate at least one schema (anyOf)\n\
             - amr.1: Invalid type. Expected: string, given: integer\n"
        );
    }

    #[test]
    fn errors_render_like_gotrue() {
        let response = HookError::uncoded(StatusCode::FORBIDDEN, "nope").into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(response.headers().get("x-sb-error-code").is_none());
        let response = HookError::coded("Error unmarshaling JSON output.").into_response();
        assert_eq!(
            response.headers().get("x-sb-error-code").unwrap(),
            "unexpected_failure"
        );
        assert_eq!(
            HookError::uncoded(StatusCode::FORBIDDEN, "nope").redirect_parts(),
            Some((403, "", "nope".to_string()))
        );
        assert!(HookError::NotImplemented.redirect_parts().is_none());
    }

    #[test]
    fn postgres_error_codes_map_like_postgrest() {
        assert_eq!(
            public_postgres_status("P0001"),
            Some(StatusCode::INTERNAL_SERVER_ERROR)
        );
        assert_eq!(public_postgres_status("PT403"), Some(StatusCode::FORBIDDEN));
        assert_eq!(public_postgres_status("PTabc"), None);
        assert_eq!(public_postgres_status("PT000"), None);
        assert_eq!(public_postgres_status("42883"), None);
        assert_eq!(public_postgres_status("P01"), None);
    }

    #[test]
    fn forwarded_ip_takes_the_first_parseable_entry() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static(" , junk, 203.0.113.9, 10.0.0.1"),
        );
        assert_eq!(forwarded_ip(&headers).as_deref(), Some("203.0.113.9"));
        assert_eq!(forwarded_ip(&HeaderMap::new()), None);
    }

    mod http {
        use std::sync::{Arc, Mutex};

        use axum::body::{to_bytes, Body};
        use axum::http::Request;
        use megabase_core::Hs256;
        use tower::ServiceExt;

        use super::*;
        use crate::config::AuthConfig;
        use crate::state::AuthState;
        use crate::store::{email_otp_hash, Backend, PlantFlags};

        const SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";

        /// Records each hook event and answers with `respond(event)`.
        fn recording_hook(
            respond: impl Fn(&Value) -> Option<String> + Send + Sync + 'static,
        ) -> (Arc<MemoryHook>, Arc<Mutex<Vec<Value>>>) {
            let events = Arc::new(Mutex::new(Vec::new()));
            let seen = Arc::clone(&events);
            let hook: Arc<MemoryHook> = Arc::new(move |event: &Value| {
                seen.lock().unwrap().push(event.clone());
                respond(event)
            });
            (hook, events)
        }

        /// GoTrue docs' RBAC example: add `user_role` to the incoming claims.
        fn add_user_role(event: &Value) -> Option<String> {
            let mut claims = event["claims"].clone();
            claims["user_role"] = json!("moderator");
            Some(json!({ "claims": claims }).to_string())
        }

        async fn app_with(
            hook: AccessTokenHook,
            function: Option<Arc<MemoryHook>>,
        ) -> (AuthState, Backend) {
            let backend = Backend::memory();
            if let Some(function) = function {
                backend.set_access_token_hook_for_test(function).await;
            }
            let mut config = AuthConfig::reference_defaults();
            config.custom_access_token_hook = Some(hook);
            let jwt = Hs256::new(SECRET.as_bytes()).unwrap();
            (AuthState::new(config, Some(jwt), backend.clone()), backend)
        }

        async fn post(state: &AuthState, path: &str, body: &str) -> (StatusCode, Value, HeaderMap) {
            let request = Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .header("x-forwarded-for", "198.51.100.7")
                .body(Body::from(body.to_string()))
                .unwrap();
            let response = crate::routes::router(state.clone())
                .oneshot(request)
                .await
                .unwrap();
            let status = response.status();
            let headers = response.headers().clone();
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            (
                status,
                serde_json::from_slice(&bytes).unwrap_or(Value::Null),
                headers,
            )
        }

        fn token_claims(body: &Value) -> Value {
            let token = body["access_token"].as_str().expect("access token");
            Value::Object(
                Hs256::new(SECRET.as_bytes())
                    .unwrap()
                    .verify(token)
                    .unwrap()
                    .raw,
            )
        }

        fn password() -> String {
            format!("pw-{}", Uuid::new_v4().simple())
        }

        #[tokio::test]
        async fn signup_password_and_refresh_sign_the_hook_claims() {
            let (function, events) = recording_hook(add_user_role);
            let (state, _) = app_with(
                AccessTokenHook::pg_function_for_test("public", "custom_access_token_hook"),
                Some(function),
            )
            .await;
            let secret = password();
            let credentials = json!({ "email": "mod@example.com", "password": secret }).to_string();
            let (status, signup, _) = post(&state, "/auth/v1/signup", &credentials).await;
            assert_eq!(status, StatusCode::OK, "{signup}");
            assert_eq!(token_claims(&signup)["user_role"], "moderator");

            let (status, login, _) =
                post(&state, "/auth/v1/token?grant_type=password", &credentials).await;
            assert_eq!(status, StatusCode::OK, "{login}");
            let claims = token_claims(&login);
            assert_eq!(claims["user_role"], "moderator");
            assert_eq!(claims["role"], "authenticated");

            let refresh = json!({ "refresh_token": login["refresh_token"] }).to_string();
            let (status, refreshed, _) =
                post(&state, "/auth/v1/token?grant_type=refresh_token", &refresh).await;
            assert_eq!(status, StatusCode::OK, "{refreshed}");
            assert_eq!(token_claims(&refreshed)["user_role"], "moderator");

            let events = events.lock().unwrap();
            let methods: Vec<&str> = events
                .iter()
                .map(|event| event["authentication_method"].as_str().unwrap())
                .collect();
            assert_eq!(methods, ["password", "password", "token_refresh"]);
            let login_event = &events[1];
            assert_eq!(login_event["user_id"], login["user"]["id"]);
            assert_eq!(login_event["claims"]["sub"], login["user"]["id"]);
            assert_eq!(login_event["claims"]["aud"], "authenticated");
            assert_eq!(login_event["claims"]["aal"], "aal1");
            assert_eq!(login_event["claims"]["is_anonymous"], false);
            assert_eq!(login_event["metadata"]["name"], HOOK_EVENT_NAME);
            assert_eq!(login_event["metadata"]["ip_address"], "198.51.100.7");
            assert!(login_event["metadata"]["uuid"].is_string());
            assert!(login_event["metadata"]["time"].is_string());
        }

        #[tokio::test]
        async fn verify_runs_the_hook_with_otp() {
            let (function, events) = recording_hook(add_user_role);
            let (state, backend) = app_with(
                AccessTokenHook::pg_function_for_test("public", "hook"),
                Some(function),
            )
            .await;
            let email = "person@example.com";
            let hash = email_otp_hash(email, "123456");
            backend
                .plant_verification_for_test(email, &hash, PlantFlags::default())
                .await;
            let body = json!({ "type": "signup", "token": "123456", "email": email }).to_string();
            let (status, verified, _) = post(&state, "/auth/v1/verify", &body).await;
            assert_eq!(status, StatusCode::OK, "{verified}");
            assert_eq!(token_claims(&verified)["user_role"], "moderator");
            assert_eq!(events.lock().unwrap()[0]["authentication_method"], "otp");
        }

        #[tokio::test]
        async fn hook_errors_become_gotrue_responses() {
            let (function, _) = recording_hook(|_| {
                Some(r#"{"error":{"http_code":403,"message":"Not allowed"}}"#.into())
            });
            let (state, _) = app_with(
                AccessTokenHook::pg_function_for_test("public", "hook"),
                Some(function),
            )
            .await;
            let credentials =
                json!({ "email": "deny@example.com", "password": password() }).to_string();
            let (status, body, headers) = post(&state, "/auth/v1/signup", &credentials).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(
                body,
                json!({ "code": 403, "error_code": "unknown", "msg": "Not allowed" })
            );
            assert!(headers.get("x-sb-error-code").is_none());

            let (function, _) = recording_hook(|_| Some(r#"{"claims":{"role":"x"}}"#.into()));
            let (state, _) = app_with(
                AccessTokenHook::pg_function_for_test("public", "hook"),
                Some(function),
            )
            .await;
            let (status, body, _) = post(&state, "/auth/v1/signup", &credentials).await;
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(body["error_code"], "unexpected_failure");
            let message = body["msg"].as_str().unwrap();
            assert!(
                message.starts_with("output claims do not conform to the expected schema: \n"),
                "{message}"
            );
            assert!(message.contains("- (root): aud is required\n"), "{message}");

            // No function installed: the Postgres call fails.
            let (state, _) = app_with(
                AccessTokenHook::pg_function_for_test("public", "missing"),
                None,
            )
            .await;
            let (status, body, headers) = post(&state, "/auth/v1/signup", &credentials).await;
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(
                body["msg"],
                "Error running hook URI: pg-functions://postgres/public/missing"
            );
            assert_eq!(
                headers.get("x-sb-error-code").unwrap(),
                "unexpected_failure"
            );
        }

        #[tokio::test]
        async fn http_hooks_are_not_implemented() {
            let hook = AccessTokenHook::from_lookup(&lookup(&[
                (ENV_ENABLED, "true"),
                (ENV_URI, "https://hooks.example.com/token"),
            ]))
            .unwrap()
            .unwrap();
            let (state, _) = app_with(hook, None).await;
            let credentials =
                json!({ "email": "web@example.com", "password": password() }).to_string();
            let (status, body, _) = post(&state, "/auth/v1/signup", &credentials).await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
            assert_eq!(body["unit"], UNIT);
        }
    }

    #[test]
    fn rfc3339_nano_trims_zeros() {
        let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(1_500);
        assert_eq!(rfc3339_nano(time), "1970-01-01T00:00:01.5Z");
        assert_eq!(rfc3339_nano(SystemTime::UNIX_EPOCH), "1970-01-01T00:00:00Z");
    }
}
