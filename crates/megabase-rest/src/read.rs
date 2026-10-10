//! Resource routes under `/rest/v1/`.
//!
//! Reads, writes, `OPTIONS`, and the root OpenAPI document follow
//! `specs/rest/resources.md`. RPC follows `specs/rest/rpc.md`. Embeds,
//! `Prefer: return`/`tx`/`timezone`, and other media types stay
//! [`megabase_core::MegabaseNotImplemented`]. `Prefer` itself follows
//! `specs/rest/prefer.md`. Values are bound parameters.
//! Relation and column names come from `pg_catalog` after a bound lookup,
//! then `quote_ident`.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{OriginalUri, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use megabase_core::{bearer_token, JwtError, MegabaseNotImplemented};
use serde_json::{Map, Value};

use crate::filter::{quote_ident, UnsafeType};
use crate::mutate::mutate;
use crate::openapi::document;
use crate::params::{build_read_sql, parse_get_query, where_clause, QueryFail, Relationship};
use crate::prefer::{set_applied, Count, Prefer, Scope};
use crate::query::path_decode;
use crate::RestState;

pub(crate) const JSON_UTF8: &str = "application/json; charset=utf-8";

/// Schemas exposed by the pinned Supabase `PGRST_DB_SCHEMAS`.
const EXPOSED_SCHEMAS: &[&str] = &["public", "graphql_public"];

pub(crate) fn router(state: RestState) -> Router {
    Router::new().fallback(dispatch).with_state(state)
}

enum Target {
    Root,
    Relation(String),
    Rpc(String),
    Invalid,
    Outside,
}

async fn dispatch(
    State(state): State<RestState>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = uri.path();
    match classify(path) {
        Target::Outside => not_implemented(&method, path),
        Target::Rpc(function) => {
            crate::rpc::call(&state, &method, &uri, &headers, &function, &body).await
        }
        Target::Invalid => pgrst(
            StatusCode::NOT_FOUND,
            "PGRST125",
            "Invalid path specified in request URL",
            None,
            None,
        ),
        Target::Root => root(&state, &method, &headers).await,
        Target::Relation(relation) => {
            relation_route(&state, &method, &uri, &headers, &relation, &body).await
        }
    }
}

fn classify(path: &str) -> Target {
    let Some(rest) = path.strip_prefix("/rest/v1") else {
        return Target::Outside;
    };
    if !rest.is_empty() && !rest.starts_with('/') {
        return Target::Outside;
    }
    let mut parts = Vec::new();
    for segment in rest.split('/') {
        if segment.is_empty() {
            continue;
        }
        let name = path_decode(segment);
        if name.is_empty() || name.contains('\0') || name.contains('/') {
            return Target::Invalid;
        }
        parts.push(name);
    }
    match parts.as_slice() {
        [] => Target::Root,
        [name] => Target::Relation(name.clone()),
        [rpc, name] if rpc == "rpc" => Target::Rpc(name.clone()),
        _ => Target::Invalid,
    }
}

async fn root(state: &RestState, method: &Method, headers: &HeaderMap) -> Response {
    let session = match session(state, headers.get(header::AUTHORIZATION)) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let schema = match negotiated_schema(method, headers) {
        Ok(schema) => schema,
        Err(response) => return *response,
    };
    match *method {
        Method::GET => {
            // megabase:unit rest:route:GET /rest/v1/
            serve_openapi(state, &session, &schema, false, headers).await
        }
        Method::HEAD => {
            // megabase:unit rest:route:HEAD /rest/v1/
            serve_openapi(state, &session, &schema, true, headers).await
        }
        Method::OPTIONS => {
            // megabase:unit rest:route:OPTIONS /rest/v1/
            options_response("OPTIONS,GET,HEAD")
        }
        _ => pgrst(
            StatusCode::METHOD_NOT_ALLOWED,
            "PGRST117",
            &format!("Unsupported HTTP method: {method}"),
            None,
            None,
        ),
    }
}

async fn serve_openapi(
    state: &RestState,
    session: &Session,
    schema: &str,
    headers_only: bool,
    headers: &HeaderMap,
) -> Response {
    let Some(pool) = state.pool.as_ref() else {
        return database_unavailable();
    };
    document(pool, schema, session, headers_only, headers).await
}

async fn relation_route(
    state: &RestState,
    method: &Method,
    uri: &axum::http::Uri,
    headers: &HeaderMap,
    relation: &str,
    body: &[u8],
) -> Response {
    let path = uri.path();
    let session = match session(state, headers.get(header::AUTHORIZATION)) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let schema = match negotiated_schema(method, headers) {
        Ok(schema) => schema,
        Err(response) => return *response,
    };
    if method == Method::OPTIONS {
        // megabase:unit rest:route:OPTIONS /rest/v1/{relation}
        return relation_options(state, &schema, relation, session.anon).await;
    }
    if !matches!(
        method,
        &Method::GET
            | &Method::HEAD
            | &Method::POST
            | &Method::PUT
            | &Method::PATCH
            | &Method::DELETE
    ) {
        return pgrst(
            StatusCode::METHOD_NOT_ALLOWED,
            "PGRST117",
            &format!("Unsupported HTTP method: {method}"),
            None,
            None,
        );
    }
    let raw = uri.query().unwrap_or("");
    let query = match parse_resource_query(method, path, raw) {
        Ok(query) => query,
        Err(response) => return *response,
    };
    let prefer = Prefer::from_headers(headers);
    if let Some(unit) = prefer.unserved {
        return unimplemented_unit(method, path, unit);
    }
    if let Err(response) = prefer.check_strict() {
        return *response;
    }
    let shape = if matches!(*method, Method::GET | Method::HEAD) {
        ReadShape::from_accept(headers.get(header::ACCEPT))
    } else {
        None
    };
    if shape.is_none() {
        if let Some(unit) = reject_accept(headers.get(header::ACCEPT)) {
            return unimplemented_unit(method, path, unit);
        }
    }
    if method == Method::GET && headers.get(header::RANGE).is_some() {
        return not_implemented(method, path);
    }
    let pool = state.pool.as_ref();
    match *method {
        Method::GET => {
            // megabase:unit rest:route:GET /rest/v1/{relation}
            let Some(pool) = pool else {
                return database_unavailable();
            };
            read_relation(
                pool,
                &schema,
                relation,
                &session,
                &query,
                ReadMode {
                    aggregates: state.aggregates,
                    headers_only: false,
                    shape,
                },
                &prefer,
            )
            .await
        }
        Method::HEAD => {
            // megabase:unit rest:route:HEAD /rest/v1/{relation}
            let Some(pool) = pool else {
                return database_unavailable();
            };
            read_relation(
                pool,
                &schema,
                relation,
                &session,
                &query,
                ReadMode {
                    aggregates: state.aggregates,
                    headers_only: true,
                    shape,
                },
                &prefer,
            )
            .await
        }
        Method::POST => {
            // megabase:unit rest:route:POST /rest/v1/{relation}
            mutate(
                pool,
                &schema,
                relation,
                &session,
                method,
                &query,
                &prefer,
                body,
                headers.get(header::CONTENT_TYPE),
            )
            .await
        }
        Method::PUT => {
            // megabase:unit rest:route:PUT /rest/v1/{relation}
            mutate(
                pool,
                &schema,
                relation,
                &session,
                method,
                &query,
                &prefer,
                body,
                headers.get(header::CONTENT_TYPE),
            )
            .await
        }
        Method::PATCH => {
            // megabase:unit rest:route:PATCH /rest/v1/{relation}
            mutate(
                pool,
                &schema,
                relation,
                &session,
                method,
                &query,
                &prefer,
                body,
                headers.get(header::CONTENT_TYPE),
            )
            .await
        }
        Method::DELETE => {
            // megabase:unit rest:route:DELETE /rest/v1/{relation}
            mutate(
                pool,
                &schema,
                relation,
                &session,
                method,
                &query,
                &prefer,
                body,
                headers.get(header::CONTENT_TYPE),
            )
            .await
        }
        _ => not_implemented(method, path),
    }
}

