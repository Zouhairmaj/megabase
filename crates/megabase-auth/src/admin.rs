// Ported from supabase/auth (MIT), pin v2.197.0:
//   internal/api/api.go (admin route table)
//   internal/api/middleware.go (requireAdminCredentials, feature flags)
//   internal/api/auth.go (extractBearerToken, parseJWTClaims, requireAdmin)
//   internal/api/audit.go
//   internal/api/admin.go (loadUser, loadFactor, adminUserDelete, adminUserDeleteFactor)
//   internal/api/custom_oauth_admin.go
//   internal/api/ssoadmin.go
//   internal/api/passkey_admin.go
//   internal/api/oauthserver/handlers.go
//   internal/api/pagination.go
//   internal/models/audit_log_entry.go
//   internal/models/user.go
//   internal/models/custom_oauth_provider.go
//   internal/models/sso.go
//   internal/models/webauthn_credential.go

//! `/auth/v1/admin` routes. Issue #6 is the first GET/DELETE batch. Issue #7
//! adds the user, SSO, OAuth client, custom-provider, and generate-link
//! handlers in `admin_batch2`.

use std::collections::HashMap;

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header::AUTHORIZATION, HeaderMap},
    response::Response,
    routing::{delete, get, post},
    Router,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use megabase_core::jwt::bearer_token;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::http::{json_ok, no_content, AuthError};
use crate::state::AuthState;

const NIL_INSTANCE: &str = "00000000-0000-0000-0000-000000000000";
const AUTH_PREFIX: &str = "/auth/v1";

pub(crate) fn nil_instance() -> Uuid {
    Uuid::nil()
}

pub fn router(state: AuthState) -> Router {
    Router::new()
        .route("/auth/v1/admin/audit", get(get_audit))
        .route(
            "/auth/v1/admin/custom-providers",
            get(list_custom_providers).post(crate::admin_batch2::post_custom_provider),
        )
        .route(
            "/auth/v1/admin/custom-providers/:identifier",
            get(get_custom_provider)
                .put(crate::admin_batch2::batch3::update_custom_provider)
                .delete(delete_custom_provider),
        )
        .route(
            "/auth/v1/admin/oauth/clients",
            get(list_oauth_clients).post(crate::admin_batch2::post_oauth_client),
        )
        .route(
            "/auth/v1/admin/oauth/clients/:client_id",
            get(crate::admin_batch2::get_oauth_client)
                .put(crate::admin_batch2::batch3::update_oauth_client)
                .delete(delete_oauth_client),
        )
        .route(
            "/auth/v1/admin/oauth/clients/:client_id/regenerate_secret",
            post(crate::admin_batch2::batch3::regenerate_oauth_secret),
        )
        .route(
            "/auth/v1/admin/sso/providers",
            get(crate::admin_batch2::list_sso_providers)
                .post(crate::admin_batch2::batch3::create_sso_provider),
        )
        .route(
            "/auth/v1/admin/sso/providers/:idp_id",
            get(crate::admin_batch2::get_sso_provider)
                .put(crate::admin_batch2::batch3::update_sso_provider)
                .delete(delete_sso_provider),
        )
        .route(
            "/auth/v1/admin/users",
            get(crate::admin_batch2::list_users).post(crate::admin_batch2::batch3::create_user),
        )
        .route(
            "/auth/v1/admin/users/:user_id",
            get(crate::admin_batch2::get_user)
                .put(crate::admin_batch2::batch3::update_user)
                .delete(delete_user),
        )
        .route(
            "/auth/v1/admin/users/:user_id/factors",
            get(crate::admin_batch2::get_user_factors),
        )
        .route(
            "/auth/v1/admin/users/:user_id/passkeys",
            get(crate::admin_batch2::get_user_passkeys),
        )
        .route(
            "/auth/v1/admin/generate_link",
            post(crate::admin_batch2::generate_link),
        )
        .route(
            "/auth/v1/admin/users/:user_id/factors/:factor_id",
            delete(delete_factor).put(crate::admin_batch2::batch3::update_factor),
        )
        .route(
            "/auth/v1/admin/users/:user_id/passkeys/:passkey_id",
            delete(delete_passkey),
        )
        .with_state(state)
}

pub(crate) fn require_admin(state: &AuthState, headers: &HeaderMap) -> Result<(), AuthError> {
    verified_admin_role(state, headers).map(|_| ())
}

/// Admin JWT role (`service_role` by default). `requireAdmin` stores this on a
/// synthetic user whose email is the role string (`auth.go`).
pub(crate) fn verified_admin_role(
    state: &AuthState,
    headers: &HeaderMap,
) -> Result<String, AuthError> {
    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let token = bearer_token(authorization).ok_or_else(AuthError::no_authorization)?;
    let jwt = state
        .jwt
        .as_ref()
        .ok_or_else(|| AuthError::bad_jwt("missing JWT secret"))?;
    let claims = jwt.verify_gotrue(token).map_err(AuthError::bad_jwt)?;
    if !state.is_admin_role(claims.role.as_deref()) {
        return Err(AuthError::not_admin());
    }
    Ok(claims.role.unwrap_or_default())
}

pub(crate) fn require_custom_oauth(state: &AuthState) -> Result<(), AuthError> {
    if state.custom_oauth_enabled {
        Ok(())
    } else {
        Err(AuthError::feature_disabled(
            "Custom OAuth providers are disabled",
        ))
    }
}

pub(crate) fn require_oauth_server(state: &AuthState) -> Result<(), AuthError> {
    if state.oauth_server_enabled {
        Ok(())
    } else {
        Err(AuthError::feature_disabled("OAuth server is disabled"))
    }
}

