// Ported from supabase/postgres-meta src/server/routes/policies.ts,
// src/lib/PostgresMetaPolicies.ts, src/lib/sql/policies.sql.ts and
// src/lib/helpers.ts (Apache-2.0), pin v0.100.0.

//! `GET`, `POST`, `PATCH` and `DELETE` `/pg/policies`.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::routing::get;
use axum::Router;
use serde_json::{json, Map, Value};
use sqlx::{Connection, PgConnection};

use crate::pg_format::{ident as quote_ident, literal as quote_literal};
use crate::{close_meta_connection, fastify_json, open_meta_connection, MetaState};

const SYSTEM_SCHEMAS: &[&str] = &["information_schema", "pg_catalog", "pg_toast"];

const IDENT_NULL: &str = "SQL identifier cannot be null or undefined";
const IDENT_OBJECT: &str = "SQL identifier cannot be an object";
const IDENT_NESTED: &str =
    "Nested array to grouped list conversion is not supported for SQL identifier";
const DESTRUCTURE_NULL: &str = "Cannot destructure property 'name' of 'null' as it is null.";
const ROLES_NULL: &str = "Cannot read properties of null (reading 'map')";
const ROLES_NOT_FN: &str = "roles.map is not a function";
const INVALID_RETRIEVE: &str = "Invalid parameters on policy retrieve";
const EMPTY_JSON: &str = "Body cannot be empty when content-type is set to 'application/json'";
const INCLUDED_SPLIT: &str = "request.query.included_schemas?.split is not a function";
const EXCLUDED_SPLIT: &str = "request.query.excluded_schemas?.split is not a function";

/// `/:id(\\d+)` and the list/create collection.
pub(crate) fn routes() -> Router<MetaState> {
    Router::new()
        .route(
            "/pg/policies",
            get(list).post(create).fallback(method_not_found),
        )
        .route(
            "/pg/policies/:id",
            get(retrieve)
                .patch(update)
                .delete(remove)
                .fallback(method_not_found),
        )
}

// megabase:unit meta:route:GET /pg/policies
async fn list(
    State(state): State<MetaState>,
    RawQuery(query): RawQuery,
) -> axum::response::Response {
    let Some(pool) = state.pool.as_ref() else {
        return missing_connection();
    };
    // Fastify's `fast-querystring` turns a repeated key into an array.
    // `included_schemas?.split(',')` then throws; `setErrorHandler` sends 500.
    if let Some(message) = duplicate_split_error(query.as_deref()) {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, message);
    }
    let sql = list_sql(query.as_deref());
    match fetch_policies(pool, &sql).await {
        Ok(rows) => ok_json(&rows),
        Err(error) => {
            tracing::error!(%error, "postgres-meta policies list failed");
            fail(StatusCode::INTERNAL_SERVER_ERROR, &db_message(&error))
        }
    }
}

// megabase:unit meta:route:GET /pg/policies/{id}
async fn retrieve(
    State(state): State<MetaState>,
    Path(id): Path<String>,
) -> axum::response::Response {
    let Some(pool) = state.pool.as_ref() else {
        return missing_connection();
    };
    match retrieve_by_id(pool, &id).await {
        Ok(policy) => ok_json(&policy),
        Err(failure) => failure.into_response(Retrieve),
    }
}

// megabase:unit meta:route:POST /pg/policies
async fn create(
    State(state): State<MetaState>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let Some(pool) = state.pool.as_ref() else {
        return missing_connection();
    };
    let value = match parse_json_body(&headers, &body) {
        Ok(value) => value,
        Err(message) => return fail(StatusCode::INTERNAL_SERVER_ERROR, &message),
    };
    let plan = match create_plan(&value) {
        Ok(plan) => plan,
        Err(message) => return fail(StatusCode::INTERNAL_SERVER_ERROR, message),
    };
    match insert_policy(pool, &plan).await {
        Ok(policy) => ok_json(&policy),
        Err(failure) => failure.into_response(Create),
    }
}

// megabase:unit meta:route:PATCH /pg/policies/{id}
async fn update(
    State(state): State<MetaState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let Some(pool) = state.pool.as_ref() else {
        return missing_connection();
    };
    // `/:id(\\d+)` misses before the body is read. A null JSON body throws
    // while destructuring, before `policies.update` retrieves the row.
    if matches!(classify_id(&id), IdClass::NotFound) {
        return not_found();
    }
    let value = match parse_json_body(&headers, &body) {
        Ok(value) => value,
        Err(message) => return fail(StatusCode::INTERNAL_SERVER_ERROR, &message),
    };
    if value.is_null() {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, DESTRUCTURE_NULL);
    }
    match update_policy(pool, &id, &value).await {
        Ok(policy) => ok_json(&policy),
        Err(failure) => failure.into_response(Mutate),
    }
}

// megabase:unit meta:route:DELETE /pg/policies/{id}
async fn remove(
    State(state): State<MetaState>,
    Path(id): Path<String>,
) -> axum::response::Response {
    let Some(pool) = state.pool.as_ref() else {
        return missing_connection();
    };
    match delete_policy(pool, &id).await {
        Ok(policy) => ok_json(&policy),
        Err(failure) => failure.into_response(Mutate),
    }
}