fn parse_resource_query(
    method: &Method,
    path: &str,
    raw: &str,
) -> Result<crate::params::ReadQuery, Box<Response>> {
    match parse_get_query(raw) {
        Ok(query) if query.handled() || query_is_blank(raw) => Ok(query),
        Ok(_) => Err(Box::new(not_implemented(method, path))),
        Err(fail) => Err(Box::new(query_fail(method, path, fail))),
    }
}

fn query_is_blank(raw: &str) -> bool {
    raw.split('&').all(|part| part.is_empty())
}

pub(crate) fn negotiated_schema(
    method: &Method,
    headers: &HeaderMap,
) -> Result<String, Box<Response>> {
    let name = if matches!(
        method,
        &Method::POST | &Method::PUT | &Method::PATCH | &Method::DELETE
    ) {
        "content-profile"
    } else {
        "accept-profile"
    };
    let Some(profile) = header_text(headers, name) else {
        return Ok("public".to_string());
    };
    if !EXPOSED_SCHEMAS.contains(&profile.as_str()) {
        return Err(Box::new(pgrst(
            StatusCode::NOT_ACCEPTABLE,
            "PGRST106",
            &format!("Invalid schema: {profile}"),
            None,
            Some("Only the following schemas are exposed: public, graphql_public"),
        )));
    }
    Ok(profile)
}

pub(crate) fn database_unavailable() -> Response {
    pgrst(
        StatusCode::SERVICE_UNAVAILABLE,
        "PGRST000",
        "Database connection error.",
        Some("DATABASE_URL is unset"),
        None,
    )
}

async fn relation_options(state: &RestState, schema: &str, relation: &str, anon: bool) -> Response {
    let Some(pool) = state.pool.as_ref() else {
        return database_unavailable();
    };
    let mut tx = match begin(pool).await {
        Ok(tx) => tx,
        Err(response) => return *response,
    };
    let privileges: Option<(bool, bool, bool, bool)> = match sqlx::query_as(
        "SELECT
            (
              c.relkind IN ('r', 'p')
              OR (
                c.relkind IN ('v', 'f')
                AND (pg_relation_is_updatable(c.oid::regclass, true) & 8) = 8
              )
            ),
            (
              c.relkind IN ('r', 'p')
              OR (
                c.relkind IN ('v', 'f')
                AND (pg_relation_is_updatable(c.oid::regclass, true) & 4) = 4
              )
            ),
            (
              c.relkind IN ('r', 'p')
              OR (
                c.relkind IN ('v', 'f')
                AND (pg_relation_is_updatable(c.oid::regclass, true) & 16) = 16
              )
            ),
            EXISTS (
              SELECT 1 FROM pg_constraint AS k
              WHERE k.conrelid = c.oid AND k.contype = 'p'
            )
         FROM pg_class AS c
         JOIN pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname = $1
           AND c.relname = $2
           AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
           AND NOT c.relispartition",
    )
    .bind(schema)
    .bind(relation)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(row) => row,
        Err(error) => return query_failure(&error, anon),
    };
    let Some((insertable, updatable, deletable, has_pk)) = privileges else {
        return table_not_found(schema, relation);
    };
    let mut allow = vec!["OPTIONS", "GET", "HEAD"];
    if insertable {
        allow.push("POST");
    }
    if insertable && updatable && has_pk {
        allow.push("PUT");
    }
    if updatable {
        allow.push("PATCH");
    }
    if deletable {
        allow.push("DELETE");
    }
    options_response(&allow.join(","))
}

pub(crate) fn options_response(allow: &str) -> Response {
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .body(axum::body::Body::empty())
        .unwrap_or_else(|_| Response::new(axum::body::Body::empty()));
    if let Ok(value) = HeaderValue::from_str(allow) {
        response.headers_mut().insert(header::ALLOW, value);
    }
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}

#[cfg(test)]
fn relation_name(path: &str) -> Option<String> {
    match classify(path) {
        Target::Relation(name) => Some(name),
        _ => None,
    }
}

#[cfg(test)]
fn rpc_name(path: &str) -> Option<String> {
    match classify(path) {
        Target::Rpc(name) => Some(name),
        _ => None,
    }
}

pub(crate) fn query_fail(method: &Method, path: &str, fail: QueryFail) -> Response {
    match fail {
        QueryFail::Parse { message, details } => pgrst(
            StatusCode::BAD_REQUEST,
            "PGRST100",
            &message,
            Some(&details),
            None,
        ),
        QueryFail::NotEmbedded { resource } => pgrst(
            StatusCode::BAD_REQUEST,
            "PGRST108",
            &format!("'{resource}' is not an embedded resource in this request"),
            None,
            Some(&format!(
                "Verify that '{resource}' is included in the 'select' query parameter."
            )),
        ),
        QueryFail::Route => not_implemented(method, path),
    }
}

pub(crate) struct Session {
    pub(crate) role: String,
    pub(crate) claims: String,
    /// `true` when the role is the anon role. PostgREST uses that to choose
    /// 401 versus 403 for `42501`.
    pub(crate) anon: bool,
}

pub(crate) fn session(
    state: &RestState,
    authorization: Option<&axum::http::HeaderValue>,
) -> Result<Session, Box<Response>> {
    let header = authorization
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if header.is_empty() {
        return Ok(anon_session());
    }
    let Some(token) = bearer_token(header) else {
        return Ok(anon_session());
    };
    let Some(jwt) = state.jwt.as_ref() else {
        return Err(Box::new(pgrst(
            StatusCode::INTERNAL_SERVER_ERROR,
            "PGRST300",
            "Server lacks JWT secret",
            None,
            None,
        )));
    };
    match jwt.verify(token) {
        Ok(claims) => Ok(session_from_claims(claims.role, claims.raw)),
        Err(error) => Err(Box::new(jwt_failure(&error))),
    }
}

fn anon_session() -> Session {
    Session {
        role: "anon".to_string(),
        claims: r#"{"role":"anon"}"#.to_string(),
        anon: true,
    }
}

fn session_from_claims(role: Option<String>, mut raw: Map<String, Value>) -> Session {
    let role = role
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "anon".to_string());
    let anon = role == "anon";
    raw.insert("role".to_string(), Value::String(role.clone()));
    Session {
        role,
        claims: Value::Object(raw).to_string(),
        anon,
    }
}

fn jwt_failure(error: &JwtError) -> Response {
    let (status, code) = match error {
        JwtError::SecretMissing | JwtError::SecretTooShort { .. } => {
            (StatusCode::INTERNAL_SERVER_ERROR, "PGRST300")
        }
        JwtError::MalformedPayload | JwtError::ExpNotNumber | JwtError::Expired => {
            (StatusCode::UNAUTHORIZED, "PGRST303")
        }
        _ => (StatusCode::UNAUTHORIZED, "PGRST301"),
    };
    let mut response = pgrst(status, code, &error.to_string(), None, None);
    if status == StatusCode::UNAUTHORIZED {
        let value = format!(
            "Bearer error=\"invalid_token\", error_description=\"{}\"",
            error
        );
        if let Ok(header_value) = axum::http::HeaderValue::from_str(&value) {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, header_value);
        }
    }
    response
}

