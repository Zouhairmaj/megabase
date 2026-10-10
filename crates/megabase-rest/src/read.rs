//! `GET /rest/v1/{relation}` when every filter is a served operator.
//!
//! Anything else on the REST prefix stays
//! [`megabase_core::MegabaseNotImplemented`]. Values are bound parameters.
//! Relation and column names come from `pg_catalog` after a bound lookup,
//! then `quote_ident`.

use axum::extract::{OriginalUri, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use megabase_core::{bearer_token, JwtError, MegabaseNotImplemented};
use serde_json::{Map, Value};

use crate::filter::{
    parse_filter_value, predicate_sql, quote_ident, BoundFilter, ParsedFilter, UnsafeType,
};
use crate::query::{classify_query, percent_decode};
use crate::RestState;

const JSON_UTF8: &str = "application/json; charset=utf-8";

pub(crate) fn router(state: RestState) -> Router {
    Router::new().fallback(dispatch).with_state(state)
}

async fn dispatch(
    State(state): State<RestState>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let path = uri.path();
    let Some(relation) = relation_name(path) else {
        return not_implemented(&method, path);
    };
    if method != Method::GET {
        return not_implemented(&method, path);
    }
    let session = match session(&state, headers.get(header::AUTHORIZATION)) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let classified = classify_query(uri.query().unwrap_or(""));
    if classified.filters.is_empty() && classified.reserved.is_none() && !classified.embed {
        return not_implemented(&method, path);
    }
    let mut served = Vec::new();
    for filter in &classified.filters {
        match parse_filter_value(&filter.value) {
            Ok(ParsedFilter::Served {
                op,
                quant,
                language,
                value,
            }) => served.push((filter.column.as_str(), op, quant, language, value)),
            Ok(ParsedFilter::Unsupported { unit }) => {
                return MegabaseNotImplemented::new(crate::COMPONENT, unit).into_response();
            }
            Err(error) => {
                return pgrst(
                    StatusCode::BAD_REQUEST,
                    "PGRST100",
                    &error.message,
                    Some(&error.details),
                    None,
                );
            }
        }
    }
    if let Some(unit) = classified.reserved {
        return MegabaseNotImplemented::new(crate::COMPONENT, unit).into_response();
    }
    if classified.embed {
        return not_implemented(&method, path);
    }
    if let Some(unit) = reject_accept(headers.get(header::ACCEPT)) {
        return unimplemented_unit(&method, path, unit);
    }
    if let Some(unit) = reject_prefer(headers.get("prefer")) {
        return unimplemented_unit(&method, path, unit);
    }
    if headers.get(header::RANGE).is_some() {
        return not_implemented(&method, path);
    }
    if let Some(profile) = header_text(&headers, "accept-profile") {
        if profile != "public" {
            return not_implemented(&method, path);
        }
    }
    let Some(pool) = state.pool.as_ref() else {
        return pgrst(
            StatusCode::SERVICE_UNAVAILABLE,
            "PGRST000",
            "Database connection error.",
            Some("DATABASE_URL is unset"),
            None,
        );
    };
    match read_rows(pool, &relation, &session, &served).await {
        Ok(body) => json_rows(&body),
        Err(error) => *error,
    }
}

fn relation_name(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/rest/v1/")?;
    if rest.is_empty() || rest.contains('/') {
        return None;
    }
    let name = percent_decode(rest);
    if name.is_empty() || name.contains('/') || name.contains('\0') {
        return None;
    }
    Some(name)
}

struct Session {
    role: String,
    claims: String,
    /// `true` when the role is the anon role. PostgREST uses that to choose
    /// 401 versus 403 for `42501`.
    anon: bool,
}

fn session(
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

/// `Err` is a coverage unit id. An empty string means the concrete path.
fn reject_accept(value: Option<&axum::http::HeaderValue>) -> Option<&'static str> {
    let value = header_str(value)?;
    let first = value.split(',').next()?.trim();
    if first.is_empty() {
        return None;
    }
    let mut pieces = first.split(';');
    let essence = pieces.next()?.trim().to_ascii_lowercase();
    let nulls_stripped = pieces.any(|param| param.trim().eq_ignore_ascii_case("nulls=stripped"));
    match essence.as_str() {
        "*/*" | "application/json" | "application/*" => None,
        "application/vnd.pgrst.object+json" if nulls_stripped => {
            Some("rest:media-type:application/vnd.pgrst.object+json;nulls=stripped")
        }
        "application/vnd.pgrst.object+json" => {
            Some("rest:media-type:application/vnd.pgrst.object+json")
        }
        "application/vnd.pgrst.array+json" if nulls_stripped => {
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

fn reject_prefer(value: Option<&axum::http::HeaderValue>) -> Option<&'static str> {
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

fn unimplemented_unit(method: &Method, path: &str, unit: &str) -> Response {
    if unit.is_empty() {
        not_implemented(method, path)
    } else {
        MegabaseNotImplemented::new(crate::COMPONENT, unit).into_response()
    }
}

fn not_implemented(method: &Method, path: &str) -> Response {
    MegabaseNotImplemented::new(crate::COMPONENT, format!("{method} {path}")).into_response()
}

type Served<'a> = (
    &'a str,
    crate::filter::ServedOp,
    Option<crate::filter::Quant>,
    Option<String>,
    String,
);

async fn read_rows(
    pool: &sqlx::PgPool,
    relation: &str,
    session: &Session,
    filters: &[Served<'_>],
) -> Result<String, Box<Response>> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| Box::new(connect_failure(&error)))?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM pg_class AS c
            JOIN pg_namespace AS n ON n.oid = c.relnamespace
            WHERE n.nspname = 'public'
              AND c.relname = $1
              AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
        )",
    )
    .bind(relation)
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| Box::new(query_failure(&error, session.anon)))?;
    if !exists {
        return Err(Box::new(pgrst(
            StatusCode::NOT_FOUND,
            "PGRST205",
            &format!("Could not find the table 'public.{relation}' in the schema cache"),
            None,
            None,
        )));
    }
    let columns: Vec<(String, String)> = sqlx::query_as(
        "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod)
         FROM pg_attribute AS a
         JOIN pg_class AS c ON c.oid = a.attrelid
         JOIN pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public'
           AND c.relname = $1
           AND a.attnum > 0
           AND NOT a.attisdropped
         ORDER BY a.attnum",
    )
    .bind(relation)
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| Box::new(query_failure(&error, session.anon)))?;
    let mut bound = Vec::with_capacity(filters.len());
    for filter in filters {
        let pg_type = columns
            .iter()
            .find(|(name, _)| name.as_str() == filter.0)
            .map(|(_, pg_type)| pg_type.clone());
        bound.push(BoundFilter {
            column: filter.0.to_string(),
            op: filter.1,
            quant: filter.2,
            language: filter.3.clone(),
            value: filter.4.clone(),
            pg_type,
        });
    }
    let predicate =
        predicate_sql(relation, &bound).map_err(|error| Box::new(unsafe_type(&error)))?;
    let request_path = format!("/{relation}");
    sqlx::query(
        "SELECT set_config('role', $1, true),
                set_config('request.jwt.claims', $2, true),
                set_config('request.method', $3, true),
                set_config('request.path', $4, true),
                set_config('search_path', $5, true)",
    )
    .bind(&session.role)
    .bind(&session.claims)
    .bind("GET")
    .bind(&request_path)
    .bind("public")
    .execute(&mut *tx)
    .await
    .map_err(|error| Box::new(query_failure(&error, session.anon)))?;
    let relation_ident = quote_ident(relation);
    let sql = format!(
        "SELECT coalesce(json_agg(_postgrest_t), '[]'::json)::text \
         FROM (SELECT {relation_ident}.* FROM {schema}.{relation_ident} \
         WHERE {predicate}) _postgrest_t",
        schema = quote_ident("public"),
        predicate = predicate.sql,
    );
    let mut query = sqlx::query_scalar::<sqlx::Postgres, String>(&sql);
    for param in &predicate.params {
        query = query.bind(param);
    }
    let body = query
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| Box::new(query_failure(&error, session.anon)))?;
    tx.commit()
        .await
        .map_err(|error| Box::new(query_failure(&error, session.anon)))?;
    Ok(body)
}