async fn method_not_found(State(state): State<MetaState>) -> axum::response::Response {
    if state.pool.is_none() {
        missing_connection()
    } else {
        not_found()
    }
}

#[derive(Debug, sqlx::FromRow, serde::Serialize)]
struct Policy {
    id: i64,
    schema: Option<String>,
    table: String,
    table_id: i64,
    name: String,
    action: String,
    roles: sqlx::types::Json<Vec<String>>,
    command: Option<String>,
    definition: Option<String>,
    check: Option<String>,
}

struct CreatePlan {
    sql: String,
    name: String,
    table: String,
    schema: String,
}

enum Failure {
    Thrown(String),
    Query(String),
    NotFound,
}

#[derive(Clone, Copy)]
enum ErrorRoute {
    Retrieve,
    Create,
    Mutate,
}

use ErrorRoute::{Create, Mutate, Retrieve};

impl Failure {
    fn into_response(self, route: ErrorRoute) -> axum::response::Response {
        match self {
            Self::NotFound => not_found(),
            Self::Thrown(message) => fail(StatusCode::INTERNAL_SERVER_ERROR, &message),
            Self::Query(message) => {
                let status = match route {
                    Retrieve => StatusCode::NOT_FOUND,
                    Create => StatusCode::BAD_REQUEST,
                    Mutate if message.starts_with("Cannot find") => StatusCode::NOT_FOUND,
                    Mutate => StatusCode::BAD_REQUEST,
                };
                fail(status, &message)
            }
        }
    }
}

fn fail(status: StatusCode, message: &str) -> axum::response::Response {
    fastify_json(status, &json!({ "error": message }))
}

fn missing_connection() -> axum::response::Response {
    // `app.ts` `setErrorHandler` answers the `onRequest` throw with
    // `{ error: message }`, not Fastify's default status envelope.
    fail(
        StatusCode::INTERNAL_SERVER_ERROR,
        "failed to get upstream connection details",
    )
}

fn not_found() -> axum::response::Response {
    fail(StatusCode::NOT_FOUND, "Not found")
}