/// How a GET or HEAD read runs and what it returns.
#[derive(Clone, Copy)]
struct ReadMode {
    aggregates: bool,
    headers_only: bool,
    shape: Option<ReadShape>,
}

/// Response shape negotiated from `Accept` on GET and HEAD reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadShape {
    Object,
    ObjectStripped,
    ArrayStripped,
}

/// The preferred `Accept` entry: its lowercase essence and whether it carries
/// `nulls=stripped`. Entries rank by `q` (default 1, `q=0` dropped); ties go to
/// the more specific entry (no wildcard, `nulls=stripped`), then header order.
fn preferred_accept(value: Option<&axum::http::HeaderValue>) -> Option<(String, bool)> {
    let mut best: Option<(f32, (bool, bool), String, bool)> = None;
    for entry in header_str(value)?.split(',') {
        let mut pieces = entry.split(';');
        let essence = pieces.next().unwrap_or("").trim().to_ascii_lowercase();
        if essence.is_empty() {
            continue;
        }
        let mut stripped = false;
        let mut quality = 1.0_f32;
        for param in pieces {
            let param = param.trim();
            if param.eq_ignore_ascii_case("nulls=stripped") {
                stripped = true;
            } else if let Some((name, number)) = param.split_once('=') {
                if name.trim().eq_ignore_ascii_case("q") {
                    quality = number.trim().parse().unwrap_or(1.0);
                }
            }
        }
        if quality <= 0.0 {
            continue;
        }
        // Concrete beats wildcard first, then `nulls=stripped`.
        let specificity = (!essence.contains('*'), stripped);
        let better = best.as_ref().is_none_or(|(top, top_specificity, _, _)| {
            quality > *top || (quality == *top && specificity > *top_specificity)
        });
        if better {
            best = Some((quality, specificity, essence, stripped));
        }
    }
    best.map(|(_, _, essence, stripped)| (essence, stripped))
}

/// The 406 `PGRST116` error for a singular read that matched `rows` rows.
fn singular_check(rows: i64) -> Result<(), Box<Response>> {
    if rows == 1 {
        return Ok(());
    }
    Err(Box::new(pgrst(
        StatusCode::NOT_ACCEPTABLE,
        "PGRST116",
        "Cannot coerce the result to a single JSON object",
        Some(&format!("The result contains {rows} rows")),
        None,
    )))
}

impl ReadShape {
    /// The preferred `Accept` entry, when it names one of the shaped JSON types.
    pub(crate) fn from_accept(value: Option<&axum::http::HeaderValue>) -> Option<Self> {
        let (essence, stripped) = preferred_accept(value)?;
        match (essence.as_str(), stripped) {
            // megabase:unit rest:media-type:application/vnd.pgrst.object+json
            ("application/vnd.pgrst.object+json" | "application/vnd.pgrst.object", false) => {
                Some(Self::Object)
            }
            // megabase:unit rest:media-type:application/vnd.pgrst.object+json;nulls=stripped
            ("application/vnd.pgrst.object+json" | "application/vnd.pgrst.object", true) => {
                Some(Self::ObjectStripped)
            }
            // megabase:unit rest:media-type:application/vnd.pgrst.array+json;nulls=stripped
            ("application/vnd.pgrst.array+json" | "application/vnd.pgrst.array", true) => {
                Some(Self::ArrayStripped)
            }
            _ => None,
        }
    }

    fn singular(self) -> bool {
        matches!(self, Self::Object | Self::ObjectStripped)
    }

    fn stripped(self) -> bool {
        !matches!(self, Self::Object)
    }

    /// `MediaType.hs` `toContentType`: the mime plus `; charset=utf-8`.
    fn content_type(self) -> &'static str {
        match self {
            Self::Object => "application/vnd.pgrst.object+json; charset=utf-8",
            Self::ObjectStripped => {
                "application/vnd.pgrst.object+json;nulls=stripped; charset=utf-8"
            }
            Self::ArrayStripped => "application/vnd.pgrst.array+json;nulls=stripped; charset=utf-8",
        }
    }
}

/// `Err` is a coverage unit id. An empty string means the concrete path.
pub(crate) fn reject_accept(value: Option<&axum::http::HeaderValue>) -> Option<&'static str> {
    let (essence, nulls_stripped) = preferred_accept(value)?;
    match essence.as_str() {
        // megabase:unit rest:media-type:*/*
        // megabase:unit rest:media-type:application/json
        "*/*" | "application/json" | "application/*" => None,
        "application/vnd.pgrst.object+json" | "application/vnd.pgrst.object" if nulls_stripped => {
            Some("rest:media-type:application/vnd.pgrst.object+json;nulls=stripped")
        }
        "application/vnd.pgrst.object+json" | "application/vnd.pgrst.object" => {
            Some("rest:media-type:application/vnd.pgrst.object+json")
        }
        "application/vnd.pgrst.array+json" | "application/vnd.pgrst.array" if nulls_stripped => {
            Some("rest:media-type:application/vnd.pgrst.array+json;nulls=stripped")
        }
        "application/geo+json" => Some("rest:media-type:application/geo+json"),
        "application/octet-stream" => Some("rest:media-type:application/octet-stream"),
        "application/openapi+json" => Some("rest:media-type:application/openapi+json"),
        "application/x-www-form-urlencoded" => {
            Some("rest:media-type:application/x-www-form-urlencoded")
        }
        "text/csv" => Some("rest:media-type:text/csv"),
        "text/plain" => Some("rest:media-type:text/plain"),
        "text/xml" => Some("rest:media-type:text/xml"),
        other if other.starts_with("application/vnd.pgrst.plan+") => {
            Some("rest:media-type:application/vnd.pgrst.plan+{format}")
        }
        _ => Some(""),
    }
}

pub(crate) fn reject_prefer(value: Option<&axum::http::HeaderValue>) -> Option<&'static str> {
    let value = header_str(value)?;
    if value.trim().is_empty() {
        return None;
    }
    for part in value.split(',') {
        let token = part.trim();
        if token.is_empty() {
            continue;
        }
        let lower = token.to_ascii_lowercase();
        let unit = match lower.as_str() {
            "count=exact" => "rest:prefer:count=exact",
            "count=estimated" => "rest:prefer:count=estimated",
            "count=planned" => "rest:prefer:count=planned",
            "handling=lenient" => "rest:prefer:handling=lenient",
            "handling=strict" => "rest:prefer:handling=strict",
            "missing=default" => "rest:prefer:missing=default",
            "missing=null" => "rest:prefer:missing=null",
            "resolution=ignore-duplicates" => "rest:prefer:resolution=ignore-duplicates",
            "resolution=merge-duplicates" => "rest:prefer:resolution=merge-duplicates",
            "return=headers-only" => "rest:prefer:return=headers-only",
            "return=minimal" => "rest:prefer:return=minimal",
            "return=representation" => "rest:prefer:return=representation",
            "tx=commit" => "rest:prefer:tx=commit",
            "tx=rollback" => "rest:prefer:tx=rollback",
            _ if lower.starts_with("max-affected=") => "rest:prefer:max-affected=*",
            _ if lower.starts_with("timezone=") => "rest:prefer:timezone=*",
            _ => "",
        };
        return Some(unit);
    }
    Some("")
}

fn header_str(value: Option<&axum::http::HeaderValue>) -> Option<&str> {
    value?.to_str().ok()
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)?
        .to_str()
        .ok()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

