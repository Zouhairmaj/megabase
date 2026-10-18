//! `/pg/tables` list, retrieve, create, update and delete.
//!
//! Statements run on one connection opened from the process pool. The `pg`
//! header is not a connection string.

// Ported from supabase/postgres-meta src/server/routes/tables.ts,
// src/lib/PostgresMetaTables.ts, src/lib/sql/table.sql.ts,
// src/lib/sql/columns.sql.ts and src/lib/helpers.ts (Apache-2.0), pin v0.100.0.

use std::future::Future;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{OriginalUri, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};
use sqlx::{Connection, PgConnection};

use crate::pg_format::{filter_by_list, ident, literal, literal_i64};
use crate::{fastify_json, no_connection, MetaState, COMPONENT, CONNECT_TIMEOUT};

const QUERY_TIMEOUT: Duration = Duration::from_secs(55);
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const SYSTEM_SCHEMAS: &[&str] = &["information_schema", "pg_catalog", "pg_toast"];
const INVALID_PARAMETERS: &str = "Invalid parameters on table retrieve";

struct QueryParts {
    schema_filter: String,
    ids_filter: String,
    table_identifier_filter: String,
    limit: String,
    offset: String,
    include_columns: bool,
}

struct ListQuery {
    include_system_schemas: bool,
    included_schemas: Vec<String>,
    excluded_schemas: Vec<String>,
    limit: Option<i64>,
    offset: Option<i64>,
    include_columns: bool,
}

struct TableUpdate {
    name: Option<String>,
    schema: Option<String>,
    rls_enabled: Option<bool>,
    rls_forced: Option<bool>,
    replica_identity: Option<Replica>,
    replica_identity_index: Option<String>,
    primary_keys: Option<Vec<String>>,
    comment: Option<String>,
}

struct TableSnapshot {
    schema: String,
    name: String,
    primary_key_count: usize,
}

enum Replica {
    Default,
    Index,
    Full,
    Nothing,
}

impl Replica {
    fn as_sql(&self) -> &'static str {
        match self {
            Self::Default => "DEFAULT",
            Self::Index => "INDEX",
            Self::Full => "FULL",
            Self::Nothing => "NOTHING",
        }
    }
}

enum IdParse {
    NotRoute,
    Invalid,
    Zero,
    Id(i64),
}

enum DbFail {
    ConnectTimeout,
    QueryTimeout,
    Other(sqlx::Error),
}

impl From<sqlx::Error> for DbFail {
    fn from(error: sqlx::Error) -> Self {
        Self::Other(error)
    }
}

impl DbFail {
    fn message(&self) -> String {
        match self {
            Self::ConnectTimeout => "Connection terminated due to connection timeout".to_string(),
            Self::QueryTimeout => "Query read timeout".to_string(),
            Self::Other(error) => match error.as_database_error() {
                Some(database) => database.message().to_string(),
                None => "connection failed".to_string(),
            },
        }
    }
}

// megabase:unit meta:route:GET /pg/tables
pub(crate) async fn list_tables(
    State(state): State<MetaState>,
    OriginalUri(uri): OriginalUri,
) -> Response {
    let Some(pool) = state.pool.as_ref() else {
        return no_connection();
    };
    let query = match parse_list_query(uri.query()) {
        Ok(query) => query,
        Err(message) => return error_body(StatusCode::INTERNAL_SERVER_ERROR, &message),
    };
    let sql = list_json_sql(&query);
    let connection = match connect_dedicated(pool).await {
        Ok(connection) => connection,
        Err(error) => return db_error(&error, list_status),
    };
    match wait_query(fetch_rows(connection, sql)).await {
        Ok(rows) => fastify_json(StatusCode::OK, &Value::Array(rows)),
        Err(error) => {
            log_db(&error);
            let message = error.message();
            error_body(list_status(&message), &message)
        }
    }
}

// megabase:unit meta:route:GET /pg/tables/{id}
pub(crate) async fn retrieve_table(
    State(state): State<MetaState>,
    OriginalUri(uri): OriginalUri,
    Path(raw_id): Path<String>,
) -> Response {
    match parse_id(&raw_id) {
        IdParse::NotRoute => not_implemented("GET", uri.path()),
        IdParse::Invalid => id_route(state.pool.as_ref(), || {
            error_body(
                StatusCode::INTERNAL_SERVER_ERROR,
                "params/id must be integer",
            )
        }),
        IdParse::Zero => id_route(state.pool.as_ref(), || {
            error_body(StatusCode::NOT_FOUND, INVALID_PARAMETERS)
        }),
        IdParse::Id(id) => {
            let Some(pool) = state.pool.as_ref() else {
                return no_connection();
            };
            let missing = format!("Cannot find a table with ID {id}");
            one_table(pool, retrieve_id_sql(id), missing).await
        }
    }
}

// megabase:unit meta:route:POST /pg/tables
pub(crate) async fn create_table(State(state): State<MetaState>, body: Bytes) -> Response {
    let Some(pool) = state.pool.as_ref() else {
        return no_connection();
    };
    let (name, schema, comment) = match parse_create(&body) {
        Ok(parsed) => parsed,
        Err(message) => return error_body(StatusCode::INTERNAL_SERVER_ERROR, &message),
    };
    let statements = create_statements(&name, &schema, comment.as_deref());
    let sql = retrieve_name_sql(&schema, &name);
    let missing = format!("Cannot find a table named {name} in schema {schema}");
    mutate(pool, statements, sql, missing).await
}