fn ok_json(body: &impl serde::Serialize) -> axum::response::Response {
    match serde_json::to_value(body) {
        Ok(value) => fastify_json(StatusCode::OK, &value),
        Err(error) => fail(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

fn db_message(error: &sqlx::Error) -> String {
    match error.as_database_error() {
        Some(database) => database.message().to_string(),
        None => "connection failed".to_string(),
    }
}

fn parse_json_body(headers: &HeaderMap, body: &[u8]) -> Result<Value, String> {
    let Some(raw) = headers.get(header::CONTENT_TYPE) else {
        return Err("Unsupported Media Type: undefined".to_string());
    };
    let Ok(raw) = raw.to_str() else {
        return Err("Unsupported Media Type: undefined".to_string());
    };
    let essence = raw.split(';').next().unwrap_or("").trim();
    // Fastify 4 matches parser names case-insensitively and treats
    // `text/plain` as a string. `create` then reads no `name` property.
    if essence.eq_ignore_ascii_case("text/plain") {
        return Ok(Value::String(String::from_utf8_lossy(body).into_owned()));
    }
    if !essence.eq_ignore_ascii_case("application/json") {
        return Err(format!("Unsupported Media Type: {raw}"));
    }
    if body.is_empty() {
        return Err(EMPTY_JSON.to_string());
    }
    serde_json::from_slice(body).map_err(|error| error.to_string())
}

fn list_sql(query: Option<&str>) -> String {
    let params = query_map(query);
    let include_system = params
        .get("include_system_schemas")
        .is_some_and(|value| value == "true");
    let included = params.get("included_schemas").map(|value| split_csv(value));
    let excluded = params.get("excluded_schemas").map(|value| split_csv(value));
    let included_refs: Vec<&str> = included.iter().flatten().map(String::as_str).collect();
    let excluded_refs: Vec<&str> = excluded.iter().flatten().map(String::as_str).collect();
    let schema_filter = filter_by_list(
        included.as_ref().map(|_| included_refs.as_slice()),
        excluded.as_ref().map(|_| excluded_refs.as_slice()),
        if include_system {
            None
        } else {
            Some(SYSTEM_SCHEMAS)
        },
    );
    let limit = params
        .get("limit")
        .map(String::as_str)
        .filter(|value| !value.is_empty());
    let offset = params
        .get("offset")
        .map(String::as_str)
        .filter(|value| !value.is_empty());
    policies_sql(&schema_filter, "", "", limit, offset)
}

fn filter_by_list(
    include: Option<&[&str]>,
    exclude: Option<&[&str]>,
    default_exclude: Option<&[&str]>,
) -> String {
    if let Some(include) = include.filter(|items| !items.is_empty()) {
        return format!("IN ({})", join_literals(include));
    }
    if let Some(defaults) = default_exclude {
        let mut items = Vec::with_capacity(defaults.len() + exclude.map_or(0, <[&str]>::len));
        items.extend_from_slice(defaults);
        if let Some(extra) = exclude {
            items.extend_from_slice(extra);
        }
        if items.is_empty() {
            return String::new();
        }
        return format!("NOT IN ({})", join_literals(&items));
    }
    if let Some(exclude) = exclude.filter(|items| !items.is_empty()) {
        return format!("NOT IN ({})", join_literals(exclude));
    }
    String::new()
}

fn filter_by_value(ids: &[&str]) -> String {
    if ids.is_empty() {
        String::new()
    } else {
        format!("IN ({})", join_literals(ids))
    }
}

fn join_literals(items: &[&str]) -> String {
    items
        .iter()
        .map(|item| quote_literal(item))
        .collect::<Vec<_>>()
        .join(",")
}

fn policies_sql(
    schema_filter: &str,
    ids_filter: &str,
    name_filter: &str,
    limit: Option<&str>,
    offset: Option<&str>,
) -> String {
    let schema_line = if schema_filter.is_empty() {
        "true".to_string()
    } else {
        format!("n.nspname {schema_filter}")
    };
    let ids_line = if ids_filter.is_empty() {
        String::new()
    } else {
        format!("AND pol.oid {ids_filter}")
    };
    let name_line = if name_filter.is_empty() {
        String::new()
    } else {
        format!("AND (c.relname || '.' || pol.polname) {name_filter}")
    };
    let limit_line = limit.map_or(String::new(), |limit| {
        format!("limit {}", quote_literal(limit))
    });
    let offset_line = offset.map_or(String::new(), |offset| {
        format!("offset {}", quote_literal(offset))
    });
    let mut sql = String::from(POLICIES_SELECT);
    sql.push_str("  ");
    sql.push_str(&schema_line);
    sql.push('\n');
    sql.push_str("  ");
    sql.push_str(&ids_line);
    sql.push('\n');
    sql.push_str("  ");
    sql.push_str(&name_line);
    sql.push('\n');
    sql.push_str(&limit_line);
    sql.push('\n');
    sql.push_str(&offset_line);
    sql.push('\n');
    sql
}

const POLICIES_SELECT: &str = "
SELECT
  pol.oid :: int8 AS id,
  n.nspname AS schema,
  c.relname AS table,
  c.oid :: int8 AS table_id,
  pol.polname AS name,
  CASE
    WHEN pol.polpermissive THEN 'PERMISSIVE' :: text
    ELSE 'RESTRICTIVE' :: text
  END AS action,
  CASE
    WHEN pol.polroles = '{0}' :: oid [] THEN array_to_json(
      string_to_array('public' :: text, '' :: text) :: name []
    )
    ELSE array_to_json(
      ARRAY(
        SELECT
          pg_roles.rolname
        FROM
          pg_roles
        WHERE
          pg_roles.oid = ANY (pol.polroles)
        ORDER BY
          pg_roles.rolname
      )
    )
  END AS roles,
  CASE
    pol.polcmd
    WHEN 'r' :: \"char\" THEN 'SELECT' :: text
    WHEN 'a' :: \"char\" THEN 'INSERT' :: text
    WHEN 'w' :: \"char\" THEN 'UPDATE' :: text
    WHEN 'd' :: \"char\" THEN 'DELETE' :: text
    WHEN '*' :: \"char\" THEN 'ALL' :: text
    ELSE NULL :: text
  END AS command,
  pg_get_expr(pol.polqual, pol.polrelid) AS definition,
  pg_get_expr(pol.polwithcheck, pol.polrelid) AS check
FROM
  pg_policy pol
  JOIN pg_class c ON c.oid = pol.polrelid
  LEFT JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE
";

/// Repeated `included_schemas` / `excluded_schemas` match Fastify arrays.
/// The handler splits `included_schemas` first, then `excluded_schemas`.
fn duplicate_split_error(query: Option<&str>) -> Option<&'static str> {
    let mut included = 0usize;
    let mut excluded = 0usize;
    let query = query.filter(|query| !query.is_empty())?;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let key = pair.split_once('=').map_or(pair, |(key, _)| key);
        let key = percent_decode(key);
        if key == "included_schemas" {
            included += 1;
        } else if key == "excluded_schemas" {
            excluded += 1;
        }
    }
    if included > 1 {
        Some(INCLUDED_SPLIT)
    } else if excluded > 1 {
        Some(EXCLUDED_SPLIT)
    } else {
        None
    }
}

fn query_map(query: Option<&str>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(query) = query.filter(|query| !query.is_empty()) else {
        return map;
    };
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(percent_decode(key), percent_decode(value));
    }
    map
}

fn split_csv(value: &str) -> Vec<String> {
    value.split(',').map(ToOwned::to_owned).collect()
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                    out.push((high << 4) | low);
                    index += 3;
                } else {
                    out.push(b'%');
                    index += 1;
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn create_plan(body: &Value) -> Result<CreatePlan, &'static str> {
    if body.is_null() {
        return Err(DESTRUCTURE_NULL);
    }
    let get = |key: &str| body.as_object().and_then(|map| map.get(key));
    let name = ident_value(get("name"))?;
    let table = ident_value(get("table"))?;
    let schema = schema_value(get("schema"))?;
    let action = keyword(get("action"), "PERMISSIVE");
    let command = keyword(get("command"), "ALL");
    let roles = role_idents(get("roles"))?;
    let definition = expr_clause(get("definition"), "USING");
    let check = expr_clause(get("check"), "WITH CHECK");
    let sql = format!(
        "\nCREATE POLICY {name_sql} ON {schema_sql}.{table_sql}\n  AS {action}\n  FOR {command}\n  TO {roles}\n  {definition} {check};",
        name_sql = name.sql,
        schema_sql = schema.sql,
        table_sql = table.sql,
        roles = roles.join(","),
    );
    Ok(CreatePlan {
        sql,
        name: name.js,
        table: table.js,
        schema: schema.js,
    })
}