pub(crate) fn unimplemented_unit(method: &Method, path: &str, unit: &str) -> Response {
    if unit.is_empty() {
        not_implemented(method, path)
    } else {
        MegabaseNotImplemented::new(crate::COMPONENT, unit).into_response()
    }
}

pub(crate) fn not_implemented(method: &Method, path: &str) -> Response {
    MegabaseNotImplemented::new(crate::COMPONENT, format!("{method} {path}")).into_response()
}

#[allow(clippy::too_many_arguments)]
async fn read_relation(
    pool: &sqlx::PgPool,
    schema: &str,
    relation: &str,
    session: &Session,
    read: &crate::params::ReadQuery,
    mode: ReadMode,
    prefer: &Prefer,
) -> Response {
    // The count of an aggregated read is not the filtered row count.
    if let (Some(count), true) = (prefer.count, read.has_aggregate()) {
        let unit = match count {
            Count::Exact => "rest:prefer:count=exact",
            Count::Planned => "rest:prefer:count=planned",
            Count::Estimated => "rest:prefer:count=estimated",
        };
        return MegabaseNotImplemented::new(crate::COMPONENT, unit).into_response();
    }
    match read_rows(pool, schema, relation, session, read, mode, prefer.count).await {
        Ok((count, body, offset, total)) => {
            let mut response = match total {
                Some(total) if offset > total => out_of_bounds(offset, total),
                _ => json_rows(
                    offset,
                    count,
                    total,
                    if mode.headers_only { "" } else { &body },
                    mode.shape.map_or(JSON_UTF8, ReadShape::content_type),
                ),
            };
            set_applied(
                &mut response,
                // `prefAppliedHeader` echoes every parsed preference.
                prefer.applied(Scope {
                    resolution: true,
                    missing: true,
                    count: true,
                    max_affected: true,
                }),
            );
            profile_header(&mut response, schema);
            response
        }
        Err(error) => *error,
    }
}

/// `416` with `PGRST103` when the offset is past the counted total.
fn out_of_bounds(offset: i64, total: i64) -> Response {
    let mut response = pgrst(
        StatusCode::RANGE_NOT_SATISFIABLE,
        "PGRST103",
        "Requested range not satisfiable",
        Some(&format!(
            "An offset of {offset} was requested, but there are only {total} rows."
        )),
        None,
    );
    if let Ok(value) = HeaderValue::from_str(&content_range_total(offset, 0, Some(total))) {
        response.headers_mut().insert(header::CONTENT_RANGE, value);
    }
    response
}

/// The reference stack's `PGRST_DB_MAX_ROWS` (Supabase `.env.example`).
const DB_MAX_ROWS: i64 = 1000;

// megabase:unit rest:prefer:count=exact
// megabase:unit rest:prefer:count=planned
// megabase:unit rest:prefer:count=estimated
/// Total rows behind the filters. `exact` counts. `estimated` counts up to
/// `DB_MAX_ROWS`, then reads the planner's row estimate like `planned`
/// (`Statements.hs` `mainRead`).
async fn count_total(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    schema: &str,
    relation: &str,
    read: &crate::params::ReadQuery,
    column_types: &[(&str, &str)],
    mode: Count,
    anon: bool,
) -> Result<i64, Box<Response>> {
    let filter = where_clause(relation, read, column_types)
        .map_err(|error| Box::new(unsafe_type(&error)))?;
    let from = format!("{}.{}", quote_ident(schema), quote_ident(relation));
    let (where_sql, params) = match filter {
        Some(filter) => (format!(" WHERE {}", filter.sql), filter.params),
        None => (String::new(), Vec::new()),
    };
    let mut planned = mode == Count::Planned;
    if mode == Count::Estimated {
        // `Statements.hs`: count up to `db-max-rows + 1`, then trust the planner.
        let sql = format!(
            "SELECT pg_catalog.count(*)::bigint FROM (SELECT 1 FROM {from}{where_sql} LIMIT {}) AS _megabase_count",
            DB_MAX_ROWS + 1
        );
        let mut query = sqlx::query_scalar::<sqlx::Postgres, i64>(&sql);
        for param in &params {
            query = query.bind(param);
        }
        let limited = query
            .fetch_one(&mut **tx)
            .await
            .map_err(|error| Box::new(query_failure(&error, anon)))?;
        if limited <= DB_MAX_ROWS {
            return Ok(limited);
        }
        planned = true;
    }
    if planned {
        let sql = format!("EXPLAIN (FORMAT JSON) SELECT 1 FROM {from}{where_sql}");
        let mut query = sqlx::query(&sql);
        for param in &params {
            query = query.bind(param);
        }
        let row = query
            .fetch_one(&mut **tx)
            .await
            .map_err(|error| Box::new(query_failure(&error, anon)))?;
        let plan: String = sqlx::Row::try_get_unchecked(&row, 0)
            .map_err(|error| Box::new(query_failure(&error, anon)))?;
        let rows = serde_json::from_str::<Value>(&plan)
            .ok()
            .and_then(|plan| plan[0]["Plan"]["Plan Rows"].as_i64());
        return rows.ok_or_else(|| {
            tracing::error!("rest planned count: no Plan Rows in EXPLAIN output");
            Box::new(pgrst(
                StatusCode::INTERNAL_SERVER_ERROR,
                "PGRST000",
                "Database connection error.",
                Some("could not read the planned row count"),
                None,
            ))
        });
    }
    let sql = format!("SELECT pg_catalog.count(*)::bigint FROM {from}{where_sql}");
    let mut query = sqlx::query_scalar::<sqlx::Postgres, i64>(&sql);
    for param in &params {
        query = query.bind(param);
    }
    query
        .fetch_one(&mut **tx)
        .await
        .map_err(|error| Box::new(query_failure(&error, anon)))
}