// megabase:unit meta:route:PATCH /pg/tables/{id}
pub(crate) async fn update_table(
    State(state): State<MetaState>,
    OriginalUri(uri): OriginalUri,
    Path(raw_id): Path<String>,
    body: Bytes,
) -> Response {
    let id = match parsed_route_id(&raw_id, "PATCH", uri.path(), state.pool.as_ref()) {
        RouteId::Done(response) => return response,
        RouteId::Id(id) => id,
    };
    let Some(pool) = state.pool.as_ref() else {
        return no_connection();
    };
    let update = match parse_update(&body) {
        Ok(update) => update,
        Err(message) => return error_body(StatusCode::INTERNAL_SERVER_ERROR, &message),
    };
    if id == 0 {
        return error_body(StatusCode::BAD_REQUEST, INVALID_PARAMETERS);
    }
    let sql = retrieve_id_sql(id);
    let current = match timed(query_owned(pool, sql)).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            return error_body(
                StatusCode::NOT_FOUND,
                &format!("Cannot find a table with ID {id}"),
            )
        }
        Err(error) => return db_error(&error, mutation_status),
    };
    let snapshot = match snapshot(&current) {
        Ok(snapshot) => snapshot,
        Err(message) => return error_body(StatusCode::BAD_REQUEST, &message),
    };
    let statements = update_statements(id, &snapshot, &update);
    if let Err(error) = timed(exec_owned(pool.clone(), statements)).await {
        return db_error(&error, mutation_status);
    }
    match timed(query_owned(pool, retrieve_id_sql(id))).await {
        Ok(Some(row)) => fastify_json(StatusCode::OK, &row),
        Ok(None) => error_body(
            StatusCode::NOT_FOUND,
            &format!("Cannot find a table with ID {id}"),
        ),
        Err(error) => db_error(&error, mutation_status),
    }
}

// megabase:unit meta:route:DELETE /pg/tables/{id}
pub(crate) async fn remove_table(
    State(state): State<MetaState>,
    OriginalUri(uri): OriginalUri,
    Path(raw_id): Path<String>,
) -> Response {
    let id = match parsed_route_id(&raw_id, "DELETE", uri.path(), state.pool.as_ref()) {
        RouteId::Done(response) => return response,
        RouteId::Id(id) => id,
    };
    let Some(pool) = state.pool.as_ref() else {
        return no_connection();
    };
    let cascade = match parse_cascade(uri.query()) {
        Ok(cascade) => cascade,
        Err(message) => return error_body(StatusCode::INTERNAL_SERVER_ERROR, &message),
    };
    if id == 0 {
        return error_body(StatusCode::BAD_REQUEST, INVALID_PARAMETERS);
    }
    let sql = retrieve_id_sql(id);
    let current = match timed(query_owned(pool, sql)).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            return error_body(
                StatusCode::NOT_FOUND,
                &format!("Cannot find a table with ID {id}"),
            )
        }
        Err(error) => return db_error(&error, mutation_status),
    };
    let snapshot = match snapshot(&current) {
        Ok(snapshot) => snapshot,
        Err(message) => return error_body(StatusCode::BAD_REQUEST, &message),
    };
    let statement = drop_statement(&snapshot.schema, &snapshot.name, cascade);
    if let Err(error) = timed(exec_owned(pool.clone(), vec![statement])).await {
        return db_error(&error, mutation_status);
    }
    fastify_json(StatusCode::OK, &current)
}

enum RouteId {
    Done(Response),
    Id(i64),
}

fn parsed_route_id(raw_id: &str, method: &str, path: &str, pool: Option<&sqlx::PgPool>) -> RouteId {
    match parse_id(raw_id) {
        IdParse::NotRoute => RouteId::Done(not_implemented(method, path)),
        IdParse::Invalid => RouteId::Done(id_route(pool, || {
            error_body(
                StatusCode::INTERNAL_SERVER_ERROR,
                "params/id must be integer",
            )
        })),
        IdParse::Zero => RouteId::Id(0),
        IdParse::Id(id) => RouteId::Id(id),
    }
}

fn id_route(pool: Option<&sqlx::PgPool>, response: impl FnOnce() -> Response) -> Response {
    if pool.is_none() {
        no_connection()
    } else {
        response()
    }
}

fn first_row(value: Value) -> Option<Value> {
    match value {
        Value::Array(rows) => rows.into_iter().next(),
        other => Some(other),
    }
}

async fn one_table(pool: &sqlx::PgPool, sql: String, missing: String) -> Response {
    let connection = match connect_dedicated(pool).await {
        Ok(connection) => connection,
        Err(error) => return db_error(&error, |_| StatusCode::NOT_FOUND),
    };
    match wait_query(fetch_rows(connection, sql)).await {
        Ok(rows) => match rows.into_iter().next() {
            Some(row) => fastify_json(StatusCode::OK, &row),
            None => error_body(StatusCode::NOT_FOUND, &missing),
        },
        Err(error) => {
            log_db(&error);
            error_body(StatusCode::NOT_FOUND, &error.message())
        }
    }
}

async fn mutate(
    pool: &sqlx::PgPool,
    statements: Vec<String>,
    retrieve_sql: String,
    missing: String,
) -> Response {
    if !statements.is_empty() {
        if let Err(error) = timed(exec_owned(pool.clone(), statements)).await {
            return db_error(&error, |_| StatusCode::BAD_REQUEST);
        }
    }
    match timed(query_owned(pool, retrieve_sql)).await {
        Ok(Some(row)) => fastify_json(StatusCode::OK, &row),
        Ok(None) => error_body(StatusCode::BAD_REQUEST, &missing),
        Err(error) => db_error(&error, |_| StatusCode::BAD_REQUEST),
    }
}