fn unsafe_type(error: &UnsafeType) -> Response {
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

fn query_failure(error: &sqlx::Error, anon: bool) -> Response {
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

fn json_rows(body: &str) -> Response {
    let len = serde_json::from_str::<Vec<Value>>(body)
        .map(|rows| rows.len())
        .unwrap_or(0);
    let range = if len == 0 {
        "*/*".to_string()
    } else {
        format!("0-{}/*", len - 1)
    };
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, JSON_UTF8),
            (header::CONTENT_RANGE, range.as_str()),
        ],
        body.to_string(),
    )
        .into_response()
}

fn pgrst(
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
    async fn bare_get_stays_501() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, _) = send(app, get("/rest/v1/todos")).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "GET /rest/v1/todos");
        assert_eq!(body["code"], "MEGABASE_NOT_IMPLEMENTED");
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
    async fn later_operator_is_501_and_does_not_query() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, _) = send(app, get("/rest/v1/todos?priority=gt.1&priority=lt.3")).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "rest:filter-operator:lt");
    }

    #[tokio::test]
    async fn reserved_param_is_501() {
        let app = router(RestState::from_config(&Config::default()));
        let (status, body, _) = send(app, get("/rest/v1/todos?select=id&done=eq.true")).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "rest:query-param:select");
    }

    #[tokio::test]
    async fn object_accept_is_501() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .uri("/rest/v1/todos?id=eq.1")
            .header(header::ACCEPT, "application/vnd.pgrst.object+json")
            .body(Body::empty())
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(
            body["unit"],
            "rest:media-type:application/vnd.pgrst.object+json"
        );
    }

    #[tokio::test]
    async fn prefer_count_is_501() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .uri("/rest/v1/todos?done=eq.true")
            .header("prefer", "count=exact")
            .body(Body::empty())
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "rest:prefer:count=exact");
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
    async fn other_methods_stay_501() {
        let app = router(RestState::from_config(&Config::default()));
        let request = Request::builder()
            .method("POST")
            .uri("/rest/v1/todos?id=eq.1")
            .body(Body::empty())
            .unwrap();
        let (status, body, _) = send(app, request).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "POST /rest/v1/todos");
    }
}