// megabase:unit rest:embed-join:left
// megabase:unit rest:embed-join:inner
#[allow(clippy::too_many_arguments)]
async fn read_rows(
    pool: &sqlx::PgPool,
    schema: &str,
    relation: &str,
    session: &Session,
    read: &crate::params::ReadQuery,
    mode: ReadMode,
    count: Option<Count>,
) -> Result<(i64, String, i64, Option<i64>), Box<Response>> {
    let ReadMode {
        aggregates,
        headers_only,
        shape,
    } = mode;
    let http_method = if headers_only { "HEAD" } else { "GET" };
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| Box::new(connect_failure(&error)))?;
    // PostgREST `WrappedReadPlan` uses `SQL.Read`: a read-only transaction.
    // Isolation stays the session default. Forcing `READ COMMITTED` would
    // ignore a role `default_transaction_isolation` that upstream honors.
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(|error| Box::new(query_failure(&error, session.anon)))?;
    let exists = relation_exists(&mut tx, schema, relation, session.anon).await?;
    if !exists {
        return Err(Box::new(table_not_found(schema, relation)));
    }
    // `validateAggFunctions` in `Plan.hs` runs after the table lookup.
    if read.has_aggregate() && !aggregates {
        return Err(Box::new(pgrst(
            StatusCode::BAD_REQUEST,
            "PGRST123",
            "Use of aggregate functions is not allowed",
            None,
            None,
        )));
    }
    // Aggregates next to an embed need `GROUP BY` over the embed; not served.
    if read.has_aggregate() && !read.embed_names().is_empty() {
        return Err(Box::new(
            MegabaseNotImplemented::new(crate::COMPONENT, format!("{http_method} /{relation}"))
                .into_response(),
        ));
    }
    let columns = load_columns(&mut tx, schema, relation, session.anon).await?;
    let column_types: Vec<(&str, &str)> = columns
        .iter()
        .map(|(name, pg_type)| (name.as_str(), pg_type.as_str()))
        .collect();
    let mut rels = HashMap::new();
    for name in read.embed_names() {
        if rels.contains_key(name) {
            continue;
        }
        let rel = load_relationship(&mut tx, schema, relation, name, session.anon).await?;
        rels.insert(name.to_string(), rel);
    }
    let built = build_read_sql(schema, relation, read, &column_types, &rels)
        .map_err(|error| Box::new(unsafe_type(&error)))?;
    let request_path = format!("/{relation}");
    set_request_context(&mut tx, session, http_method, &request_path, schema).await?;
    let mut query = sqlx::query_as::<sqlx::Postgres, (i64, String)>(&built.sql);
    for param in &built.params {
        query = query.bind(param);
    }
    let mut row = query
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| Box::new(query_failure(&error, session.anon)))?;
    let total = match count {
        Some(count_mode) => Some(
            count_total(
                &mut tx,
                schema,
                relation,
                read,
                &column_types,
                count_mode,
                session.anon,
            )
            .await?,
        ),
        None => None,
    };
    if let Some(shape) = shape {
        if shape.singular() {
            singular_check(row.0)?;
        }
        // Shaping stays in Postgres so key order and number text are kept.
        let sql = match (shape.singular(), shape.stripped()) {
            (true, true) => "SELECT json_strip_nulls(($1::json)->0)::text",
            (true, false) => "SELECT (($1::json)->0)::text",
            (false, _) => "SELECT json_strip_nulls($1::json)::text",
        };
        row.1 = sqlx::query_scalar(sql)
            .bind(&row.1)
            .fetch_one(&mut *tx)
            .await
            .map_err(|error| Box::new(query_failure(&error, session.anon)))?;
    }
    tx.commit()
        .await
        .map_err(|error| Box::new(query_failure(&error, session.anon)))?;
    Ok((row.0, row.1, built.offset, total))
}

/// Foreign keys between two tables of one schema, in either direction.
const RELATIONSHIPS_SQL: &str = "SELECT (c.relname = $2) AS from_root,
         ARRAY(SELECT a.attname::text FROM unnest(con.conkey) WITH ORDINALITY AS k(num, ord)
               JOIN pg_attribute AS a ON a.attrelid = con.conrelid AND a.attnum = k.num
               ORDER BY k.ord),
         ARRAY(SELECT a.attname::text FROM unnest(con.confkey) WITH ORDINALITY AS k(num, ord)
               JOIN pg_attribute AS a ON a.attrelid = con.confrelid AND a.attnum = k.num
               ORDER BY k.ord)
         FROM pg_constraint AS con
         JOIN pg_class AS c ON c.oid = con.conrelid
         JOIN pg_namespace AS n ON n.oid = c.relnamespace
         JOIN pg_class AS f ON f.oid = con.confrelid
         JOIN pg_namespace AS fnn ON fnn.oid = f.relnamespace
         WHERE con.contype = 'f'
           AND n.nspname = $1 AND fnn.nspname = $1
           AND ((c.relname = $2 AND f.relname = $3) OR (c.relname = $3 AND f.relname = $2))";

/// Find the one foreign key that links `relation` and the embedded `target`.
///
/// No match is `PGRST200`. A self-reference or several candidate keys stay 501.
async fn load_relationship(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    schema: &str,
    relation: &str,
    target: &str,
    anon: bool,
) -> Result<Relationship, Box<Response>> {
    // Views, materialized views and foreign tables carry no foreign keys here.
    let kinds: Vec<String> = sqlx::query_scalar(
        "SELECT c.relkind::text FROM pg_class AS c
         JOIN pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname = $1 AND c.relname = ANY($2)",
    )
    .bind(schema)
    .bind(vec![relation.to_string(), target.to_string()])
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| Box::new(query_failure(&error, anon)))?;
    if kinds.iter().any(|kind| kind != "r" && kind != "p") {
        return Err(Box::new(
            MegabaseNotImplemented::new(crate::COMPONENT, "rest:embed-join:left").into_response(),
        ));
    }
    let rows: Vec<(bool, Vec<String>, Vec<String>)> = sqlx::query_as(RELATIONSHIPS_SQL)
        .bind(schema)
        .bind(relation)
        .bind(target)
        .fetch_all(&mut **tx)
        .await
        .map_err(|error| Box::new(query_failure(&error, anon)))?;
    match rows.as_slice() {
        [] => Err(Box::new(pgrst(
            StatusCode::BAD_REQUEST,
            "PGRST200",
            &format!(
                "Could not find a relationship between '{relation}' and '{target}' in the schema cache"
            ),
            Some(&format!(
                "Searched for a foreign key relationship between '{relation}' and '{target}' in the schema '{schema}', but no matches were found."
            )),
            None,
        ))),
        [(from_root, key, referenced)] if relation != target => {
            let (root_columns, target_columns) = if *from_root {
                (key.clone(), referenced.clone())
            } else {
                (referenced.clone(), key.clone())
            };
            Ok(Relationship {
                to_many: !*from_root,
                root_columns,
                target_columns,
            })
        }
        _ => Err(Box::new(
            MegabaseNotImplemented::new(crate::COMPONENT, "rest:embed-join:left").into_response(),
        )),
    }
}

/// Column names and base types. `NULL` typmod matches an unknown literal:
/// `varchar(5)` and `numeric(10,2)` must not truncate or round the value.
const COLUMN_TYPES_SQL: &str = "SELECT a.attname::text, format_type(a.atttypid, NULL)
         FROM pg_attribute AS a
         JOIN pg_class AS c ON c.oid = a.attrelid
         JOIN pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname = $1
           AND c.relname = $2
           AND a.attnum > 0
           AND NOT a.attisdropped
         ORDER BY a.attnum";

/// Open a transaction. A connect failure is `PGRST000`.
pub(crate) async fn begin(
    pool: &sqlx::PgPool,
) -> Result<sqlx::Transaction<'_, sqlx::Postgres>, Box<Response>> {
    pool.begin()
        .await
        .map_err(|error| Box::new(connect_failure(&error)))
}

/// `SET LOCAL` role, JWT claims, method, path, and `search_path`.
pub(crate) async fn set_request_context(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    session: &Session,
    method: &str,
    request_path: &str,
    schema: &str,
) -> Result<(), Box<Response>> {
    sqlx::query(
        "SELECT set_config('role', $1, true),
                set_config('request.jwt.claims', $2, true),
                set_config('request.method', $3, true),
                set_config('request.path', $4, true),
                set_config('search_path', $5, true)",
    )
    .bind(&session.role)
    .bind(&session.claims)
    .bind(method)
    .bind(request_path)
    .bind(schema)
    .execute(&mut **tx)
    .await
    .map_err(|error| Box::new(query_failure(&error, session.anon)))?;
    Ok(())
}

/// `true` when `schema.relation` is a table, view, or foreign table in the cache.
pub(crate) async fn relation_exists(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    schema: &str,
    relation: &str,
    anon: bool,
) -> Result<bool, Box<Response>> {
    sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM pg_class AS c
            JOIN pg_namespace AS n ON n.oid = c.relnamespace
            WHERE n.nspname = $1
              AND c.relname = $2
              AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
              AND NOT c.relispartition
        )",
    )
    .bind(schema)
    .bind(relation)
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| Box::new(query_failure(&error, anon)))
}