fn list_status(message: &str) -> StatusCode {
    match message {
        "Connection terminated due to connection timeout" => StatusCode::GATEWAY_TIMEOUT,
        "sorry, too many clients already" => StatusCode::SERVICE_UNAVAILABLE,
        "Query read timeout" => StatusCode::REQUEST_TIMEOUT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn mutation_status(message: &str) -> StatusCode {
    if message.starts_with("Cannot find") {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::BAD_REQUEST
    }
}

fn error_body(status: StatusCode, message: &str) -> Response {
    fastify_json(status, &serde_json::json!({ "error": message }))
}

fn not_implemented(method: &str, path: &str) -> Response {
    megabase_core::MegabaseNotImplemented::new(COMPONENT, format!("{method} {path}"))
        .into_response()
}

fn log_db(error: &DbFail) {
    let message = error.message();
    tracing::error!(%message, "postgres-meta table query failed");
}

fn parse_id(raw: &str) -> IdParse {
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return IdParse::NotRoute;
    }
    match raw.parse::<u64>() {
        Ok(0) => IdParse::Zero,
        Ok(value) if value <= MAX_SAFE_INTEGER => {
            IdParse::Id(i64::try_from(value).expect("a JavaScript safe integer fits in i64"))
        }
        _ => IdParse::Invalid,
    }
}

async fn connect_dedicated(pool: &sqlx::PgPool) -> Result<PgConnection, DbFail> {
    let options = pool.connect_options();
    match tokio::time::timeout(CONNECT_TIMEOUT, PgConnection::connect_with(&options)).await {
        Ok(Ok(connection)) => Ok(connection),
        Ok(Err(error)) => Err(DbFail::Other(error)),
        Err(_elapsed) => Err(DbFail::ConnectTimeout),
    }
}

async fn wait_query<T, E>(work: impl Future<Output = Result<T, E>> + Send) -> Result<T, E>
where
    E: From<DbFail>,
{
    match tokio::time::timeout(QUERY_TIMEOUT, work).await {
        Ok(result) => result,
        Err(_elapsed) => Err(E::from(DbFail::QueryTimeout)),
    }
}

fn db_error(error: &DbFail, status: impl FnOnce(&str) -> StatusCode) -> Response {
    log_db(error);
    let message = error.message();
    error_body(status(&message), &message)
}

async fn fetch_rows(mut connection: PgConnection, sql: String) -> Result<Vec<Value>, DbFail> {
    let result = sqlx::query_scalar::<_, Value>(&sql)
        .fetch_one(&mut connection)
        .await
        .map_err(DbFail::from);
    close_connection(connection).await;
    match result? {
        Value::Array(rows) => Ok(rows),
        other => Ok(vec![other]),
    }
}

async fn query_owned(pool: &sqlx::PgPool, sql: String) -> Result<Option<Value>, DbFail> {
    let mut connection = connect_dedicated(pool).await?;
    let result = sqlx::query_scalar::<_, Value>(&sql)
        .fetch_one(&mut connection)
        .await
        .map_err(DbFail::from);
    close_connection(connection).await;
    result.map(first_row)
}

async fn exec_owned(pool: sqlx::PgPool, statements: Vec<String>) -> Result<(), DbFail> {
    if statements.is_empty() {
        return Ok(());
    }
    let mut connection = connect_dedicated(&pool).await?;
    let mut tx = connection.begin().await.map_err(DbFail::from)?;
    for statement in &statements {
        sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .map_err(DbFail::from)?;
    }
    tx.commit().await.map_err(DbFail::from)?;
    Ok(())
}

async fn timed<T>(work: impl Future<Output = Result<T, DbFail>> + Send) -> Result<T, DbFail> {
    match tokio::time::timeout(QUERY_TIMEOUT, work).await {
        Ok(result) => result,
        Err(_elapsed) => Err(DbFail::QueryTimeout),
    }
}

async fn close_connection(connection: PgConnection) {
    if let Err(error) = connection.close().await {
        tracing::warn!(%error, "closing the postgres-meta connection failed");
    }
}

fn snapshot(row: &Value) -> Result<TableSnapshot, String> {
    let schema = row
        .get("schema")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing schema".to_string())?
        .to_string();
    let name = row
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing name".to_string())?
        .to_string();
    let primary_key_count = row
        .get("primary_keys")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    Ok(TableSnapshot {
        schema,
        name,
        primary_key_count,
    })
}

fn list_json_sql(query: &ListQuery) -> String {
    let included: Vec<&str> = query.included_schemas.iter().map(String::as_str).collect();
    let excluded: Vec<&str> = query.excluded_schemas.iter().map(String::as_str).collect();
    let default_exclude: &[&str] = if query.include_system_schemas {
        &[]
    } else {
        SYSTEM_SCHEMAS
    };
    let (limit, offset) = limit_offset(query.limit, query.offset);
    json_sql(&enriched(&QueryParts {
        schema_filter: filter_by_list(&included, &excluded, default_exclude),
        ids_filter: String::new(),
        table_identifier_filter: String::new(),
        limit,
        offset,
        include_columns: query.include_columns,
    }))
}

fn retrieve_id_sql(id: i64) -> String {
    let ids_filter = format!("IN ({})", literal_i64(id));
    json_sql(&enriched(&QueryParts {
        schema_filter: String::new(),
        ids_filter,
        table_identifier_filter: String::new(),
        limit: String::new(),
        offset: String::new(),
        include_columns: true,
    }))
}

fn retrieve_name_sql(schema: &str, name: &str) -> String {
    let schema_filter = if schema.is_empty() {
        String::new()
    } else {
        filter_by_list(&[schema], &[], &[])
    };
    let table_identifier_filter = format!("IN ({})", literal(&format!("{schema}.{name}")));
    json_sql(&enriched(&QueryParts {
        schema_filter,
        ids_filter: String::new(),
        table_identifier_filter,
        limit: String::new(),
        offset: String::new(),
        include_columns: true,
    }))
}

fn limit_offset(limit: Option<i64>, offset: Option<i64>) -> (String, String) {
    let limit = match limit {
        Some(value) if value != 0 => format!("limit {}", literal_i64(value)),
        _ => String::new(),
    };
    let offset = match offset {
        Some(value) if value != 0 => format!("offset {}", literal_i64(value)),
        _ => String::new(),
    };
    (limit, offset)
}

fn json_sql(inner: &str) -> String {
    format!(
        "SELECT COALESCE(jsonb_agg(to_jsonb(tables_row)), '[]'::jsonb) FROM ({inner}) AS tables_row"
    )
}

fn enriched(parts: &QueryParts) -> String {
    let tables = tables_sql(parts);
    if parts.include_columns {
        let columns = columns_sql(parts);
        let extra = coalesce_rows("columns", "columns.table_id = tables.id");
        format!("with tables as ({tables}), columns as ({columns}) select *, {extra} from tables")
    } else {
        format!("with tables as ({tables}) select * from tables")
    }
}

fn and_filter(expr: &str, filter: &str) -> String {
    if filter.is_empty() {
        String::new()
    } else {
        format!("{expr} {filter} AND")
    }
}

fn or_and_filter(left: &str, right: &str, filter: &str) -> String {
    if filter.is_empty() {
        String::new()
    } else {
        format!("{left} {filter} OR {right} {filter} AND")
    }
}

fn tables_sql(parts: &QueryParts) -> String {
    let pk_schema = and_filter("n.nspname", &parts.schema_filter);
    let pk_table = and_filter(
        "n.nspname || '.' || c.relname",
        &parts.table_identifier_filter,
    );
    let rel_schema = or_and_filter("nsa.nspname", "nta.nspname", &parts.schema_filter);
    let rel_table = or_and_filter(
        "(nsa.nspname || '.' || csa.relname)",
        "(nta.nspname || '.' || cta.relname)",
        &parts.table_identifier_filter,
    );
    let where_schema = and_filter("nc.nspname", &parts.schema_filter);
    let where_ids = and_filter("c.oid", &parts.ids_filter);
    let where_table = and_filter(
        "nc.nspname || '.' || c.relname",
        &parts.table_identifier_filter,
    );
    format!(
        r#"
SELECT
  c.oid :: int8 AS id,
  nc.nspname AS schema,
  c.relname AS name,
  c.relrowsecurity AS rls_enabled,
  c.relforcerowsecurity AS rls_forced,
  CASE
    WHEN c.relreplident = 'd' THEN 'DEFAULT'
    WHEN c.relreplident = 'i' THEN 'INDEX'
    WHEN c.relreplident = 'f' THEN 'FULL'
    ELSE 'NOTHING'
  END AS replica_identity,
  pg_total_relation_size(format('%I.%I', nc.nspname, c.relname)) :: int8 AS bytes,
  pg_size_pretty(
    pg_total_relation_size(format('%I.%I', nc.nspname, c.relname))
  ) AS size,
  pg_stat_get_live_tuples(c.oid) AS live_rows_estimate,
  pg_stat_get_dead_tuples(c.oid) AS dead_rows_estimate,
  obj_description(c.oid) AS comment,
  coalesce(pk.primary_keys, '[]') as primary_keys,
  coalesce(
    jsonb_agg(relationships) filter (where relationships is not null),
    '[]'
  ) as relationships
FROM
  pg_namespace nc
  JOIN pg_class c ON nc.oid = c.relnamespace
  left join (
    select
      c.oid::int8 as table_id,
      jsonb_agg(
        jsonb_build_object(
          'table_id', c.oid::int8,
          'schema', n.nspname,
          'table_name', c.relname,
          'name', a.attname
        )
        order by array_position(i.indkey, a.attnum)
      ) as primary_keys
    from
      pg_index i
      join pg_class c on i.indrelid = c.oid
      join pg_namespace n on c.relnamespace = n.oid
      join pg_attribute a on a.attrelid = c.oid and a.attnum = any(i.indkey)
    where
      {pk_schema}
      {pk_table}
      i.indisprimary
    group by c.oid
  ) as pk
  on pk.table_id = c.oid
  left join (
    select
      c.oid :: int8 as id,
      c.conname as constraint_name,
      nsa.nspname as source_schema,
      csa.relname as source_table_name,
      sa.attname as source_column_name,
      nta.nspname as target_table_schema,
      cta.relname as target_table_name,
      ta.attname as target_column_name
    from
      pg_constraint c
    join (
      pg_attribute sa
      join pg_class csa on sa.attrelid = csa.oid
      join pg_namespace nsa on csa.relnamespace = nsa.oid
    ) on sa.attrelid = c.conrelid and sa.attnum = any (c.conkey)
    join (
      pg_attribute ta
      join pg_class cta on ta.attrelid = cta.oid
      join pg_namespace nta on cta.relnamespace = nta.oid
    ) on ta.attrelid = c.confrelid and ta.attnum = any (c.confkey)
    where
      {rel_schema}
      {rel_table}
      c.contype = 'f'
  ) as relationships
  on (relationships.source_schema = nc.nspname and relationships.source_table_name = c.relname)
  or (relationships.target_table_schema = nc.nspname and relationships.target_table_name = c.relname)
WHERE
  {where_schema}
  {where_ids}
  {where_table}
  c.relkind IN ('r', 'p')
  AND NOT pg_is_other_temp_schema(nc.oid)
  AND (
    pg_has_role(c.relowner, 'USAGE')
    OR has_table_privilege(
      c.oid,
      'SELECT, INSERT, UPDATE, DELETE, TRUNCATE, REFERENCES, TRIGGER'
    )
    OR has_any_column_privilege(c.oid, 'SELECT, INSERT, UPDATE, REFERENCES')
  )
group by
  c.oid,
  c.relname,
  c.relrowsecurity,
  c.relforcerowsecurity,
  c.relreplident,
  nc.nspname,
  pk.primary_keys
{limit}
{offset}
"#,
        pk_schema = pk_schema,
        pk_table = pk_table,
        rel_schema = rel_schema,
        rel_table = rel_table,
        where_schema = where_schema,
        where_ids = where_ids,
        where_table = where_table,
        limit = parts.limit,
        offset = parts.offset,
    )
}

fn columns_sql(parts: &QueryParts) -> String {
    let schema = and_filter("nc.nspname", &parts.schema_filter);
    let table_id = and_filter("c.oid", &parts.ids_filter);
    let table_name = and_filter(
        "nc.nspname || '.' || c.relname",
        &parts.table_identifier_filter,
    );
    format!(
        r#"
SELECT
  c.oid :: int8 AS table_id,
  nc.nspname AS schema,
  c.relname AS table,
  (c.oid || '.' || a.attnum) AS id,
  a.attnum AS ordinal_position,
  a.attname AS name,
  CASE
    WHEN a.atthasdef THEN pg_get_expr(ad.adbin, ad.adrelid)
    ELSE NULL
  END AS default_value,
  CASE
    WHEN t.typtype = 'd' THEN CASE
      WHEN bt.typelem <> 0 :: oid
      AND bt.typlen = -1 THEN 'ARRAY'
      WHEN nbt.nspname = 'pg_catalog' THEN format_type(t.typbasetype, NULL)
      ELSE 'USER-DEFINED'
    END
    ELSE CASE
      WHEN t.typelem <> 0 :: oid
      AND t.typlen = -1 THEN 'ARRAY'
      WHEN nt.nspname = 'pg_catalog' THEN format_type(a.atttypid, NULL)
      ELSE 'USER-DEFINED'
    END
  END AS data_type,
  COALESCE(bt.typname, t.typname) AS format,
  a.attidentity IN ('a', 'd') AS is_identity,
  CASE
    a.attidentity
    WHEN 'a' THEN 'ALWAYS'
    WHEN 'd' THEN 'BY DEFAULT'
    ELSE NULL
  END AS identity_generation,
  a.attgenerated IN ('s') AS is_generated,
  NOT (
    a.attnotnull
    OR t.typtype = 'd' AND t.typnotnull
  ) AS is_nullable,
  (
    c.relkind IN ('r', 'p')
    OR c.relkind IN ('v', 'f') AND pg_column_is_updatable(c.oid, a.attnum, FALSE)
  ) AS is_updatable,
  uniques.table_id IS NOT NULL AS is_unique,
  check_constraints.definition AS "check",
  array_to_json(
    array(
      SELECT
        enumlabel
      FROM
        pg_catalog.pg_enum enums
      WHERE
        enums.enumtypid = coalesce(bt.oid, t.oid)
        OR enums.enumtypid = coalesce(bt.typelem, t.typelem)
      ORDER BY
        enums.enumsortorder
    )
  ) AS enums,
  col_description(c.oid, a.attnum) AS comment
FROM
  pg_attribute a
  LEFT JOIN pg_attrdef ad ON a.attrelid = ad.adrelid
  AND a.attnum = ad.adnum
  JOIN (
    pg_class c
    JOIN pg_namespace nc ON c.relnamespace = nc.oid
  ) ON a.attrelid = c.oid
  JOIN (
    pg_type t
    JOIN pg_namespace nt ON t.typnamespace = nt.oid
  ) ON a.atttypid = t.oid
  LEFT JOIN (
    pg_type bt
    JOIN pg_namespace nbt ON bt.typnamespace = nbt.oid
  ) ON t.typtype = 'd'
  AND t.typbasetype = bt.oid
  LEFT JOIN (
    SELECT DISTINCT ON (table_id, ordinal_position)
      conrelid AS table_id,
      conkey[1] AS ordinal_position
    FROM pg_catalog.pg_constraint
    WHERE contype = 'u' AND cardinality(conkey) = 1
  ) AS uniques ON uniques.table_id = c.oid AND uniques.ordinal_position = a.attnum
  LEFT JOIN (
    SELECT DISTINCT ON (table_id, ordinal_position)
      conrelid AS table_id,
      conkey[1] AS ordinal_position,
      substring(
        pg_get_constraintdef(pg_constraint.oid, true),
        8,
        length(pg_get_constraintdef(pg_constraint.oid, true)) - 8
      ) AS "definition"
    FROM pg_constraint
    WHERE contype = 'c' AND cardinality(conkey) = 1
    ORDER BY table_id, ordinal_position, oid asc
  ) AS check_constraints ON check_constraints.table_id = c.oid AND check_constraints.ordinal_position = a.attnum
WHERE
  {schema}
  {table_id}
  {table_name}
  NOT pg_is_other_temp_schema(nc.oid)
  AND a.attnum > 0
  AND NOT a.attisdropped
  AND (c.relkind IN ('r', 'v', 'm', 'f', 'p'))
  AND (
    pg_has_role(c.relowner, 'USAGE')
    OR has_column_privilege(
      c.oid,
      a.attnum,
      'SELECT, INSERT, UPDATE, REFERENCES'
    )
  )
"#,
        schema = schema,
        table_id = table_id,
        table_name = table_name,
    )
}

fn coalesce_rows(source: &str, filter: &str) -> String {
    format!(
        "COALESCE((SELECT array_agg(row_to_json({source})) FILTER (WHERE {filter}) FROM {source}), '{{}}') AS {source}"
    )
}

fn create_statements(name: &str, schema: &str, comment: Option<&str>) -> Vec<String> {
    let mut statements = vec![format!(
        "CREATE TABLE {}.{} ();",
        ident(schema),
        ident(name)
    )];
    if let Some(comment) = comment {
        statements.push(format!(
            "COMMENT ON TABLE {}.{} IS {};",
            ident(schema),
            ident(name),
            literal(comment)
        ));
    }
    statements
}

fn update_statements(id: i64, old: &TableSnapshot, update: &TableUpdate) -> Vec<String> {
    let alter = format!("ALTER TABLE {}.{}", ident(&old.schema), ident(&old.name));
    let mut statements = Vec::new();
    if let Some(enabled) = update.rls_enabled {
        let verb = if enabled { "ENABLE" } else { "DISABLE" };
        statements.push(format!("{alter} {verb} ROW LEVEL SECURITY;"));
    }
    if let Some(forced) = update.rls_forced {
        let verb = if forced {
            "FORCE ROW LEVEL SECURITY;"
        } else {
            "NO FORCE ROW LEVEL SECURITY;"
        };
        statements.push(format!("{alter} {verb}"));
    }
    if let Some(replica) = &update.replica_identity {
        if matches!(replica, Replica::Index) {
            let index = update
                .replica_identity_index
                .as_deref()
                .unwrap_or("undefined");
            statements.push(format!(
                "{alter} REPLICA IDENTITY USING INDEX {};",
                ident(index)
            ));
        } else {
            statements.push(format!("{alter} REPLICA IDENTITY {};", replica.as_sql()));
        }
    }
    if let Some(keys) = &update.primary_keys {
        if old.primary_key_count != 0 {
            let drop_prefix = literal(&format!("{alter} DROP CONSTRAINT "));
            let id_literal = literal_i64(id);
            statements.push(format!(
                "DO $$\nDECLARE\n  r record;\nBEGIN\n  SELECT conname\n    INTO r\n    FROM pg_constraint\n    WHERE contype = 'p' AND conrelid = {id_literal};\n  EXECUTE {drop_prefix} || quote_ident(r.conname);\nEND\n$$;"
            ));
        }
        if !keys.is_empty() {
            let columns = keys
                .iter()
                .map(|name| ident(name))
                .collect::<Vec<_>>()
                .join(",");
            statements.push(format!("{alter} ADD PRIMARY KEY ({columns});"));
        }
    }
    if let Some(comment) = &update.comment {
        statements.push(format!(
            "COMMENT ON TABLE {}.{} IS {};",
            ident(&old.schema),
            ident(&old.name),
            literal(comment)
        ));
    }
    if let Some(schema) = &update.schema {
        statements.push(format!("{alter} SET SCHEMA {};", ident(schema)));
    }
    if let Some(name) = &update.name {
        if name != &old.name {
            let current_schema = update.schema.as_deref().unwrap_or(old.schema.as_str());
            statements.push(format!(
                "ALTER TABLE {}.{} RENAME TO {};",
                ident(current_schema),
                ident(&old.name),
                ident(name)
            ));
        }
    }
    statements
}

fn drop_statement(schema: &str, name: &str, cascade: bool) -> String {
    let tail = if cascade { "CASCADE" } else { "RESTRICT" };
    format!("DROP TABLE {}.{} {tail};", ident(schema), ident(name))
}

fn parse_list_query(query: Option<&str>) -> Result<ListQuery, String> {
    let pairs = query_pairs(query);
    let include_system_schemas = optional_bool(&pairs, "include_system_schemas")?.unwrap_or(false);
    let included = optional_query_string(&pairs, "included_schemas")?;
    let excluded = optional_query_string(&pairs, "excluded_schemas")?;
    Ok(ListQuery {
        include_system_schemas,
        included_schemas: split_csv(included),
        excluded_schemas: split_csv(excluded),
        limit: optional_integer(&pairs, "limit")?,
        offset: optional_integer(&pairs, "offset")?,
        include_columns: optional_bool(&pairs, "include_columns")?.unwrap_or(true),
    })
}

fn parse_cascade(query: Option<&str>) -> Result<bool, String> {
    Ok(optional_bool(&query_pairs(query), "cascade")?.unwrap_or(false))
}

fn split_csv(value: Option<&str>) -> Vec<String> {
    match value {
        None => Vec::new(),
        Some(value) => value.split(',').map(str::to_string).collect(),
    }
}

fn parse_create(bytes: &[u8]) -> Result<(String, String, Option<String>), String> {
    let map = json_object(bytes)?;
    let name = require_string(&map, "name")?;
    let schema = optional_string(&map, "schema")?.unwrap_or_else(|| "public".to_string());
    let comment = optional_string(&map, "comment")?;
    Ok((name, schema, comment))
}

fn parse_update(bytes: &[u8]) -> Result<TableUpdate, String> {
    let map = json_object(bytes)?;
    Ok(TableUpdate {
        name: optional_string(&map, "name")?,
        schema: optional_string(&map, "schema")?,
        rls_enabled: optional_bool_field(&map, "rls_enabled")?,
        rls_forced: optional_bool_field(&map, "rls_forced")?,
        replica_identity: optional_replica(&map)?,
        replica_identity_index: optional_string(&map, "replica_identity_index")?,
        primary_keys: optional_primary_keys(&map)?,
        comment: optional_string(&map, "comment")?,
    })
}

fn json_object(bytes: &[u8]) -> Result<Map<String, Value>, String> {
    if bytes.is_empty() {
        return Err("body must be object".to_string());
    }
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err("body must be object".to_string()),
        Err(error) => Err(error.to_string()),
    }
}