pub(crate) fn is_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (i, byte) in bytes.iter().enumerate() {
        match i {
            8 | 13 | 18 | 23 => {
                if *byte != b'-' {
                    return false;
                }
            }
            _ => {
                if !byte.is_ascii_hexdigit() {
                    return false;
                }
            }
        }
    }
    true
}

pub(crate) fn pool(state: &AuthState) -> Result<sqlx::PgPool, AuthError> {
    state
        .backend
        .pg_pool()
        .ok_or_else(|| AuthError::internal("Database error"))
}

pub(crate) fn ts_json(time: Option<chrono::DateTime<chrono::Utc>>) -> Value {
    match time {
        Some(time) => Value::String(system_time_rfc3339(std::time::SystemTime::from(time))),
        None => Value::Null,
    }
}

#[derive(Debug, Deserialize, Default)]
struct AuditQuery {
    page: Option<String>,
    per_page: Option<String>,
    query: Option<String>,
}

pub(crate) fn parse_uint(raw: Option<&str>, default: u64) -> Result<u64, AuthError> {
    match raw {
        None | Some("") => Ok(default),
        Some(value) => value.parse::<u64>().map_err(|_| {
            // strconv.ParseUint message from internal/api/pagination.go:60.
            AuthError::validation(
                400,
                format!("Bad Pagination Parameters: strconv.ParseUint: parsing \"{value}\": invalid syntax"),
            )
        }),
    }
}

// megabase:unit auth:route:GET /auth/v1/admin/audit
async fn get_audit(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
    uri: axum::http::Uri,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    let page = parse_uint(query.page.as_deref(), 1)?;
    let per_page = parse_uint(query.per_page.as_deref(), 50)?;
    let filter = audit_filter(query.query.as_deref())?;
    let db = pool(&state)?;
    let mut conn = db
        .acquire()
        .await
        .map_err(|_| AuthError::internal("Error searching for audit logs"))?;
    let instance = nil_instance();
    let count = audit_count(&mut conn, &filter, instance).await?;
    let offset = page.saturating_sub(1).saturating_mul(per_page) as i64;
    let limit = per_page as i64;
    let logs = audit_page(&mut conn, &filter, instance, limit, offset).await?;

    let total = u64::try_from(count).unwrap_or(0);
    let total_pages = match per_page {
        0 => 0,
        _ => total / per_page + u64::from(total % per_page > 0),
    };
    let gotrue_path = uri.path().strip_prefix(AUTH_PREFIX).unwrap_or(uri.path());
    let mut pairs: Vec<(String, String)> = uri
        .query()
        .unwrap_or("")
        .split('&')
        .filter(|part| !part.is_empty())
        .filter_map(|part| {
            let (k, v) = part.split_once('=')?;
            Some((k.to_string(), v.to_string()))
        })
        .filter(|(k, _)| k != "page")
        .collect();
    let mut link = String::new();
    if total_pages > page {
        pairs.push(("page".into(), (page + 1).to_string()));
        link.push_str(&format!(
            "<{}>; rel=\"next\", ",
            format_link(gotrue_path, &pairs)
        ));
        pairs.pop();
    }
    pairs.push(("page".into(), total_pages.to_string()));
    link.push_str(&format!(
        "<{}>; rel=\"last\"",
        format_link(gotrue_path, &pairs)
    ));

    let mut response = json_ok(&Value::Array(logs));
    response
        .headers_mut()
        .insert("x-total-count", format!("{total}").parse().expect("digits"));
    if let Ok(value) = axum::http::HeaderValue::from_str(&link) {
        response.headers_mut().insert("link", value);
    }
    Ok(response)
}

enum AuditFilter {
    None,
    Author(String),
    Action(String),
    Type(String),
}

fn audit_filter(raw: Option<&str>) -> Result<AuditFilter, AuthError> {
    let Some(raw) = raw.filter(|query| !query.is_empty()) else {
        return Ok(AuditFilter::None);
    };
    let (scope, value) = raw
        .split_once(':')
        .ok_or_else(|| AuthError::validation(400, format!("Invalid query scope: {raw}")))?;
    // FindAuditLogEntries applies the ILIKE only when the value is non-empty.
    if value.is_empty() {
        return match scope {
            "author" | "action" | "type" => Ok(AuditFilter::None),
            _ => Err(AuthError::validation(
                400,
                format!("Invalid query scope: {raw}"),
            )),
        };
    }
    let like = format!("%{value}%");
    match scope {
        "author" => Ok(AuditFilter::Author(like)),
        "action" => Ok(AuditFilter::Action(like)),
        "type" => Ok(AuditFilter::Type(like)),
        _ => Err(AuthError::validation(
            400,
            format!("Invalid query scope: {raw}"),
        )),
    }
}

fn audit_entry(
    id: Uuid,
    payload: Option<Value>,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
    ip_address: &str,
) -> Value {
    json!({
        "id": id.to_string(),
        "payload": payload.unwrap_or(Value::Null),
        "created_at": ts_json(created_at),
        "ip_address": ip_address,
    })
}