/// Column names and `format_type(atttypid, NULL)` in `attnum` order.
pub(crate) async fn load_columns(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    schema: &str,
    relation: &str,
    anon: bool,
) -> Result<Vec<(String, String)>, Box<Response>> {
    sqlx::query_as(COLUMN_TYPES_SQL)
        .bind(schema)
        .bind(relation)
        .fetch_all(&mut **tx)
        .await
        .map_err(|error| Box::new(query_failure(&error, anon)))
}

pub(crate) fn table_not_found(schema: &str, relation: &str) -> Response {
    pgrst(
        StatusCode::NOT_FOUND,
        "PGRST205",
        &format!("Could not find the table '{schema}.{relation}' in the schema cache"),
        None,
        None,
    )
}

pub(crate) fn profile_header(response: &mut Response, schema: &str) {
    if let Ok(value) = HeaderValue::from_str(schema) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("content-profile"), value);
    }
}

pub(crate) fn unsafe_type(error: &UnsafeType) -> Response {
    tracing::error!(error = %error, "rest filter cast rejected");
    pgrst(
        StatusCode::INTERNAL_SERVER_ERROR,
        "PGRST000",
        "Database connection error.",
        Some("column type cannot be used in a filter"),
        None,
    )
}

fn connect_failure(error: &sqlx::Error) -> Response {
    tracing::error!(error = %sanitize(&error.to_string()), "rest filter transaction failed");
    pgrst(
        StatusCode::SERVICE_UNAVAILABLE,
        "PGRST000",
        "Database connection error.",
        Some("could not query the database"),
        None,
    )
}

pub(crate) fn query_failure(error: &sqlx::Error, anon: bool) -> Response {
    if let Some(db) = error.as_database_error() {
        let code = db
            .code()
            .map(|value| value.into_owned())
            .unwrap_or_default();
        let message = db.message().to_string();
        let (details, hint) = db
            .try_downcast_ref::<sqlx::postgres::PgDatabaseError>()
            .map(|pg| {
                (
                    pg.detail().map(str::to_string),
                    pg.hint().map(str::to_string),
                )
            })
            .unwrap_or((None, None));
        let status = sqlstate_status(&code, &message, anon);
        let sqlstate = if code.is_empty() {
            "PGRST000"
        } else {
            code.as_str()
        };
        return pgrst(
            status,
            sqlstate,
            &message,
            details.as_deref(),
            hint.as_deref(),
        );
    }
    tracing::error!(error = %sanitize(&error.to_string()), "rest filter query failed");
    pgrst(
        StatusCode::SERVICE_UNAVAILABLE,
        "PGRST000",
        "Database connection error.",
        Some("could not query the database"),
        None,
    )
}

fn sqlstate_status(code: &str, message: &str, anon: bool) -> StatusCode {
    if code == "42501" {
        return if anon {
            StatusCode::UNAUTHORIZED
        } else {
            StatusCode::FORBIDDEN
        };
    }
    if code == "22023" && message.starts_with("role") && message.ends_with("does not exist") {
        return StatusCode::UNAUTHORIZED;
    }
    // `Error.hs` `mapSQLtoHTTP`: undefined_function is 404, except `xmlagg`.
    if code == "42883" {
        return if message.starts_with("function xmlagg(") {
            StatusCode::NOT_ACCEPTABLE
        } else {
            StatusCode::NOT_FOUND
        };
    }
    match code.as_bytes() {
        [b'0', b'8', ..] | [b'5', b'3', ..] => StatusCode::SERVICE_UNAVAILABLE,
        b"57P01" => StatusCode::SERVICE_UNAVAILABLE,
        b"23503" | b"23505" => StatusCode::CONFLICT,
        b"42P01" => StatusCode::NOT_FOUND,
        [b'0', b'9', ..] | [b'5', b'4', ..] | [b'X', b'X', ..] => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    }
}

fn sanitize(text: &str) -> String {
    if text.contains("://") || text.to_ascii_lowercase().contains("password") {
        "connection failed".to_string()
    } else {
        text.to_string()
    }
}

fn json_rows(
    offset: i64,
    count: i64,
    total: Option<i64>,
    body: &str,
    content_type: &'static str,
) -> Response {
    let range = content_range_total(offset, count, total);
    // `RangeQuery.rangeStatusHeader`: a page shorter than the total is `206`.
    let status = match total {
        Some(total) if count < total => StatusCode::PARTIAL_CONTENT,
        _ => StatusCode::OK,
    };
    (
        status,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CONTENT_RANGE, range.as_str()),
        ],
        body.to_string(),
    )
        .into_response()
}

/// `Content-Range` without `Prefer: count`. The total stays `*`.
///
/// `lower` is the offset. `upper` is `offset + count - 1`. An empty page,
/// or a lower bound past the upper bound, is `*/*` (`RangeQuery.contentRangeH`).
pub(crate) fn content_range(offset: i64, count: i64) -> String {
    content_range_total(offset, count, None)
}

/// `Content-Range` with an optional counted total. A zero total, an empty
/// page, or a lower bound past the upper bound prints `*` for the range.
pub(crate) fn content_range_total(offset: i64, count: i64, total: Option<i64>) -> String {
    let total_text = total.map_or_else(|| "*".to_string(), |total| total.to_string());
    if count <= 0 || total == Some(0) {
        return format!("*/{total_text}");
    }
    match offset.checked_add(count - 1) {
        Some(upper) if offset <= upper => format!("{offset}-{upper}/{total_text}"),
        _ => format!("*/{total_text}"),
    }
}