fn require_string(map: &Map<String, Value>, key: &str) -> Result<String, String> {
    match map.get(key) {
        None => Err(format!("body must have required property '{key}'")),
        Some(Value::String(value)) => Ok(value.clone()),
        Some(_) => Err(format!("body/{key} must be string")),
    }
}

fn optional_string(map: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("body/{key} must be string")),
    }
}

fn optional_bool_field(map: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!("body/{key} must be boolean")),
    }
}

fn optional_replica(map: &Map<String, Value>) -> Result<Option<Replica>, String> {
    match map.get("replica_identity") {
        None => Ok(None),
        Some(Value::String(value)) => match value.as_str() {
            "DEFAULT" => Ok(Some(Replica::Default)),
            "INDEX" => Ok(Some(Replica::Index)),
            "FULL" => Ok(Some(Replica::Full)),
            "NOTHING" => Ok(Some(Replica::Nothing)),
            _ => Err("body/replica_identity must match a schema in anyOf".to_string()),
        },
        Some(_) => Err("body/replica_identity must match a schema in anyOf".to_string()),
    }
}

fn optional_primary_keys(map: &Map<String, Value>) -> Result<Option<Vec<String>>, String> {
    match map.get("primary_keys") {
        None => Ok(None),
        Some(Value::Array(items)) => {
            let mut names = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let Value::Object(object) = item else {
                    return Err(format!("body/primary_keys/{index} must be object"));
                };
                match object.get("name") {
                    None => {
                        return Err(format!(
                            "body/primary_keys/{index} must have required property 'name'"
                        ));
                    }
                    Some(Value::String(name)) => names.push(name.clone()),
                    Some(_) => {
                        return Err(format!("body/primary_keys/{index}/name must be string"));
                    }
                }
            }
            Ok(Some(names))
        }
        Some(_) => Err("body/primary_keys must be array".to_string()),
    }
}