struct IdentValue {
    js: String,
    sql: String,
}

fn ident_value(value: Option<&Value>) -> Result<IdentValue, &'static str> {
    let Some(value) = value else {
        return Err(IDENT_NULL);
    };
    Ok(IdentValue {
        sql: quote_ident_json(value)?,
        js: js_template(value),
    })
}

fn schema_value(value: Option<&Value>) -> Result<IdentValue, &'static str> {
    match value {
        None => Ok(IdentValue {
            js: "public".to_string(),
            sql: quote_ident("public"),
        }),
        Some(value) => ident_value(Some(value)),
    }
}

fn keyword(value: Option<&Value>, default: &str) -> String {
    match value {
        None => default.to_string(),
        Some(value) => js_template(value),
    }
}

fn expr_clause(value: Option<&Value>, prefix: &str) -> String {
    match value {
        None => String::new(),
        Some(value) => format!("{prefix} ({})", js_template(value)),
    }
}

fn role_idents(value: Option<&Value>) -> Result<Vec<String>, &'static str> {
    match value {
        None => Ok(vec![quote_ident("public")]),
        Some(Value::Null) => Err(ROLES_NULL),
        Some(Value::Array(items)) => items.iter().map(quote_ident_json).collect(),
        Some(_) => Err(ROLES_NOT_FN),
    }
}

fn quote_ident_json(value: &Value) -> Result<String, &'static str> {
    match value {
        Value::Null => Err(IDENT_NULL),
        Value::Bool(false) => Ok("\"f\"".to_string()),
        Value::Bool(true) => Ok("\"t\"".to_string()),
        Value::Number(number) => Ok(quote_ident(&number.to_string())),
        Value::String(text) => Ok(quote_ident(text)),
        Value::Array(items) => {
            let mut parts = Vec::with_capacity(items.len());
            for item in items {
                if item.is_array() {
                    return Err(IDENT_NESTED);
                }
                parts.push(quote_ident_json(item)?);
            }
            Ok(parts.join(","))
        }
        Value::Object(_) => Err(IDENT_OBJECT),
    }
}

fn js_template(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(js_template).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

#[cfg(test)]
fn update_sql(policy: &Policy, body: &Value) -> Result<String, &'static str> {
    let [definition_sql, check_sql, roles_sql, name_sql] = update_clauses(policy, body)?;
    Ok(format!(
        "BEGIN; {definition_sql} {check_sql} {roles_sql} {name_sql} COMMIT;"
    ))
}

/// The four `ALTER POLICY` pieces, each `""` or a statement ending in `;`.
fn update_clauses(policy: &Policy, body: &Value) -> Result<[String; 4], &'static str> {
    if body.is_null() {
        return Err(DESTRUCTURE_NULL);
    }
    let schema = policy.schema.as_deref().ok_or(IDENT_NULL)?;
    let alter = format!(
        "ALTER POLICY {} ON {}.{}",
        quote_ident(&policy.name),
        quote_ident(schema),
        quote_ident(&policy.table),
    );
    let get = |key: &str| {
        body.as_object()
            .and_then(|map: &Map<String, Value>| map.get(key))
    };
    let definition_sql = match get("definition") {
        None => String::new(),
        Some(value) => format!("{alter} USING ({});", js_template(value)),
    };
    let check_sql = match get("check") {
        None => String::new(),
        Some(value) => format!("{alter} WITH CHECK ({});", js_template(value)),
    };
    let roles_sql = match get("roles") {
        None => String::new(),
        Some(value) => {
            let roles = role_idents(Some(value))?;
            format!("{alter} TO {};", roles.join(","))
        }
    };
    let name_sql = match get("name") {
        None => String::new(),
        Some(value) => format!("{alter} RENAME TO {};", quote_ident_json(value)?),
    };
    Ok([definition_sql, check_sql, roles_sql, name_sql])
}

fn drop_sql(policy: &Policy) -> Result<String, &'static str> {
    let schema = policy.schema.as_deref().ok_or(IDENT_NULL)?;
    Ok(format!(
        "DROP POLICY {} ON {}.{};",
        quote_ident(&policy.name),
        quote_ident(schema),
        quote_ident(&policy.table),
    ))
}

enum IdClass {
    NotFound,
    Invalid,
    Id(String),
}

fn classify_id(raw: &str) -> IdClass {
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return IdClass::NotFound;
    }
    match js_policy_id(raw) {
        Some(id) => IdClass::Id(id),
        None => IdClass::Invalid,
    }
}

/// `Number(id)` then `String(number)`, for digit-only route params.
fn js_policy_id(digits: &str) -> Option<String> {
    let number: f64 = digits.parse().ok()?;
    if number == 0.0 {
        return None;
    }
    if !number.is_finite() {
        return Some("Infinity".to_string());
    }
    Some(format_js_number(number))
}

fn format_js_number(number: f64) -> String {
    if number >= 1e21 {
        let exp = format!("{number:e}");
        let (coeff, exponent) = exp.split_once('e').unwrap_or((exp.as_str(), "0"));
        return format!("{coeff}e+{exponent}");
    }
    format!("{number}")
}