pub(crate) fn pgrst(
    status: StatusCode,
    code: &str,
    message: &str,
    details: Option<&str>,
    hint: Option<&str>,
) -> Response {
    let body = serde_json::json!({
        "code": code,
        "message": message,
        "details": details,
        "hint": hint,
    });
    (
        status,
        [(header::CONTENT_TYPE, JSON_UTF8)],
        body.to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use megabase_core::{Config, Hs256, JwtSecret};
    use serde_json::json;
    use tower::ServiceExt;

    const SECRET: &str = "jwt-secret-for-rest-filter-tests!!";

    fn state_with_secret(secret: &str) -> RestState {
        let config = Config {
            jwt_secret: Some(JwtSecret::new(secret)),
            ..Config::default()
        };
        RestState::from_config(&config)
    }

    fn sign(secret: &str, claims: &Value) -> String {
        Hs256::new(secret.as_bytes())
            .expect("test secret")
            .sign(claims)
            .expect("sign")
            .to_string()
    }

    async fn send(app: Router, request: Request<Body>) -> (StatusCode, Value, HeaderMap) {
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, body, headers)
    }

    fn get(path: &str) -> Request<Body> {
        Request::builder().uri(path).body(Body::empty()).unwrap()
    }

    #[test]
    fn relation_name_keeps_a_plus_in_the_path() {
        assert_eq!(relation_name("/rest/v1/a+b").as_deref(), Some("a+b"));
        assert_eq!(relation_name("/rest/v1/a%2Bb").as_deref(), Some("a+b"));
        assert_eq!(relation_name("/rest/v1/a%20b").as_deref(), Some("a b"));
    }

    #[test]
    fn column_types_drop_typmod() {
        assert!(COLUMN_TYPES_SQL.contains("format_type(a.atttypid, NULL)"));
        assert!(!COLUMN_TYPES_SQL.contains("atttypmod"));
    }

    #[test]
    fn content_range_uses_the_sql_count() {
        let response = json_rows(0, 2, None, "not json", JSON_UTF8);
        let range = response
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok());
        assert_eq!(range, Some("0-1/*"));
        let empty = json_rows(0, 0, None, "[]", JSON_UTF8);
        let range = empty
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok());
        assert_eq!(range, Some("*/*"));
        let shifted = json_rows(1, 1, None, "[{}]", JSON_UTF8);
        let range = shifted
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok());
        assert_eq!(range, Some("1-1/*"));
    }

    #[test]
    fn undefined_function_is_not_found_unless_xmlagg() {
        assert_eq!(
            sqlstate_status("42883", "operator does not exist: text @> unknown", true),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            sqlstate_status("42883", "function xmlagg(integer) does not exist", true),
            StatusCode::NOT_ACCEPTABLE
        );
    }

    #[tokio::test]
    async fn bare_get_reaches_the_pool() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, _) = send(app, get("/rest/v1/todos")).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "PGRST000");
        assert!(body["unit"].is_null());
    }

    #[tokio::test]
    async fn blank_query_reaches_the_pool() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, _) = send(app, get("/rest/v1/todos?&&&")).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "PGRST000");
    }

    #[tokio::test]
    async fn unknown_operator_is_pgrst100() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, headers) = send(app, get("/rest/v1/todos?id=0")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "PGRST100");
        assert_eq!(
            body["message"],
            "\"failed to parse filter (0)\" (line 1, column 1)"
        );
        assert_eq!(
            body["details"],
            "unexpected \"0\" expecting \"not\" or operator (eq, gt, ...)"
        );
        assert!(body["hint"].is_null());
        let content_type = headers.get(header::CONTENT_TYPE).unwrap().to_str().unwrap();
        assert!(content_type.contains("application/json"));
    }

    #[tokio::test]
    async fn range_and_fts_filters_reach_the_database() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, _) = send(
            app,
            get("/rest/v1/todos?priority=ov.[1,4)&title=plfts(english).spec&during=nxl.[4,7)"),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "PGRST000");
        assert!(body["unit"].is_null());
    }

    #[tokio::test]
    async fn query_params_without_pool_are_503() {
        let paths = [
            "/rest/v1/todos?select=id&done=eq.true",
            "/rest/v1/todos?order=id.desc",
            "/rest/v1/todos?limit=1&offset=1",
            "/rest/v1/todos?and=(done.eq.true,priority.gte.1)",
            "/rest/v1/todos?or=(id.eq.1,id.eq.3)",
            "/rest/v1/todos?columns=id,title",
            "/rest/v1/todos?on_conflict=id&select=id",
            "/rest/v1/todos?limit=",
        ];
        for path in paths {
            let app = router(RestState::from_config(&Config::default()));
            let (status, body, _) = send(app, get(path)).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
            assert_eq!(body["code"], "PGRST000", "{path}");
            assert!(body["unit"].is_null(), "{path}");
        }
    }

    #[tokio::test]
    async fn bad_order_is_pgrst100() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, _) = send(app, get("/rest/v1/todos?order=id.ac&id=nope")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "PGRST100");
        assert_eq!(
            body["message"],
            "\"failed to parse order (id.ac)\" (line 1, column 4)"
        );
    }

    #[tokio::test]
    async fn empty_columns_is_pgrst100() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, _) = send(app, get("/rest/v1/todos?columns=")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "PGRST100");
        assert!(body["message"]
            .as_str()
            .unwrap()
            .contains("failed to parse columns parameter"));
    }

    #[tokio::test]
    async fn embedded_order_is_pgrst108() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, _) = send(app, get("/rest/v1/todos?select=id&items.order=id")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "PGRST108");
        assert_eq!(
            body["message"],
            "'items' is not an embedded resource in this request"
        );
        assert!(body["details"].is_null());
        assert_eq!(
            body["hint"],
            "Verify that 'items' is included in the 'select' query parameter."
        );
    }

    #[tokio::test]
    async fn aggregate_select_without_a_database_is_503() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, _, _) = send(app, get("/rest/v1/todos?select=id.count()")).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn embed_select_is_route_501() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, _) = send(app, get("/rest/v1/todos?select=notes!fk(body)")).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "GET /rest/v1/todos");
    }

    #[tokio::test]
    async fn inner_embed_needs_a_database() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, _) = send(app, get("/rest/v1/todos?select=notes!inner(body)")).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "PGRST000");
    }

    #[tokio::test]
    async fn embed_select_with_bad_filter_is_pgrst100() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, _) = send(app, get("/rest/v1/todos?select=notes(body)&id=nope")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "PGRST100");
    }

    #[tokio::test]
    async fn dotted_filter_stays_501() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, _) = send(app, get("/rest/v1/todos?a.b=eq.1")).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "GET /rest/v1/todos");
    }

    #[tokio::test]
    async fn select_without_equals_stays_501() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, _) = send(app, get("/rest/v1/todos?select")).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "GET /rest/v1/todos");
    }

    #[tokio::test]
    async fn bad_bearer_is_401_before_query_parse() {
        let app = router(state_with_secret(SECRET));
        let request = Request::builder()
            .uri("/rest/v1/todos?order=id.ac")
            .header(header::AUTHORIZATION, "Bearer not-a-jwt")
            .body(Body::empty())
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["code"], "PGRST301");
    }

    #[test]
    fn read_shape_decodes_accept() {
        let hv = |text: &'static str| Some(HeaderValue::from_static(text));
        let shape = |text: &'static str| ReadShape::from_accept(hv(text).as_ref());
        assert_eq!(
            shape("application/vnd.pgrst.object+json"),
            Some(ReadShape::Object)
        );
        assert_eq!(
            shape("application/vnd.pgrst.object+json;nulls=stripped"),
            Some(ReadShape::ObjectStripped)
        );
        assert_eq!(
            shape("application/vnd.pgrst.array+json;nulls=stripped"),
            Some(ReadShape::ArrayStripped)
        );
        assert_eq!(shape("application/json"), None);
        assert_eq!(shape("application/vnd.pgrst.array+json"), None);
        assert_eq!(ReadShape::from_accept(None), None);
        assert_eq!(
            shape("application/vnd.pgrst.object"),
            Some(ReadShape::Object)
        );
    }

    #[test]
    fn read_shape_ranks_accept_by_quality() {
        let hv = |text: &'static str| Some(HeaderValue::from_static(text));
        let shape = |text: &'static str| ReadShape::from_accept(hv(text).as_ref());
        assert_eq!(
            shape("application/json;q=0.1, application/vnd.pgrst.object+json;q=0.9"),
            Some(ReadShape::Object)
        );
        assert_eq!(
            shape("application/vnd.pgrst.object+json;q=0.1, application/json"),
            None
        );
        assert_eq!(
            shape("application/vnd.pgrst.object+json;q=0, application/json"),
            None
        );
    }

    #[test]
    fn read_shape_prefers_specific_entry_on_equal_quality() {
        let hv = |text: &'static str| Some(HeaderValue::from_static(text));
        let shape = |text: &'static str| ReadShape::from_accept(hv(text).as_ref());
        assert_eq!(
            shape("*/*, application/vnd.pgrst.object+json"),
            Some(ReadShape::Object)
        );
        assert_eq!(
            shape("application/vnd.pgrst.object+json, application/vnd.pgrst.object+json;nulls=stripped"),
            Some(ReadShape::ObjectStripped)
        );
        // A concrete type outranks a stripped wildcard.
        assert_eq!(
            shape("*/*;nulls=stripped, application/vnd.pgrst.object+json"),
            Some(ReadShape::Object)
        );
    }

    #[test]
    fn singular_check_requires_exactly_one_row() {
        assert!(singular_check(1).is_ok());
        for rows in [0, 2] {
            let response = singular_check(rows).unwrap_err();
            assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
        }
    }

    #[test]
    fn shaped_content_types() {
        assert_eq!(
            ReadShape::Object.content_type(),
            "application/vnd.pgrst.object+json; charset=utf-8"
        );
        assert_eq!(
            ReadShape::ArrayStripped.content_type(),
            "application/vnd.pgrst.array+json;nulls=stripped; charset=utf-8"
        );
    }

    #[tokio::test]
    async fn geo_json_accept_is_501() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .uri("/rest/v1/todos")
            .header(header::ACCEPT, "application/geo+json")
            .body(Body::empty())
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "rest:media-type:application/geo+json");
    }

    #[tokio::test]
    async fn prefer_return_is_501() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .uri("/rest/v1/todos?done=eq.true")
            .header("prefer", "count=exact, return=representation")
            .body(Body::empty())
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "rest:prefer:return=representation");
    }

    #[tokio::test]
    async fn prefer_count_reaches_the_pool() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .uri("/rest/v1/todos")
            .header("prefer", "count=planned")
            .body(Body::empty())
            .unwrap();
        let (status, _, _) = send(app, request).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn strict_handling_rejects_unknown_preferences() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .uri("/rest/v1/todos")
            .header("prefer", "handling=strict, wat")
            .body(Body::empty())
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "PGRST122");
        assert_eq!(body["details"], "Invalid preferences: wat");
    }

    #[tokio::test]
    async fn lenient_handling_ignores_unknown_preferences() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .uri("/rest/v1/todos")
            .header("prefer", "handling=lenient, wat")
            .body(Body::empty())
            .unwrap();
        let (status, _, _) = send(app, request).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn content_range_carries_the_counted_total() {
        assert_eq!(content_range_total(0, 2, Some(5)), "0-1/5");
        assert_eq!(content_range_total(0, 0, Some(0)), "*/0");
        assert_eq!(content_range_total(5, 0, Some(3)), "*/3");
        assert_eq!(
            json_rows(0, 2, Some(5), "[]", JSON_UTF8).status(),
            StatusCode::PARTIAL_CONTENT
        );
        assert_eq!(
            json_rows(0, 5, Some(5), "[]", JSON_UTF8).status(),
            StatusCode::OK
        );
        assert_eq!(
            out_of_bounds(9, 3).status(),
            StatusCode::RANGE_NOT_SATISFIABLE
        );
    }

    #[tokio::test]
    async fn served_filter_without_pool_is_503_not_a_row() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, _) = send(app, get("/rest/v1/todos?done=eq.true")).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "PGRST000");
        assert!(body["message"].as_str().unwrap().contains("Database"));
        assert!(body.as_array().is_none());
    }

    #[tokio::test]
    async fn bad_bearer_is_401_before_the_database() {
        let app = router(state_with_secret(SECRET));
        let request = Request::builder()
            .uri("/rest/v1/todos?done=eq.true")
            .header(header::AUTHORIZATION, "Bearer not-a-jwt")
            .body(Body::empty())
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["code"], "PGRST301");
    }

    #[tokio::test]
    async fn signed_anon_token_reaches_the_pool_check() {
        let app = router(state_with_secret(SECRET));
        let token = sign(SECRET, &json!({"role": "anon", "exp": 4_000_000_000_i64}));
        let request = Request::builder()
            .uri("/rest/v1/todos?title=ilike.*spec*&priority=gte.1")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "PGRST000");
    }

    #[tokio::test]
    async fn empty_post_is_pgrst102() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .method("POST")
            .uri("/rest/v1/todos?id=eq.1")
            .body(Body::empty())
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "PGRST102");
        assert_eq!(body["message"], "Empty or invalid json");
    }

    #[tokio::test]
    async fn root_options_lists_read_methods() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .method("OPTIONS")
            .uri("/rest/v1/")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::ALLOW)
                .and_then(|value| value.to_str().ok()),
            Some("OPTIONS,GET,HEAD")
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(bytes.is_empty());
    }

    #[tokio::test]
    async fn root_post_is_pgrst117() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .method("POST")
            .uri("/rest/v1/")
            .body(Body::from("{}"))
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(body["code"], "PGRST117");
    }

    #[tokio::test]
    async fn root_get_without_pool_is_503() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, _) = send(app, get("/rest/v1/")).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "PGRST000");
    }

    #[tokio::test]
    async fn put_without_filters_is_pgrst105() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .method("PUT")
            .uri("/rest/v1/todos")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"id":1}"#))
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(body["code"], "PGRST105");
    }

    #[tokio::test]
    async fn write_object_accept_is_501() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .method("POST")
            .uri("/rest/v1/todos")
            .header(header::ACCEPT, "application/vnd.pgrst.object+json")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"id":1}"#))
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(
            body["unit"],
            "rest:media-type:application/vnd.pgrst.object+json"
        );
    }

    #[tokio::test]
    async fn head_object_accept_is_not_501() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .method("HEAD")
            .uri("/rest/v1/todos")
            .header(header::ACCEPT, "application/vnd.pgrst.object+json")
            .body(Body::empty())
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "PGRST000");
    }

    #[tokio::test]
    async fn patch_limit_stays_501() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .method("PATCH")
            .uri("/rest/v1/todos?limit=1")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["code"], "MEGABASE_NOT_IMPLEMENTED");
        assert_eq!(body["unit"], "PATCH /rest/v1/todos");
    }

    #[tokio::test]
    async fn delete_offset_stays_501() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .method("DELETE")
            .uri("/rest/v1/todos?id=eq.1&offset=2")
            .body(Body::empty())
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "DELETE /rest/v1/todos");
    }

    #[tokio::test]
    async fn put_limit_is_pgrst114() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .method("PUT")
            .uri("/rest/v1/todos?id=eq.1&limit=1")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "PGRST114");
    }

    #[tokio::test]
    async fn mismatched_json_keys_are_pgrst102() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .method("POST")
            .uri("/rest/v1/todos")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"[{"id":1},{"title":"a"}]"#))
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["message"], "All object keys must match");
    }

    #[tokio::test]
    async fn unknown_content_type_is_pgrst102() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .method("POST")
            .uri("/rest/v1/todos")
            .header(header::CONTENT_TYPE, "application/x-custom")
            .body(Body::from("{}"))
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "PGRST102");
        assert_eq!(
            body["message"],
            "Content-Type not acceptable: application/x-custom"
        );
    }

    #[tokio::test]
    async fn deep_path_is_pgrst125() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, _) = send(app, get("/rest/v1/todos/extra")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["code"], "PGRST125");
    }

    #[tokio::test]
    async fn hidden_schema_is_pgrst106() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .uri("/rest/v1/todos")
            .header("accept-profile", "secret")
            .body(Body::empty())
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::NOT_ACCEPTABLE);
        assert_eq!(body["code"], "PGRST106");
        assert_eq!(
            body["hint"],
            "Only the following schemas are exposed: public, graphql_public"
        );
    }

    #[test]
    fn rpc_name_is_one_decoded_segment() {
        assert_eq!(
            rpc_name("/rest/v1/rpc/add_numbers").as_deref(),
            Some("add_numbers")
        );
        assert_eq!(rpc_name("/rest/v1/rpc/a%2Bb").as_deref(), Some("a+b"));
        assert_eq!(rpc_name("/rest/v1/rpc"), None);
        assert_eq!(rpc_name("/rest/v1/rpc/a/b"), None);
    }

    #[tokio::test]
    async fn delete_without_pool_is_503() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .method("DELETE")
            .uri("/rest/v1/todos?id=eq.1")
            .body(Body::empty())
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "PGRST000");
    }
}