fn query_pairs(query: Option<&str>) -> Vec<(String, String)> {
    let Some(query) = query else {
        return Vec::new();
    };
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            (percent_decode(key), percent_decode(value))
        })
        .collect()
}

fn optional_bool(pairs: &[(String, String)], name: &str) -> Result<Option<bool>, String> {
    match one(pairs, name, "boolean")? {
        None => Ok(None),
        Some(raw) => parse_bool(raw)
            .map(Some)
            .ok_or_else(|| format!("querystring/{name} must be boolean")),
    }
}

fn optional_integer(pairs: &[(String, String)], name: &str) -> Result<Option<i64>, String> {
    match one(pairs, name, "integer")? {
        None => Ok(None),
        Some(raw) => parse_integer(raw)
            .map(Some)
            .ok_or_else(|| format!("querystring/{name} must be integer")),
    }
}

fn optional_query_string<'a>(
    pairs: &'a [(String, String)],
    name: &str,
) -> Result<Option<&'a str>, String> {
    one(pairs, name, "string")
}

fn one<'a>(
    pairs: &'a [(String, String)],
    name: &str,
    expected: &str,
) -> Result<Option<&'a str>, String> {
    let mut found = None;
    for (key, value) in pairs {
        if key == name {
            if found.is_some() {
                return Err(format!("querystring/{name} must be {expected}"));
            }
            found = Some(value.as_str());
        }
    }
    Ok(found)
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