fn named_sql(schema: &str, table: &str, name: &str) -> String {
    let included = [schema];
    let schema_filter = filter_by_list(Some(&included), Some(&[]), None);
    let pair = format!("{table}.{name}");
    let pairs = [pair.as_str()];
    let name_filter = filter_by_value(&pairs);
    policies_sql(&schema_filter, "", &name_filter, None, None)
}

fn id_sql(id: &str) -> String {
    let ids = [id];
    let ids_filter = filter_by_value(&ids);
    policies_sql("", &ids_filter, "", None, None)
}

async fn fetch_policies(pool: &sqlx::PgPool, sql: &str) -> Result<Vec<Policy>, sqlx::Error> {
    let mut connection = open_meta_connection(pool).await?;
    let rows = sqlx::query_as(sql).fetch_all(&mut connection).await;
    close_meta_connection(connection).await;
    rows
}

async fn retrieve_by_id(pool: &sqlx::PgPool, raw: &str) -> Result<Policy, Failure> {
    let id = match classify_id(raw) {
        IdClass::NotFound => return Err(Failure::NotFound),
        IdClass::Invalid => return Err(Failure::Query(INVALID_RETRIEVE.to_string())),
        IdClass::Id(id) => id,
    };
    let sql = id_sql(&id);
    let mut connection = open_meta_connection(pool)
        .await
        .map_err(|error| Failure::Query(db_message(&error)))?;
    let found = fetch_one(&mut connection, &sql, &id).await;
    close_meta_connection(connection).await;
    found
}

async fn fetch_one(connection: &mut PgConnection, sql: &str, id: &str) -> Result<Policy, Failure> {
    let rows: Vec<Policy> = sqlx::query_as(sql)
        .fetch_all(&mut *connection)
        .await
        .map_err(|error| {
            tracing::error!(%error, "postgres-meta policies query failed");
            Failure::Query(db_message(&error))
        })?;
    rows.into_iter()
        .next()
        .ok_or_else(|| Failure::Query(format!("Cannot find a policy with ID {id}")))
}

async fn insert_policy(pool: &sqlx::PgPool, plan: &CreatePlan) -> Result<Policy, Failure> {
    let mut connection = open_meta_connection(pool)
        .await
        .map_err(|error| Failure::Query(db_message(&error)))?;
    let inserted = insert_on(&mut connection, plan).await;
    close_meta_connection(connection).await;
    inserted
}

async fn insert_on(connection: &mut PgConnection, plan: &CreatePlan) -> Result<Policy, Failure> {
    exec_statement(connection, &plan.sql)
        .await
        .map_err(|error| {
            tracing::error!(%error, "postgres-meta policies create failed");
            Failure::Query(db_message(&error))
        })?;
    let sql = named_sql(&plan.schema, &plan.table, &plan.name);
    let rows: Vec<Policy> = sqlx::query_as(&sql)
        .fetch_all(&mut *connection)
        .await
        .map_err(|error| {
            tracing::error!(%error, "postgres-meta policies retrieve after create failed");
            Failure::Query(db_message(&error))
        })?;
    rows.into_iter().next().ok_or_else(|| {
        Failure::Query(format!(
            "Cannot find a policy named {} for table {}.{}",
            plan.name, plan.schema, plan.table
        ))
    })
}

async fn update_policy(pool: &sqlx::PgPool, raw: &str, body: &Value) -> Result<Policy, Failure> {
    let id = match classify_id(raw) {
        IdClass::NotFound => return Err(Failure::NotFound),
        IdClass::Invalid => return Err(Failure::Query(INVALID_RETRIEVE.to_string())),
        IdClass::Id(id) => id,
    };
    let mut connection = open_meta_connection(pool)
        .await
        .map_err(|error| Failure::Query(db_message(&error)))?;
    let updated = update_on(&mut connection, &id, body).await;
    close_meta_connection(connection).await;
    updated
}

async fn update_on(
    connection: &mut PgConnection,
    id: &str,
    body: &Value,
) -> Result<Policy, Failure> {
    let current = fetch_one(connection, &id_sql(id), id).await?;
    let clauses =
        update_clauses(&current, body).map_err(|message| Failure::Thrown(message.to_string()))?;
    exec_transaction(connection, &clauses)
        .await
        .map_err(|error| {
            tracing::error!(%error, "postgres-meta policies update failed");
            Failure::Query(db_message(&error))
        })?;
    fetch_one(connection, &id_sql(id), id).await
}

async fn delete_policy(pool: &sqlx::PgPool, raw: &str) -> Result<Policy, Failure> {
    let id = match classify_id(raw) {
        IdClass::NotFound => return Err(Failure::NotFound),
        IdClass::Invalid => return Err(Failure::Query(INVALID_RETRIEVE.to_string())),
        IdClass::Id(id) => id,
    };
    let mut connection = open_meta_connection(pool)
        .await
        .map_err(|error| Failure::Query(db_message(&error)))?;
    let deleted = delete_on(&mut connection, &id).await;
    close_meta_connection(connection).await;
    deleted
}

/// One statement on the extended protocol. `sqlx::raw_sql` (simple protocol,
/// several statements) does not produce a `Send` future, which axum requires.
async fn exec_statement(connection: &mut PgConnection, sql: &str) -> Result<(), sqlx::Error> {
    let sql = sql.trim().trim_end_matches(';');
    sqlx::query(sql).execute(&mut *connection).await.map(|_| ())
}