async fn audit_count(
    conn: &mut sqlx::PgConnection,
    filter: &AuditFilter,
    instance: Uuid,
) -> Result<i64, AuthError> {
    let fail = |_| AuthError::internal("Error searching for audit logs");
    let count = match filter {
        AuditFilter::None => {
            sqlx::query!(
                "SELECT COUNT(*) AS count FROM auth.audit_log_entries WHERE instance_id = $1",
                instance,
            )
            .fetch_one(&mut *conn)
            .await
            .map_err(fail)?
            .count
        }
        AuditFilter::Author(like) => {
            sqlx::query!(
                "SELECT COUNT(*) AS count FROM auth.audit_log_entries
                 WHERE instance_id = $1
                   AND (payload->>'actor_username' ILIKE $2 OR payload->>'actor_name' ILIKE $2)",
                instance,
                like,
            )
            .fetch_one(&mut *conn)
            .await
            .map_err(fail)?
            .count
        }
        AuditFilter::Action(like) => {
            sqlx::query!(
                "SELECT COUNT(*) AS count FROM auth.audit_log_entries
                 WHERE instance_id = $1 AND payload->>'action' ILIKE $2",
                instance,
                like,
            )
            .fetch_one(&mut *conn)
            .await
            .map_err(fail)?
            .count
        }
        AuditFilter::Type(like) => {
            sqlx::query!(
                "SELECT COUNT(*) AS count FROM auth.audit_log_entries
                 WHERE instance_id = $1 AND payload->>'log_type' ILIKE $2",
                instance,
                like,
            )
            .fetch_one(&mut *conn)
            .await
            .map_err(fail)?
            .count
        }
    };
    Ok(count.unwrap_or(0))
}