fn parse_integer(raw: &str) -> Option<i64> {
    let value: i64 = raw.parse().ok()?;
    if value.unsigned_abs() > MAX_SAFE_INTEGER {
        return None;
    }
    Some(value)
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
                let hex = &input[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
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

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::Router;
    use tower::ServiceExt;

    use super::*;
    use crate::{router_with_state, MetaState};

    const SECRET: &str = "super-secret-jwt-token-with-at-least-32-characters-long";

    fn key(role: &str) -> String {
        megabase_core::Hs256::new(SECRET)
            .unwrap()
            .sign(&serde_json::json!({ "role": role }))
            .unwrap()
            .as_str()
            .to_string()
    }

    fn app(pool: Option<sqlx::PgPool>) -> Router {
        router_with_state(MetaState::new(
            pool,
            Some(megabase_core::Hs256::new(SECRET).unwrap()),
        ))
    }

    fn lazy_pool() -> sqlx::PgPool {
        sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://postgres@127.0.0.1:1/postgres")
            .expect("lazy pool")
    }

    async fn call(
        app: Router,
        method: &str,
        path: &str,
        apikey: Option<&str>,
        body: Option<&[u8]>,
    ) -> (StatusCode, Value) {
        let mut request = axum::http::Request::builder().method(method).uri(path);
        if let Some(apikey) = apikey {
            request = request.header("apikey", apikey);
        }
        if body.is_some() {
            request = request.header("content-type", "application/json");
        }
        let response = app
            .oneshot(
                request
                    .body(Body::from(body.unwrap_or_default().to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let parsed = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, parsed)
    }

    fn sample_update() -> TableUpdate {
        TableUpdate {
            name: Some("T".to_string()),
            schema: Some("private".to_string()),
            rls_enabled: Some(true),
            rls_forced: Some(false),
            replica_identity: Some(Replica::Full),
            replica_identity_index: None,
            primary_keys: Some(vec!["c".to_string(), "cc".to_string()]),
            comment: Some("it's".to_string()),
        }
    }

    #[test]
    fn list_sql_excludes_system_schemas_and_includes_columns() {
        let sql = list_json_sql(&ListQuery {
            include_system_schemas: false,
            included_schemas: Vec::new(),
            excluded_schemas: Vec::new(),
            limit: Some(10),
            offset: Some(0),
            include_columns: true,
        });
        assert!(sql.contains("NOT IN ('information_schema','pg_catalog','pg_toast')"));
        assert!(sql.contains("columns as"));
        assert!(sql.contains("c.relkind IN ('r', 'p')"));
        assert!(sql.contains("pg_has_role"));
        assert!(sql.contains("limit '10'"));
        assert!(!sql.contains("offset"));
        assert!(sql.contains("jsonb_agg(to_jsonb(tables_row))"));
    }

    #[test]
    fn list_sql_honors_includes_and_can_omit_columns() {
        let sql = list_json_sql(&ListQuery {
            include_system_schemas: true,
            included_schemas: vec!["public".to_string(), "a'b".to_string()],
            excluded_schemas: vec!["pg_catalog".to_string()],
            limit: Some(0),
            offset: Some(5),
            include_columns: false,
        });
        assert!(sql.contains("IN ('public','a''b')"));
        assert!(!sql.contains("pg_catalog"));
        assert!(!sql.contains("columns as"));
        assert!(!sql.contains("limit"));
        assert!(sql.contains("offset '5'"));
    }

    #[test]
    fn retrieve_sql_filters_by_id_without_a_schema_list() {
        let sql = retrieve_id_sql(7);
        assert!(sql.contains("c.oid IN ('7')"));
        assert!(sql.contains("columns as"));
        assert!(!sql.contains("information_schema"));
    }

    #[test]
    fn create_and_drop_statements_quote_names_and_comments() {
        assert_eq!(
            create_statements("todos", "public", Some("'")),
            vec![
                "CREATE TABLE public.todos ();".to_string(),
                "COMMENT ON TABLE public.todos IS '''';".to_string(),
            ]
        );
        assert_eq!(
            create_statements("user", "Public", None),
            vec!["CREATE TABLE \"Public\".\"user\" ();".to_string()]
        );
        assert_eq!(
            drop_statement("public", "todos", false),
            "DROP TABLE public.todos RESTRICT;"
        );
        assert_eq!(
            drop_statement("public", "user", true),
            "DROP TABLE public.\"user\" CASCADE;"
        );
    }

    #[test]
    fn update_statements_follow_the_pinned_order() {
        let old = TableSnapshot {
            schema: "public".to_string(),
            name: "t".to_string(),
            primary_key_count: 1,
        };
        let statements = update_statements(5, &old, &sample_update());
        assert_eq!(
            statements[0],
            "ALTER TABLE public.t ENABLE ROW LEVEL SECURITY;"
        );
        assert_eq!(
            statements[1],
            "ALTER TABLE public.t NO FORCE ROW LEVEL SECURITY;"
        );
        assert_eq!(statements[2], "ALTER TABLE public.t REPLICA IDENTITY FULL;");
        assert!(statements[3].contains("conrelid = '5'"));
        assert!(statements[3].contains("EXECUTE 'ALTER TABLE public.t DROP CONSTRAINT '"));
        assert_eq!(
            statements[4],
            "ALTER TABLE public.t ADD PRIMARY KEY (c,cc);"
        );
        assert_eq!(statements[5], "COMMENT ON TABLE public.t IS 'it''s';");
        assert_eq!(statements[6], "ALTER TABLE public.t SET SCHEMA private;");
        assert_eq!(statements[7], "ALTER TABLE private.t RENAME TO \"T\";");
        assert_eq!(statements.len(), 8);
    }

    #[test]
    fn replica_index_uses_ident_and_undefined_when_omitted() {
        let old = TableSnapshot {
            schema: "public".to_string(),
            name: "t".to_string(),
            primary_key_count: 0,
        };
        let missing = TableUpdate {
            replica_identity: Some(Replica::Index),
            ..empty_update()
        };
        assert_eq!(
            update_statements(1, &old, &missing),
            vec!["ALTER TABLE public.t REPLICA IDENTITY USING INDEX undefined;"]
        );
        let named = TableUpdate {
            replica_identity: Some(Replica::Index),
            replica_identity_index: Some("MyIdx".to_string()),
            ..empty_update()
        };
        assert_eq!(
            update_statements(1, &old, &named),
            vec!["ALTER TABLE public.t REPLICA IDENTITY USING INDEX \"MyIdx\";"]
        );
    }

    fn empty_update() -> TableUpdate {
        TableUpdate {
            name: None,
            schema: None,
            rls_enabled: None,
            rls_forced: None,
            replica_identity: None,
            replica_identity_index: None,
            primary_keys: None,
            comment: None,
        }
    }

    #[test]
    fn unchanged_name_skips_rename_and_an_empty_key_list_only_drops() {
        let old = TableSnapshot {
            schema: "public".to_string(),
            name: "t".to_string(),
            primary_key_count: 2,
        };
        let update = TableUpdate {
            name: Some("t".to_string()),
            primary_keys: Some(Vec::new()),
            ..empty_update()
        };
        let statements = update_statements(9, &old, &update);
        assert_eq!(statements.len(), 1);
        assert!(statements[0].contains("DROP CONSTRAINT"));
        assert!(!statements[0].contains("ADD PRIMARY KEY"));
    }

    #[tokio::test]
    async fn tables_without_a_database_use_the_connection_error() {
        let admin = key("service_role");
        let (status, body) = call(app(None), "GET", "/pg/tables", Some(&admin), None).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["message"], "failed to get upstream connection details");
        let (status, body) = call(
            app(None),
            "POST",
            "/pg/tables",
            Some(&admin),
            Some(br#"{"name":1}"#),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["message"], "failed to get upstream connection details");
    }

    #[tokio::test]
    async fn non_admin_keys_are_forbidden_on_tables() {
        let (status, body) = call(app(None), "GET", "/pg/tables", Some(&key("anon")), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["message"], "You cannot consume this service");
        let (status, _) = call(app(None), "DELETE", "/pg/tables/1", None, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn a_non_numeric_id_stays_501() {
        let (status, body) = call(
            app(None),
            "GET",
            "/pg/tables/abc",
            Some(&key("service_role")),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "GET /pg/tables/abc");
        let (status, body) = call(
            app(Some(lazy_pool())),
            "PATCH",
            "/pg/tables/abc",
            Some(&key("service_role")),
            Some(b"{}"),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "PATCH /pg/tables/abc");
    }

    #[tokio::test]
    async fn id_zero_is_invalid_parameters_and_validation_precedes_it() {
        let admin = key("service_role");
        let (status, body) = call(
            app(Some(lazy_pool())),
            "GET",
            "/pg/tables/0",
            Some(&admin),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], INVALID_PARAMETERS);
        let (status, body) = call(
            app(Some(lazy_pool())),
            "DELETE",
            "/pg/tables/00?cascade=false",
            Some(&admin),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], INVALID_PARAMETERS);
        let (status, body) = call(
            app(Some(lazy_pool())),
            "PATCH",
            "/pg/tables/0",
            Some(&admin),
            Some(br#"{"rls_enabled":"yes"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "body/rls_enabled must be boolean");
    }

    #[tokio::test]
    async fn schema_failures_use_the_fastify_error_text() {
        let admin = key("service_role");
        let pool = Some(lazy_pool());
        let (status, body) = call(
            app(pool.clone()),
            "POST",
            "/pg/tables",
            Some(&admin),
            Some(b"{}"),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "body must have required property 'name'");
        let (status, body) = call(
            app(pool.clone()),
            "POST",
            "/pg/tables",
            Some(&admin),
            Some(br#"{"name":"t","schema":1}"#),
        )
        .await;
        assert_eq!(body["error"], "body/schema must be string");
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let (status, body) = call(
            app(pool.clone()),
            "GET",
            "/pg/tables?limit=no&include_columns=false",
            Some(&admin),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "querystring/limit must be integer");
        let (status, body) = call(
            app(pool.clone()),
            "PATCH",
            "/pg/tables/1",
            Some(&admin),
            Some(br#"{"replica_identity":"MAYBE"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            body["error"],
            "body/replica_identity must match a schema in anyOf"
        );
        let (status, body) = call(
            app(pool.clone()),
            "PATCH",
            "/pg/tables/1",
            Some(&admin),
            Some(br#"{"primary_keys":[{"name":1}]}"#),
        )
        .await;
        assert_eq!(body["error"], "body/primary_keys/0/name must be string");
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let (status, body) = call(
            app(pool),
            "DELETE",
            "/pg/tables/1?cascade=yes",
            Some(&admin),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "querystring/cascade must be boolean");
        let huge = "9".repeat(20);
        let (status, body) = call(
            app(Some(lazy_pool())),
            "GET",
            &format!("/pg/tables/{huge}"),
            Some(&admin),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "params/id must be integer");
    }

    #[tokio::test]
    async fn other_table_methods_stay_501() {
        let (status, body) = call(
            app(None),
            "PUT",
            "/pg/tables",
            Some(&key("service_role")),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "PUT /pg/tables");
    }
}