async fn exec_transaction(
    connection: &mut PgConnection,
    clauses: &[String; 4],
) -> Result<(), sqlx::Error> {
    let mut tx = connection.begin().await?;
    for clause in clauses {
        let sql = clause.trim().trim_end_matches(';').trim();
        if sql.is_empty() {
            continue;
        }
        sqlx::query(sql).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn delete_on(connection: &mut PgConnection, id: &str) -> Result<Policy, Failure> {
    let current = fetch_one(connection, &id_sql(id), id).await?;
    let statement = drop_sql(&current).map_err(|message| Failure::Thrown(message.to_string()))?;
    exec_statement(connection, &statement)
        .await
        .map_err(|error| {
            tracing::error!(%error, "postgres-meta policies delete failed");
            Failure::Query(db_message(&error))
        })?;
    Ok(current)
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Method, StatusCode};
    use axum::Router;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use super::*;
    use crate::pg_format::{ident as quote_ident, literal as quote_literal};
    use crate::{router_with_state, MetaState};
    use megabase_core::Hs256;

    const SECRET: &str = "super-secret-jwt-token-with-at-least-32-characters-long";

    fn key(role: &str) -> String {
        Hs256::new(SECRET)
            .unwrap()
            .sign(&json!({ "role": role }))
            .unwrap()
            .as_str()
            .to_string()
    }

    fn app(pool: Option<sqlx::PgPool>) -> Router {
        router_with_state(MetaState::new(pool, Some(Hs256::new(SECRET).unwrap())))
    }

    fn unreachable() -> sqlx::PgPool {
        sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(200))
            .connect_lazy("postgres://postgres@127.0.0.1:1/postgres")
            .unwrap()
    }

    async fn call(
        pool: Option<sqlx::PgPool>,
        method: Method,
        path: &str,
        apikey: Option<&str>,
        content_type: Option<&str>,
        body: &str,
    ) -> (StatusCode, Value) {
        let mut request = axum::http::Request::builder().method(method).uri(path);
        if let Some(apikey) = apikey {
            request = request.header("apikey", apikey);
        }
        if let Some(content_type) = content_type {
            request = request.header(header::CONTENT_TYPE, content_type);
        }
        let response = app(pool)
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let content_type = response.headers()[header::CONTENT_TYPE].clone();
        assert_eq!(content_type, "application/json; charset=utf-8");
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[test]
    fn quoting_matches_pg_format() {
        assert_eq!(quote_ident("public"), "public");
        assert_eq!(quote_ident("user"), "\"user\"");
        assert_eq!(quote_ident("test policy"), "\"test policy\"");
        assert_eq!(quote_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_literal("10"), "'10'");
        assert_eq!(quote_literal("a'\\b"), "E'a''\\\\b'");
    }

    #[test]
    fn list_sql_matches_postgres_meta_filters() {
        let sql = list_sql(None);
        assert!(sql.contains("n.nspname NOT IN ('information_schema','pg_catalog','pg_toast')"));
        assert!(!sql.contains("limit "));
        let limited = list_sql(Some("limit=10&offset=0"));
        assert!(limited.contains("limit '10'\n"));
        assert!(limited.contains("offset '0'\n"));
        let included = list_sql(Some("included_schemas=public,core&excluded_schemas=secret"));
        assert!(included.contains("n.nspname IN ('public','core')"));
        assert!(!included.contains("secret"));
        let excluded = list_sql(Some("excluded_schemas=public"));
        assert!(excluded
            .contains("n.nspname NOT IN ('information_schema','pg_catalog','pg_toast','public')"));
        let system = list_sql(Some("include_system_schemas=true&excluded_schemas=public"));
        assert!(system.contains("n.nspname NOT IN ('public')"));
        assert!(!system.contains("pg_catalog"));
        let empty = list_sql(Some("included_schemas="));
        assert!(empty.contains("n.nspname IN ('')"));
        let zero = list_sql(Some("limit=0"));
        assert!(zero.contains("limit '0'\n"), "{zero}");
        assert_eq!(
            duplicate_split_error(Some("included_schemas=secret&included_schemas=public")),
            Some(INCLUDED_SPLIT)
        );
        assert_eq!(
            duplicate_split_error(Some("excluded_schemas=a&excluded_schemas=b")),
            Some(EXCLUDED_SPLIT)
        );
        assert_eq!(
            duplicate_split_error(Some(
                "excluded_schemas=a&excluded_schemas=b&included_schemas=c&included_schemas=d"
            )),
            Some(INCLUDED_SPLIT)
        );
        assert_eq!(
            duplicate_split_error(Some("included_schemas=public,core")),
            None
        );
    }

    #[test]
    fn create_and_update_sql_match_pg_format_templates() {
        let plan = create_plan(&json!({
            "name": "test policy",
            "table": "memes",
            "action": "RESTRICTIVE"
        }))
        .unwrap();
        assert_eq!(
            plan.sql,
            "\nCREATE POLICY \"test policy\" ON public.memes\n  AS RESTRICTIVE\n  FOR ALL\n  TO public\n   ;"
        );
        assert_eq!(plan.schema, "public");
        let full = create_plan(&json!({
            "name": "p",
            "table": "t",
            "schema": "My Schema",
            "definition": "true",
            "check": "id > 0",
            "command": "SELECT",
            "roles": ["public", "user"]
        }))
        .unwrap();
        assert_eq!(
            full.sql,
            "\nCREATE POLICY p ON \"My Schema\".t\n  AS PERMISSIVE\n  FOR SELECT\n  TO public,\"user\"\n  USING (true) WITH CHECK (id > 0);"
        );
        let policy = Policy {
            id: 1,
            schema: Some("public".to_string()),
            table: "memes".to_string(),
            table_id: 2,
            name: "old".to_string(),
            action: "PERMISSIVE".to_string(),
            roles: sqlx::types::Json(vec!["public".to_string()]),
            command: Some("ALL".to_string()),
            definition: None,
            check: None,
        };
        assert_eq!(
            update_sql(
                &policy,
                &json!({
                    "name": "policy updated",
                    "definition": "true",
                    "check": "true",
                    "roles": ["postgres"]
                })
            )
            .unwrap(),
            "BEGIN; ALTER POLICY \"old\" ON public.memes USING (true); ALTER POLICY \"old\" ON public.memes WITH CHECK (true); ALTER POLICY \"old\" ON public.memes TO postgres; ALTER POLICY \"old\" ON public.memes RENAME TO \"policy updated\"; COMMIT;"
        );
        assert_eq!(
            update_sql(&policy, &json!({})).unwrap(),
            "BEGIN;     COMMIT;"
        );
        assert_eq!(
            drop_sql(&policy).unwrap(),
            "DROP POLICY \"old\" ON public.memes;"
        );
    }

    #[test]
    fn query_parser_does_not_panic_on_arbitrary_bytes() {
        let mut messy = String::new();
        for byte in 0u8..=255 {
            messy.push(char::from(byte));
        }
        messy.push_str("%zz%a+&a=1&a=2&included_schemas=%ff");
        let _ = percent_decode(&messy);
        let _ = query_map(Some(&messy));
        let _ = duplicate_split_error(Some(&messy));
        let _ = list_sql(Some(&messy));
        let _ = list_sql(None);
    }

    #[test]
    fn javascript_id_strings_match_number_tostring() {
        assert!(matches!(classify_id("abc"), IdClass::NotFound));
        assert!(matches!(classify_id("0"), IdClass::Invalid));
        assert!(matches!(classify_id("00"), IdClass::Invalid));
        assert!(matches!(classify_id("01"), IdClass::Id(id) if id == "1"));
        assert!(
            matches!(classify_id("9007199254740993"), IdClass::Id(id) if id == "9007199254740992")
        );
        assert!(matches!(
            classify_id("1000000000000000000000"),
            IdClass::Id(id) if id == "1e+21"
        ));
        assert!(matches!(
            classify_id("123456789012345678901234"),
            IdClass::Id(id) if id == "1.2345678901234569e+23"
        ));
        let huge = format!("1{}", "0".repeat(309));
        assert!(matches!(classify_id(&huge), IdClass::Id(id) if id == "Infinity"));
    }

    #[tokio::test]
    async fn service_role_is_required() {
        let admin = key("service_role");
        let (status, body) = call(
            None,
            Method::GET,
            "/pg/policies",
            Some(&key("anon")),
            None,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["message"], "You cannot consume this service");
        let (status, body) = call(None, Method::GET, "/pg/policies", Some(&admin), None, "").await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "failed to get upstream connection details");
        assert!(body.get("message").is_none());
    }

    #[tokio::test]
    async fn unknown_id_and_method_are_fastify_404_when_a_database_is_configured() {
        let admin = key("service_role");
        let pool = Some(unreachable());
        let (status, body) = call(
            pool.clone(),
            Method::GET,
            "/pg/policies/abc",
            Some(&admin),
            None,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "Not found");
        let (status, body) = call(
            pool.clone(),
            Method::PUT,
            "/pg/policies",
            Some(&admin),
            None,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "Not found");
        let (status, body) =
            call(pool, Method::GET, "/pg/policies/0", Some(&admin), None, "").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], INVALID_RETRIEVE);
    }

    #[tokio::test]
    async fn writes_report_body_errors_before_touching_postgres() {
        let admin = key("service_role");
        let pool = Some(unreachable());
        let (status, body) = call(
            pool.clone(),
            Method::POST,
            "/pg/policies",
            Some(&admin),
            Some("application/json"),
            "",
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], EMPTY_JSON);
        let (status, body) = call(
            pool.clone(),
            Method::POST,
            "/pg/policies",
            Some(&admin),
            Some("application/json"),
            "null",
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], DESTRUCTURE_NULL);
        let (status, body) = call(
            pool.clone(),
            Method::POST,
            "/pg/policies",
            Some(&admin),
            Some("application/json"),
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], IDENT_NULL);
        let (status, body) = call(
            pool.clone(),
            Method::POST,
            "/pg/policies",
            Some(&admin),
            None,
            "{\"name\":\"p\",\"table\":\"t\"}",
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "Unsupported Media Type: undefined");
        let (status, body) = call(
            pool.clone(),
            Method::POST,
            "/pg/policies",
            Some(&admin),
            Some("Application/JSON"),
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], IDENT_NULL);
        let (status, body) = call(
            pool.clone(),
            Method::POST,
            "/pg/policies",
            Some(&admin),
            Some("Application/JSON; charset=utf-8"),
            "",
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], EMPTY_JSON);
        let (status, body) = call(
            pool.clone(),
            Method::POST,
            "/pg/policies",
            Some(&admin),
            Some("text/plain"),
            "{\"name\":\"p\",\"table\":\"t\"}",
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], IDENT_NULL);
        let (status, body) = call(
            pool.clone(),
            Method::PATCH,
            "/pg/policies/0",
            Some(&admin),
            Some("application/json"),
            "null",
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], DESTRUCTURE_NULL);
        let (status, body) = call(
            pool,
            Method::PATCH,
            "/pg/policies/0",
            Some(&admin),
            Some("application/json"),
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], INVALID_RETRIEVE);
    }

    #[tokio::test]
    async fn a_down_database_uses_the_route_status() {
        let admin = key("service_role");
        let pool = Some(unreachable());
        let (status, body) = call(
            pool.clone(),
            Method::GET,
            "/pg/policies",
            Some(&admin),
            None,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "connection failed");
        let (status, body) = call(
            pool.clone(),
            Method::POST,
            "/pg/policies",
            Some(&admin),
            Some("application/json"),
            "{\"name\":\"p\",\"table\":\"t\"}",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "connection failed");
        let (status, body) = call(
            pool.clone(),
            Method::GET,
            "/pg/policies/15",
            Some(&admin),
            None,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "connection failed");
        let (status, body) = call(
            pool,
            Method::DELETE,
            "/pg/policies/15",
            Some(&admin),
            None,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "connection failed");
    }

    #[tokio::test]
    async fn repeated_schema_filters_match_fastify_split_typeerror() {
        let admin = key("service_role");
        let pool = Some(unreachable());
        let (status, body) = call(
            pool.clone(),
            Method::GET,
            "/pg/policies?included_schemas=secret&included_schemas=public",
            Some(&admin),
            None,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], INCLUDED_SPLIT);
        let (status, body) = call(
            pool,
            Method::GET,
            "/pg/policies?excluded_schemas=a&excluded_schemas=b",
            Some(&admin),
            None,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], EXCLUDED_SPLIT);
    }

    #[tokio::test]
    async fn round_trip_when_database_url_is_set() {
        let Ok(url) = std::env::var("MEGABASE_TEST_DATABASE_URL") else {
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect(&url)
            .await
            .expect("test database");
        let table = "megabase_policies_probe";
        sqlx::query(&format!("DROP TABLE IF EXISTS public.{table}"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(&format!("CREATE TABLE public.{table} (id int)"))
            .execute(&pool)
            .await
            .unwrap();
        let admin = key("service_role");
        let (status, created) = call(
            Some(pool.clone()),
            Method::POST,
            "/pg/policies",
            Some(&admin),
            Some("application/json"),
            &format!(
                "{{\"name\":\"probe policy\",\"table\":\"{table}\",\"action\":\"RESTRICTIVE\",\"command\":\"SELECT\",\"definition\":\"true\"}}"
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        assert_eq!(created["name"], "probe policy");
        assert_eq!(created["action"], "RESTRICTIVE");
        assert_eq!(created["command"], "SELECT");
        assert_eq!(created["schema"], "public");
        assert_eq!(created["table"], table);
        assert_eq!(created["roles"], json!(["public"]));
        assert!(created["definition"].as_str().unwrap().contains("true"));
        let id = created["id"].as_i64().unwrap();
        let (status, listed) = call(
            Some(pool.clone()),
            Method::GET,
            "/pg/policies?included_schemas=public&limit=1000",
            Some(&admin),
            None,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert!(listed
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == created["id"]));
        let (status, one) = call(
            Some(pool.clone()),
            Method::GET,
            &format!("/pg/policies/{id}"),
            Some(&admin),
            None,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(one["id"], created["id"]);
        let (status, patched) = call(
            Some(pool.clone()),
            Method::PATCH,
            &format!("/pg/policies/{id}"),
            Some(&admin),
            Some("application/json"),
            "{\"name\":\"probe renamed\",\"roles\":[\"postgres\"]}",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{patched}");
        assert_eq!(patched["name"], "probe renamed");
        assert_eq!(patched["roles"], json!(["postgres"]));
        assert_eq!(patched["id"], created["id"]);
        let (status, removed) = call(
            Some(pool.clone()),
            Method::DELETE,
            &format!("/pg/policies/{id}"),
            Some(&admin),
            None,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{removed}");
        assert_eq!(removed["name"], "probe renamed");
        let (status, missing) = call(
            Some(pool.clone()),
            Method::GET,
            &format!("/pg/policies/{id}"),
            Some(&admin),
            None,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(
            missing["error"],
            format!("Cannot find a policy with ID {id}")
        );
        let (status, missing_delete) = call(
            Some(pool.clone()),
            Method::DELETE,
            &format!("/pg/policies/{id}"),
            Some(&admin),
            None,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(
            missing_delete["error"],
            format!("Cannot find a policy with ID {id}")
        );
        sqlx::query(&format!("DROP TABLE IF EXISTS public.{table}"))
            .execute(&pool)
            .await
            .unwrap();
    }
}