async fn audit_page(
    conn: &mut sqlx::PgConnection,
    filter: &AuditFilter,
    instance: Uuid,
    limit: i64,
    offset: i64,
) -> Result<Vec<Value>, AuthError> {
    let fail = |_| AuthError::internal("Error searching for audit logs");
    let logs = match filter {
        AuditFilter::None => sqlx::query!(
            "SELECT id, payload, created_at, ip_address FROM auth.audit_log_entries
             WHERE instance_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
            instance,
            limit,
            offset,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(fail)?
        .into_iter()
        .map(|row| audit_entry(row.id, row.payload, row.created_at, &row.ip_address))
        .collect(),
        AuditFilter::Author(like) => sqlx::query!(
            "SELECT id, payload, created_at, ip_address FROM auth.audit_log_entries
             WHERE instance_id = $1
               AND (payload->>'actor_username' ILIKE $2 OR payload->>'actor_name' ILIKE $2)
             ORDER BY created_at DESC LIMIT $3 OFFSET $4",
            instance,
            like,
            limit,
            offset,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(fail)?
        .into_iter()
        .map(|row| audit_entry(row.id, row.payload, row.created_at, &row.ip_address))
        .collect(),
        AuditFilter::Action(like) => sqlx::query!(
            "SELECT id, payload, created_at, ip_address FROM auth.audit_log_entries
             WHERE instance_id = $1 AND payload->>'action' ILIKE $2
             ORDER BY created_at DESC LIMIT $3 OFFSET $4",
            instance,
            like,
            limit,
            offset,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(fail)?
        .into_iter()
        .map(|row| audit_entry(row.id, row.payload, row.created_at, &row.ip_address))
        .collect(),
        AuditFilter::Type(like) => sqlx::query!(
            "SELECT id, payload, created_at, ip_address FROM auth.audit_log_entries
             WHERE instance_id = $1 AND payload->>'log_type' ILIKE $2
             ORDER BY created_at DESC LIMIT $3 OFFSET $4",
            instance,
            like,
            limit,
            offset,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(fail)?
        .into_iter()
        .map(|row| audit_entry(row.id, row.payload, row.created_at, &row.ip_address))
        .collect(),
    };
    Ok(logs)
}

fn format_link(path: &str, pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        return path.to_string();
    }
    let query = pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    format!("{path}?{query}")
}

pub(crate) fn system_time_rfc3339(time: std::time::SystemTime) -> String {
    // Go `time.Time` JSON uses RFC3339Nano (fractional seconds, trailing zeros
    // stripped). Dropping sub-second precision via `as_secs()` would diverge.
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => format_unix(duration.as_secs() as i64, duration.subsec_nanos()),
        Err(_) => format_unix(0, 0),
    }
}

fn format_unix(secs: i64, nanos: u32) -> String {
    let secs = secs.max(0);
    let days = secs / 86400;
    let rem = secs % 86400;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;
    let (year, month, day) = civil_from_days(days);
    if nanos == 0 {
        return format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z");
    }
    let mut frac = format!("{nanos:09}");
    while frac.ends_with('0') {
        frac.pop();
    }
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}.{frac}Z")
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    // Howard Hinnant civil-from-days (proleptic Gregorian).
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[derive(Debug, Deserialize, Default)]
struct TypeQuery {
    #[serde(rename = "type")]
    provider_type: Option<String>,
}

// megabase:unit auth:route:GET /auth/v1/admin/custom-providers
async fn list_custom_providers(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Query(query): Query<TypeQuery>,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    require_custom_oauth(&state)?;
    match query.provider_type.as_deref() {
        None | Some("") | Some("oauth2") | Some("oidc") => {}
        Some(_) => {
            return Err(AuthError::validation(
                400,
                "type must be either 'oauth2' or 'oidc'",
            ))
        }
    }
    let db = pool(&state)?;
    let mut conn = db
        .acquire()
        .await
        .map_err(|_| AuthError::internal("Error retrieving custom OAuth providers"))?;
    let rows = match query.provider_type.as_deref() {
        Some(kind @ ("oauth2" | "oidc")) => sqlx::query_as!(
            CustomProviderRow,
            "SELECT id, provider_type, identifier, name, client_id,
                    acceptable_client_ids, scopes, pkce_enabled, attribute_mapping,
                    custom_claims_allowlist, authorization_params, enabled, email_optional,
                    issuer, discovery_url, skip_nonce_check, cached_discovery,
                    authorization_url, token_url, userinfo_url, jwks_uri,
                    created_at, updated_at
             FROM auth.custom_oauth_providers
             WHERE provider_type = $1
             ORDER BY created_at DESC",
            kind,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(|_| AuthError::internal("Error retrieving custom OAuth providers"))?,
        _ => sqlx::query_as!(
            CustomProviderRow,
            "SELECT id, provider_type, identifier, name, client_id,
                    acceptable_client_ids, scopes, pkce_enabled, attribute_mapping,
                    custom_claims_allowlist, authorization_params, enabled, email_optional,
                    issuer, discovery_url, skip_nonce_check, cached_discovery,
                    authorization_url, token_url, userinfo_url, jwks_uri,
                    created_at, updated_at
             FROM auth.custom_oauth_providers
             ORDER BY created_at DESC",
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(|_| AuthError::internal("Error retrieving custom OAuth providers"))?,
    };
    let providers: Vec<Value> = rows.iter().map(custom_provider_json).collect();
    Ok(json_ok(&json!({ "providers": providers })))
}

// megabase:unit auth:route:GET /auth/v1/admin/custom-providers/{identifier}
async fn get_custom_provider(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(identifier): Path<String>,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    require_custom_oauth(&state)?;
    validate_custom_identifier(&identifier)?;
    let db = pool(&state)?;
    let mut conn = db
        .acquire()
        .await
        .map_err(|_| AuthError::internal("Error retrieving custom OAuth provider"))?;
    let provider = load_custom_provider(&mut conn, &identifier).await?;
    Ok(json_ok(&provider))
}

// megabase:unit auth:route:DELETE /auth/v1/admin/custom-providers/{identifier}
async fn delete_custom_provider(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(identifier): Path<String>,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    require_custom_oauth(&state)?;
    validate_custom_identifier(&identifier)?;
    let db = pool(&state)?;
    let deleted = sqlx::query!(
        "DELETE FROM auth.custom_oauth_providers WHERE identifier = $1",
        identifier,
    )
    .execute(&db)
    .await
    .map_err(|_| AuthError::internal("Error deleting custom OAuth provider"))?
    .rows_affected();
    if deleted == 0 {
        return Err(AuthError::not_found(
            "custom_provider_not_found",
            "Custom OAuth provider not found",
        ));
    }
    Ok(no_content())
}

fn validate_custom_identifier(identifier: &str) -> Result<(), AuthError> {
    if identifier.is_empty() {
        return Err(AuthError::validation(400, "identifier is required"));
    }
    if !identifier.starts_with("custom:") {
        return Err(AuthError::validation(
            400,
            format!("identifier must start with 'custom:' prefix, e.g. 'custom:{identifier}'"),
        ));
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
pub(crate) struct CustomProviderRow {
    id: Uuid,
    provider_type: String,
    identifier: String,
    name: String,
    client_id: String,
    acceptable_client_ids: Vec<String>,
    scopes: Vec<String>,
    pkce_enabled: bool,
    attribute_mapping: Value,
    custom_claims_allowlist: Vec<String>,
    authorization_params: Value,
    enabled: bool,
    email_optional: bool,
    issuer: Option<String>,
    discovery_url: Option<String>,
    skip_nonce_check: bool,
    cached_discovery: Option<Value>,
    authorization_url: Option<String>,
    token_url: Option<String>,
    userinfo_url: Option<String>,
    jwks_uri: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

async fn load_custom_provider(
    conn: &mut sqlx::PgConnection,
    identifier: &str,
) -> Result<Value, AuthError> {
    let row = sqlx::query_as!(
        CustomProviderRow,
        "SELECT id, provider_type, identifier, name, client_id,
                acceptable_client_ids, scopes, pkce_enabled, attribute_mapping,
                custom_claims_allowlist, authorization_params, enabled, email_optional,
                issuer, discovery_url, skip_nonce_check, cached_discovery,
                authorization_url, token_url, userinfo_url, jwks_uri,
                created_at, updated_at
         FROM auth.custom_oauth_providers WHERE identifier = $1",
        identifier,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Error retrieving custom OAuth provider"))?;
    let Some(row) = row else {
        return Err(AuthError::not_found(
            "custom_provider_not_found",
            "Custom OAuth provider not found",
        ));
    };
    Ok(custom_provider_json(&row))
}

pub(crate) fn custom_provider_json(row: &CustomProviderRow) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("id".into(), json!(row.id.to_string()));
    object.insert("provider_type".into(), json!(row.provider_type));
    object.insert("identifier".into(), json!(row.identifier));
    object.insert("name".into(), json!(row.name));
    object.insert("client_id".into(), json!(row.client_id));
    object.insert(
        "acceptable_client_ids".into(),
        json!(row.acceptable_client_ids),
    );
    object.insert("scopes".into(), json!(row.scopes));
    object.insert("pkce_enabled".into(), json!(row.pkce_enabled));
    object.insert("attribute_mapping".into(), row.attribute_mapping.clone());
    object.insert(
        "custom_claims_allowlist".into(),
        json!(row.custom_claims_allowlist),
    );
    object.insert(
        "authorization_params".into(),
        row.authorization_params.clone(),
    );
    object.insert("enabled".into(), json!(row.enabled));
    object.insert("email_optional".into(), json!(row.email_optional));
    insert_opt_str(&mut object, "issuer", row.issuer.clone());
    insert_opt_str(&mut object, "discovery_url", row.discovery_url.clone());
    object.insert("skip_nonce_check".into(), json!(row.skip_nonce_check));
    if let Some(discovery) = row.cached_discovery.clone() {
        if !discovery.is_null() {
            object.insert("discovery_document".into(), discovery);
        }
    }
    insert_opt_str(
        &mut object,
        "authorization_url",
        row.authorization_url.clone(),
    );
    insert_opt_str(&mut object, "token_url", row.token_url.clone());
    insert_opt_str(&mut object, "userinfo_url", row.userinfo_url.clone());
    insert_opt_str(&mut object, "jwks_uri", row.jwks_uri.clone());
    object.insert("created_at".into(), ts_json(Some(row.created_at)));
    object.insert("updated_at".into(), ts_json(Some(row.updated_at)));
    Value::Object(object)
}

fn insert_opt_str(object: &mut serde_json::Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value.filter(|v| !v.is_empty()) {
        object.insert(key.into(), json!(value));
    }
}

// megabase:unit auth:route:GET /auth/v1/admin/oauth/clients
async fn list_oauth_clients(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    require_oauth_server(&state)?;
    let db = pool(&state)?;
    let rows = sqlx::query_as!(
        OAuthClientRow,
        "SELECT id, client_type::text AS client_type, redirect_uris, token_endpoint_auth_method,
                grant_types, client_name, client_uri, logo_uri, registration_type::text AS registration_type,
                created_at, updated_at
         FROM auth.oauth_clients WHERE deleted_at IS NULL ORDER BY created_at DESC",
    )
    .fetch_all(&db)
    .await
    .map_err(|_| AuthError::internal("Error listing OAuth clients"))?;
    if rows.is_empty() {
        return Ok(json_ok(&json!({})));
    }
    let clients: Vec<Value> = rows.iter().map(oauth_client_json).collect();
    Ok(json_ok(&json!({ "clients": clients })))
}

#[derive(sqlx::FromRow)]
pub(crate) struct OAuthClientRow {
    id: Uuid,
    client_type: Option<String>,
    redirect_uris: String,
    token_endpoint_auth_method: String,
    grant_types: String,
    client_name: Option<String>,
    client_uri: Option<String>,
    logo_uri: Option<String>,
    registration_type: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

pub(crate) fn oauth_client_json(row: &OAuthClientRow) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("client_id".into(), json!(row.id.to_string()));
    object.insert(
        "client_type".into(),
        json!(row.client_type.clone().unwrap_or_default()),
    );
    insert_nonempty_list(&mut object, "redirect_uris", &split_csv(&row.redirect_uris));
    insert_opt_str(
        &mut object,
        "token_endpoint_auth_method",
        Some(row.token_endpoint_auth_method.clone()),
    );
    insert_nonempty_list(&mut object, "grant_types", &split_csv(&row.grant_types));
    object.insert("response_types".into(), json!(["code"]));
    insert_opt_str(&mut object, "client_name", row.client_name.clone());
    insert_opt_str(&mut object, "client_uri", row.client_uri.clone());
    insert_opt_str(&mut object, "logo_uri", row.logo_uri.clone());
    insert_opt_str(
        &mut object,
        "registration_type",
        row.registration_type.clone(),
    );
    object.insert("created_at".into(), ts_json(Some(row.created_at)));
    object.insert("updated_at".into(), ts_json(Some(row.updated_at)));
    Value::Object(object)
}

fn split_csv(raw: &str) -> Vec<String> {
    if raw.is_empty() {
        Vec::new()
    } else {
        raw.split(',').map(str::to_string).collect()
    }
}

fn insert_nonempty_list(object: &mut serde_json::Map<String, Value>, key: &str, values: &[String]) {
    if !values.is_empty() {
        object.insert(key.into(), json!(values));
    }
}

// megabase:unit auth:route:DELETE /auth/v1/admin/oauth/clients/{client_id}
async fn delete_oauth_client(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(client_id): Path<String>,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    require_oauth_server(&state)?;
    if client_id.is_empty() {
        return Err(AuthError::validation(400, "client_id is required"));
    }
    if !is_uuid(&client_id) {
        return Err(AuthError::validation(400, "invalid client_id format"));
    }
    let id = Uuid::parse_str(&client_id)
        .map_err(|_| AuthError::validation(400, "invalid client_id format"))?;
    let db = pool(&state)?;
    let deleted = sqlx::query!(
        "UPDATE auth.oauth_clients SET deleted_at = NOW()
         WHERE id = $1 AND deleted_at IS NULL",
        id,
    )
    .execute(&db)
    .await
    .map_err(|_| AuthError::internal("Error deleting OAuth client"))?
    .rows_affected();
    if deleted == 0 {
        return Err(AuthError::not_found(
            "oauth_client_not_found",
            "OAuth client not found",
        ));
    }
    Ok(no_content())
}

struct SsoProviderRow {
    id: Uuid,
    resource_id: Option<String>,
    disabled: Option<bool>,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
    updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

// megabase:unit auth:route:DELETE /auth/v1/admin/sso/providers/{idp_id}
async fn delete_sso_provider(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(idp_id): Path<String>,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    let resource_id = idp_id.strip_prefix("resource_");
    if resource_id.is_none() && !is_uuid(&idp_id) {
        return Err(AuthError::not_found(
            "sso_provider_not_found",
            "SSO Identity Provider not found",
        ));
    }
    let db = pool(&state)?;
    let mut conn = db
        .acquire()
        .await
        .map_err(|_| AuthError::internal("Database error finding SSO Identity Provider"))?;
    let row = if let Some(resource_id) = resource_id {
        sqlx::query_as!(
            SsoProviderRow,
            "SELECT id, resource_id, disabled, created_at, updated_at
             FROM auth.sso_providers WHERE resource_id = $1",
            resource_id,
        )
        .fetch_optional(&mut *conn)
        .await
    } else {
        let id = Uuid::parse_str(&idp_id).map_err(|_| {
            AuthError::not_found("sso_provider_not_found", "SSO Identity Provider not found")
        })?;
        sqlx::query_as!(
            SsoProviderRow,
            "SELECT id, resource_id, disabled, created_at, updated_at
             FROM auth.sso_providers WHERE id = $1",
            id,
        )
        .fetch_optional(&mut *conn)
        .await
    }
    .map_err(|_| AuthError::internal("Database error finding SSO Identity Provider"))?;
    let Some(row) = row else {
        return Err(AuthError::not_found(
            "sso_provider_not_found",
            "SSO Identity Provider not found",
        ));
    };
    let saml = sqlx::query!(
        "SELECT entity_id, metadata_xml, metadata_url, name_id_format, attribute_mapping
         FROM auth.saml_providers WHERE sso_provider_id = $1",
        row.id,
    )
    .fetch_optional(&mut *conn)
    .await
    .ok()
    .flatten();
    let domains = sqlx::query!(
        "SELECT domain FROM auth.sso_domains WHERE sso_provider_id = $1",
        row.id,
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap_or_default();

    sqlx::query!("DELETE FROM auth.sso_providers WHERE id = $1", row.id)
        .execute(&mut *conn)
        .await
        .map_err(|_| AuthError::internal("Database error deleting SSO Identity Provider"))?;

    let mut provider = serde_json::Map::new();
    provider.insert("id".into(), json!(row.id.to_string()));
    if let Some(resource_id) = row.resource_id {
        provider.insert("resource_id".into(), json!(resource_id));
    }
    provider.insert("disabled".into(), json!(row.disabled));
    if let Some(saml) = saml {
        let mut saml_obj = serde_json::Map::new();
        saml_obj.insert("entity_id".into(), json!(saml.entity_id));
        // Metadata XML is cleared on list; delete returns the stored row, then
        // destroy. Match list's empty metadata so the body stays small.
        saml_obj.insert("metadata_xml".into(), json!(""));
        if let Some(url) = saml.metadata_url {
            saml_obj.insert("metadata_url".into(), json!(url));
        }
        provider.insert("saml".into(), Value::Object(saml_obj));
    }
    provider.insert(
        "domains".into(),
        json!(domains
            .into_iter()
            .map(|domain| json!({ "domain": domain.domain }))
            .collect::<Vec<_>>()),
    );
    provider.insert("created_at".into(), ts_json(row.created_at));
    provider.insert("updated_at".into(), ts_json(row.updated_at));
    Ok(json_ok(&Value::Object(provider)))
}

#[derive(Debug, Deserialize, Default)]
struct DeleteUserBody {
    should_soft_delete: Option<bool>,
}

// megabase:unit auth:route:DELETE /auth/v1/admin/users/{user_id}
async fn delete_user(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    body: Bytes,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    if !is_uuid(&user_id) {
        return Err(AuthError::not_found(
            "validation_failed",
            "user_id must be an UUID",
        ));
    }
    let id = Uuid::parse_str(&user_id)
        .map_err(|_| AuthError::not_found("validation_failed", "user_id must be an UUID"))?;
    let db = pool(&state)?;
    let mut conn = db
        .acquire()
        .await
        .map_err(|_| AuthError::internal("Database error loading user"))?;
    let user = load_user(&mut conn, id).await?;
    drop(conn);
    let soft = if body.is_empty() {
        false
    } else {
        serde_json::from_slice::<DeleteUserBody>(&body)
            .map_err(|_| AuthError::new(400, "bad_json", "Could not parse request body as JSON"))?
            .should_soft_delete
            .unwrap_or(false)
    };
    let mut tx = db
        .begin()
        .await
        .map_err(|_| AuthError::internal("Database error deleting user"))?;
    write_user_deleted_audit(&mut tx, &user).await?;
    if soft {
        if user.deleted_at.is_none() {
            soft_delete_user(&mut tx, &user).await?;
        }
    } else {
        sqlx::query!("DELETE FROM auth.users WHERE id = $1", user.id)
            .execute(&mut *tx)
            .await
            .map_err(|_| AuthError::internal("Database error deleting user"))?;
    }
    tx.commit()
        .await
        .map_err(|_| AuthError::internal("Database error deleting user"))?;
    Ok(json_ok(&json!({})))
}

async fn soft_delete_user(
    conn: &mut sqlx::PgConnection,
    user: &LoadedUser,
) -> Result<(), AuthError> {
    let user_key = user.id.to_string();
    let email = obfuscate_email(&user_key, user.email.as_deref().unwrap_or(""));
    let phone = obfuscate_phone(&user_key, user.phone.as_deref().unwrap_or(""));
    let email_change = obfuscate_email(&user_key, user.email_change.as_deref().unwrap_or(""));
    let phone_change = obfuscate_phone(&user_key, user.phone_change.as_deref().unwrap_or(""));
    sqlx::query!(
        "UPDATE auth.users SET
            email = $2, phone = $3, email_change = $4, phone_change = $5,
            encrypted_password = NULL,
            confirmation_token = '', recovery_token = '',
            email_change_token_current = '', email_change_token_new = '',
            phone_change_token = '', deleted_at = NOW(),
            raw_user_meta_data = '{}'::jsonb, raw_app_meta_data = '{}'::jsonb
         WHERE id = $1",
        user.id,
        email,
        phone,
        email_change,
        phone_change,
    )
    .execute(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Error soft deleting user"))?;
    sqlx::query!(
        "DELETE FROM auth.one_time_tokens WHERE user_id = $1",
        user.id,
    )
    .execute(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Error soft deleting user"))?;
    soft_delete_user_identities(conn, &user_key, user.id).await?;
    sqlx::query!("DELETE FROM auth.mfa_factors WHERE user_id = $1", user.id,)
        .execute(&mut *conn)
        .await
        .map_err(|_| AuthError::internal("Error deleting user's factors"))?;
    sqlx::query!(
        "DELETE FROM auth.webauthn_credentials WHERE user_id = $1",
        user.id,
    )
    .execute(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Error deleting user's WebAuthn credentials"))?;
    sqlx::query!("DELETE FROM auth.sessions WHERE user_id = $1", user.id)
        .execute(&mut *conn)
        .await
        .map_err(|_| AuthError::internal("Error deleting user's sessions"))?;
    Ok(())
}

async fn soft_delete_user_identities(
    conn: &mut sqlx::PgConnection,
    user_key: &str,
    user_id: Uuid,
) -> Result<(), AuthError> {
    let rows = sqlx::query!(
        "SELECT id, provider, provider_id FROM auth.identities WHERE user_id = $1",
        user_id,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Error soft deleting user identities"))?;
    for row in rows {
        let obfuscated =
            obfuscate_value(user_key, &format!("{}:{}", row.provider, row.provider_id));
        sqlx::query!(
            "UPDATE auth.identities SET identity_data = '{}'::jsonb, provider_id = $2
             WHERE id = $1",
            row.id,
            obfuscated,
        )
        .execute(&mut *conn)
        .await
        .map_err(|_| AuthError::internal("Error soft deleting user identities"))?;
    }
    Ok(())
}

struct LoadedUser {
    id: Uuid,
    email: Option<String>,
    phone: Option<String>,
    email_change: Option<String>,
    phone_change: Option<String>,
    deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn load_user(conn: &mut sqlx::PgConnection, user_id: Uuid) -> Result<LoadedUser, AuthError> {
    let instance = nil_instance();
    let row = sqlx::query!(
        "SELECT email, phone, email_change, phone_change, deleted_at FROM auth.users
         WHERE instance_id = $1 AND id = $2",
        instance,
        user_id,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Database error loading user"))?;
    let Some(row) = row else {
        return Err(AuthError::not_found("user_not_found", "User not found"));
    };
    Ok(LoadedUser {
        id: user_id,
        email: row.email,
        phone: row.phone,
        email_change: row.email_change,
        phone_change: row.phone_change,
        deleted_at: row.deleted_at,
    })
}

async fn write_user_deleted_audit(
    conn: &mut sqlx::PgConnection,
    user: &LoadedUser,
) -> Result<(), AuthError> {
    let instance = nil_instance();
    let payload = json!({
        "actor_id": NIL_INSTANCE,
        "actor_via_sso": false,
        "actor_username": "service_role",
        "action": "user_deleted",
        "log_type": "team",
        "traits": {
            "user_id": user.id.to_string(),
            "user_email": user.email,
            "user_phone": user.phone,
        }
    });
    sqlx::query!(
        "INSERT INTO auth.audit_log_entries (instance_id, id, payload, created_at, ip_address)
         VALUES ($1, gen_random_uuid(), $2, NOW(), '')",
        instance,
        payload,
    )
    .execute(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Error recording audit log entry"))?;
    Ok(())
}

fn obfuscate_value(user_id: &str, value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(user_id.as_bytes());
    hasher.update(value.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

fn obfuscate_email(user_id: &str, email: &str) -> String {
    obfuscate_value(user_id, email)
}

fn obfuscate_phone(user_id: &str, phone: &str) -> String {
    obfuscate_value(user_id, phone).chars().take(15).collect()
}

// megabase:unit auth:route:DELETE /auth/v1/admin/users/{user_id}/factors/{factor_id}
async fn delete_factor(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(params): Path<HashMap<String, String>>,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    let user_id = params.get("user_id").map(String::as_str).unwrap_or("");
    let factor_id = params.get("factor_id").map(String::as_str).unwrap_or("");
    if !is_uuid(user_id) {
        return Err(AuthError::not_found(
            "validation_failed",
            "user_id must be an UUID",
        ));
    }
    let user_uuid = Uuid::parse_str(user_id)
        .map_err(|_| AuthError::not_found("validation_failed", "user_id must be an UUID"))?;
    let db = pool(&state)?;
    let mut conn = db
        .acquire()
        .await
        .map_err(|_| AuthError::internal("Database error loading user"))?;
    let _user = load_user(&mut conn, user_uuid).await?;
    if !is_uuid(factor_id) {
        return Err(AuthError::not_found(
            "validation_failed",
            "factor_id must be an UUID",
        ));
    }
    let factor_uuid = Uuid::parse_str(factor_id)
        .map_err(|_| AuthError::not_found("validation_failed", "factor_id must be an UUID"))?;
    let row = sqlx::query_as!(
        FactorRow,
        "SELECT id, friendly_name, factor_type::text AS factor_type, status::text AS status,
                created_at, updated_at, phone, last_challenged_at
         FROM auth.mfa_factors WHERE user_id = $1 AND id = $2",
        user_uuid,
        factor_uuid,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Database error loading factor"))?;
    drop(conn);
    let Some(row) = row else {
        return Err(AuthError::not_found(
            "mfa_factor_not_found",
            "Factor not found",
        ));
    };
    let mut factor = serde_json::Map::new();
    let factor_type = row.factor_type.clone().unwrap_or_default();
    factor.insert("id".into(), json!(row.id.to_string()));
    if let Some(name) = row.friendly_name.clone().filter(|name| !name.is_empty()) {
        factor.insert("friendly_name".into(), json!(name));
    }
    factor.insert("factor_type".into(), json!(factor_type));
    factor.insert(
        "status".into(),
        json!(row.status.clone().unwrap_or_default()),
    );
    factor.insert("created_at".into(), ts_json(Some(row.created_at)));
    factor.insert("updated_at".into(), ts_json(Some(row.updated_at)));
    factor.insert("phone".into(), json!(row.phone.unwrap_or_default()));
    factor.insert("last_challenged_at".into(), ts_json(row.last_challenged_at));

    let amr = amr_method_for_factor_type(&factor_type)?;
    let mut tx = db
        .begin()
        .await
        .map_err(|_| AuthError::internal("Database error deleting factor"))?;
    sqlx::query!("DELETE FROM auth.mfa_factors WHERE id = $1", factor_uuid)
        .execute(&mut *tx)
        .await
        .map_err(|_| AuthError::internal("Database error deleting factor"))?;
    let sessions = sqlx::query!(
        "SELECT id FROM auth.sessions WHERE factor_id = $1",
        factor_uuid,
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|_| AuthError::internal("Database error downgrading sessions"))?;
    for session in sessions {
        sqlx::query!(
            "DELETE FROM auth.mfa_amr_claims
             WHERE session_id = $1 AND authentication_method = $2",
            session.id,
            amr,
        )
        .execute(&mut *tx)
        .await
        .map_err(|_| AuthError::internal("Database error downgrading sessions"))?;
    }
    sqlx::query!(
        "UPDATE auth.sessions SET aal = 'aal1', factor_id = NULL
         WHERE user_id = $1 AND factor_id = $2",
        user_uuid,
        factor_uuid,
    )
    .execute(&mut *tx)
    .await
    .map_err(|_| AuthError::internal("Database error downgrading sessions"))?;
    tx.commit()
        .await
        .map_err(|_| AuthError::internal("Database error deleting factor"))?;
    Ok(json_ok(&Value::Object(factor)))
}

struct FactorRow {
    id: Uuid,
    friendly_name: Option<String>,
    factor_type: Option<String>,
    status: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    phone: Option<String>,
    last_challenged_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn amr_method_for_factor_type(factor_type: &str) -> Result<String, AuthError> {
    let method = match factor_type {
        "totp" => "totp",
        "phone" => "mfa/phone",
        "webauthn" => "mfa/webauthn",
        "recovery_code" => "mfa/recovery_code",
        _ => return Err(AuthError::internal("Database error downgrading sessions")),
    };
    Ok(method.to_string())
}

// megabase:unit auth:route:DELETE /auth/v1/admin/users/{user_id}/passkeys/{passkey_id}
async fn delete_passkey(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(params): Path<HashMap<String, String>>,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    let user_id = params.get("user_id").map(String::as_str).unwrap_or("");
    let passkey_id = params.get("passkey_id").map(String::as_str).unwrap_or("");
    if !is_uuid(user_id) {
        return Err(AuthError::not_found(
            "validation_failed",
            "user_id must be an UUID",
        ));
    }
    let user_uuid = Uuid::parse_str(user_id)
        .map_err(|_| AuthError::not_found("validation_failed", "user_id must be an UUID"))?;
    let db = pool(&state)?;
    let mut conn = db
        .acquire()
        .await
        .map_err(|_| AuthError::internal("Database error loading user"))?;
    let _user = load_user(&mut conn, user_uuid).await?;
    drop(conn);
    if !is_uuid(passkey_id) {
        return Err(AuthError::not_found(
            "validation_failed",
            "Passkey not found",
        ));
    }
    let passkey_uuid = Uuid::parse_str(passkey_id)
        .map_err(|_| AuthError::not_found("validation_failed", "Passkey not found"))?;
    let deleted = sqlx::query!(
        "DELETE FROM auth.webauthn_credentials WHERE id = $1 AND user_id = $2",
        passkey_uuid,
        user_uuid,
    )
    .execute(&db)
    .await
    .map_err(|_| AuthError::internal("Database error deleting passkey"))?
    .rows_affected();
    if deleted == 0 {
        return Err(AuthError::not_found(
            "validation_failed",
            "Passkey not found",
        ));
    }
    Ok(no_content())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obfuscate_matches_gotrue_sha256() {
        let user_id = "11111111-1111-1111-1111-111111111111";
        let email = obfuscate_email(user_id, "user@example.com");
        assert_eq!(email.len(), 43);
        assert_eq!(email, obfuscate_value(user_id, "user@example.com"));
        let phone = obfuscate_phone(user_id, "+15551234567");
        assert_eq!(phone.len(), 15);
        assert_eq!(phone, &obfuscate_value(user_id, "+15551234567")[..15]);
    }

    #[test]
    fn amr_methods_match_factor_types() {
        assert_eq!(amr_method_for_factor_type("totp").unwrap(), "totp");
        assert_eq!(amr_method_for_factor_type("phone").unwrap(), "mfa/phone");
        assert_eq!(
            amr_method_for_factor_type("webauthn").unwrap(),
            "mfa/webauthn"
        );
        assert_eq!(
            amr_method_for_factor_type("recovery_code").unwrap(),
            "mfa/recovery_code"
        );
        assert_eq!(
            amr_method_for_factor_type("unknown")
                .unwrap_err()
                .error_code,
            "unexpected_failure"
        );
    }

    #[test]
    fn split_csv_omits_empty() {
        assert!(split_csv("").is_empty());
        assert_eq!(
            split_csv("https://a.example,https://b.example"),
            ["https://a.example", "https://b.example"]
        );
    }

    #[test]
    fn rfc3339_preserves_fractional_seconds() {
        use std::time::{Duration, UNIX_EPOCH};
        // 2024-01-01T00:00:00Z
        let epoch = UNIX_EPOCH + Duration::from_secs(1_704_067_200);
        assert_eq!(system_time_rfc3339(epoch), "2024-01-01T00:00:00Z");
        assert_eq!(
            system_time_rfc3339(epoch + Duration::new(0, 123_456_000)),
            "2024-01-01T00:00:00.123456Z"
        );
        assert_eq!(
            system_time_rfc3339(epoch + Duration::new(0, 123_000_000)),
            "2024-01-01T00:00:00.123Z"
        );
        assert_eq!(
            system_time_rfc3339(epoch + Duration::new(0, 1_000)),
            "2024-01-01T00:00:00.000001Z"
        );
    }
}
