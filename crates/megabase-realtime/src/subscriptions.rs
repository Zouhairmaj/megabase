// Ported from supabase/realtime lib/extensions/postgres_cdc_rls/subscriptions.ex (Apache-2.0), pin v2.143.3.

//! Postgres Changes subscriptions: parse `postgres_changes` params and manage
//! rows in `realtime.subscription`.
//!
//! Error `Display` strings are the exact `message` the upstream channel pushes
//! in the `system` event (`realtime_channel.ex` `push_system_message/5`):
//! fatal reasons push their binary as is, retryable ones push `inspect/1` of
//! the `{:error, reason}` tuple.

use std::time::Duration;

use serde_json::Value;
use sqlx::postgres::PgDatabaseError;
use sqlx::PgPool;

/// All supported filter operators (`@filter_types`).
const FILTER_TYPES: [&str; 13] = [
    "eq",
    "neq",
    "lt",
    "lte",
    "gt",
    "gte",
    "in",
    "like",
    "ilike",
    "is",
    "match",
    "imatch",
    "isdistinct",
];

/// The transaction deadline (`@transaction_timeout`): statement_timeout (5s) <
/// this (10s) < RPC (15s).
const TRANSACTION_TIMEOUT: Duration = Duration::from_millis(10_000);

/// `cdc_rls.ex` `@database_timeout_reason`.
const DATABASE_TIMEOUT_REASON: &str = "Too many database timeouts";

/// Width used by `inspect(value, pretty: true)` (`Inspect.Opts` default).
const PRETTY_WIDTH: usize = 80;

/// Item limit used by `inspect/2` (`Inspect.Opts` default `limit: 50`).
const INSPECT_LIMIT: usize = 50;

const NO_PARAMS: &str =
    "No subscription params provided. Please provide at least a `schema` or `table` to subscribe to: ";

const INSERT_SQL: &str = "with sub_tables as (
        select
        rr.entity
        from
        pg_publication_tables pub,
        lateral (
        select
        format('%I.%I', pub.schemaname, pub.tablename)::regclass entity
        ) rr
        where
        pub.pubname = $1
        and pub.schemaname like (case $2 when '*' then '%' else $2 end) escape ''
        and pub.tablename like (case $3 when '*' then '%' else $3 end) escape ''
     )
     insert into realtime.subscription as x(
        subscription_id,
        entity,
        filters,
        claims,
        action_filter,
        selected_columns
      )
      select
        $4::text::uuid,
        sub_tables.entity,
        -- Build the realtime.user_defined_filter[] server-side from primitive type arrays
        -- instead of binding a list of composite tuples. Postgrex caches the composite type's
        -- field count per connection at bootstrap and never refreshes it, so a long-lived
        -- connection whose cache predates an ALTER TYPE on user_defined_filter would otherwise
        -- encode against a stale arity and crash with :badarg. Constructing the rows here keeps
        -- the arity resolved by the server's current catalog.
        (
          select coalesce(
            array_agg(row(c, o::realtime.equality_op, v, n)::realtime.user_defined_filter),
            '{}'::realtime.user_defined_filter[]
          )
          from unnest($6::text[], $7::text[], $8::text[], $9::boolean[]) as f(c, o, v, n)
        ),
        $5,
        $10,
        $11
      from
        sub_tables
        on conflict
        -- coalesce needed: NULL != NULL in unique constraints; NULL selected_columns means all columns
        (subscription_id, entity, filters, action_filter, coalesce(selected_columns, '{}'))
        do update set
        claims = excluded.claims,
        created_at = now()
      returning
         id";

const DELETE_MULTI_SQL: &str =
    "delete from realtime.subscription where subscription_id = ANY($1::uuid[])";

/// One filter written to `realtime.subscription.filters` (`@type filter`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Filter {
    /// The column name, left of `=`.
    pub column: String,
    /// The operator, one of `FILTER_TYPES`.
    pub op: String,
    /// The value; an `in` list is rewritten to Postgres array syntax `{a,b}`.
    pub value: String,
    /// Whether the operator carried the `not.` prefix.
    pub negate: bool,
}

/// One parsed `postgres_changes` entry (`parse_subscription_params/1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubscriptionParams {
    /// `INSERT`, `UPDATE`, `DELETE` or `*`.
    pub action_filter: String,
    /// The schema pattern; `*` matches every schema.
    pub schema: String,
    /// The table pattern; `*` matches every table.
    pub table: String,
    /// AND-combined row filters.
    pub filters: Vec<Filter>,
    /// `None` means all columns; `Some(vec![])` means primary keys only.
    pub selected_columns: Option<Vec<String>>,
}

/// Why `postgres_changes` params could not be parsed (`{:error, binary}`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ParamsError {
    /// The `filter` string is malformed; the field is the upstream reason.
    #[error("Error parsing `filter` params: {0}")]
    Filter(String),
    /// `select` is a string instead of a list.
    #[error("Error parsing `select` params: expected a list of column name strings, e.g. select: [\"col1\", \"col2\"]")]
    Select,
    /// `select` was combined with a wildcard schema or table.
    #[error("Column selection is not supported for wildcard subscriptions. Provide an explicit schema and table name.")]
    WildcardSelect,
    /// Neither a usable `schema` nor `table`, and the params carry a token.
    #[error("No subscription params provided. Please provide at least a `schema` or `table` to subscribe to: <redacted>")]
    MissingRedacted,
    /// Neither a usable `schema` nor `table`; the field is `inspect(params)`.
    #[error("{NO_PARAMS}{0}")]
    Missing(String),
}

/// One subscription to insert (`subscription_list` entry).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NewSubscription {
    /// The `subscription_id`.
    pub id: uuid::Uuid,
    /// The JWT claims stored with the subscription.
    pub claims: Value,
    /// The parsed params.
    pub params: SubscriptionParams,
}

/// Why `create` failed; `Display` is the client-visible `system` message.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CreateError {
    /// `{:malformed_subscription_params, reason}` from `cdc_rls.ex`.
    #[error(transparent)]
    MalformedSubscriptionParams(#[from] ParamsError),
    /// `{:subscription_insert_failed, msg}`.
    #[error("{0}")]
    SubscriptionInsertFailed(String),
    /// A statement timeout, a dropped connection or the transaction deadline;
    /// `cdc_rls.ex` turns all of these into `{:error, "Too many database timeouts"}`.
    #[error("{{:error, {}}}", inspect_str(DATABASE_TIMEOUT_REASON))]
    DatabaseTimeout(#[source] Option<sqlx::Error>),
    /// Any other database failure outside the insert (begin or commit).
    #[error("{{:error, {}}}", inspect_str(&exception_message(.0)))]
    Database(#[source] sqlx::Error),
}

impl CreateError {
    /// Whether the channel should stop retrying (`@postgres_subscribe_fatal_reasons`).
    pub(crate) fn fatal(&self) -> bool {
        matches!(
            self,
            Self::MalformedSubscriptionParams(_) | Self::SubscriptionInsertFailed(_)
        )
    }
}

/// Insert failure for `claims_role`, the `NOT NULL` generated column
/// `realtime.to_regrole((claims ->> 'role'))` (`role_name::regrole`).
///
/// A name that is not in `pg_roles` is the cast's `42704` (`role "…" does
/// not exist`). A missing or JSON-null `role` makes the expression NULL,
/// which is `23502` on `claims_role`. The text matches
/// [`exception_message`] so the client sees the same `Exception:` tail as
/// a rejected insert, and the error is fatal so the channel does not retry.
pub(crate) fn undefined_claims_role(role: &str, params: &SubscriptionParams) -> CreateError {
    insert_exception(
        "42704",
        &format!("role \"{role}\" does not exist"),
        params,
        &[],
    )
}

/// `23502` when `claims ->> 'role'` is NULL.
pub(crate) fn null_claims_role(params: &SubscriptionParams) -> CreateError {
    insert_exception(
        "23502",
        "null value in column \"claims_role\" of relation \"subscription\" violates not-null constraint",
        params,
        &[("column", "claims_role"), ("table", "subscription")],
    )
}

/// The name exists, but it is not a configured API role, so the row is not
/// inserted. The SQLSTATE is `42501`.
pub(crate) fn unconfigured_existing_role(role: &str, params: &SubscriptionParams) -> CreateError {
    insert_exception(
        "42501",
        &format!("permission denied to set role \"{role}\""),
        params,
        &[],
    )
}

fn insert_exception(
    sqlstate: &str,
    message: &str,
    params: &SubscriptionParams,
    fields: &[(&str, &str)],
) -> CreateError {
    let mut exception = format!(
        "ERROR {} ({}) {}",
        sqlstate,
        condition_name(sqlstate),
        message
    );
    if !fields.is_empty() {
        exception.push('\n');
        for (key, value) in fields {
            exception.push_str("\n    ");
            exception.push_str(key);
            exception.push_str(": ");
            exception.push_str(value);
        }
    }
    CreateError::SubscriptionInsertFailed(format!(
        "Unable to subscribe to changes with given parameters. An exception happened so please check your connect parameters: [{}]. Exception: {exception}",
        params_to_log(params),
    ))
}

/// Parses one `postgres_changes` params object (`parse_subscription_params/1`).
///
/// # Errors
///
/// Returns a [`ParamsError`] whose `Display` is the upstream error message
/// when the filter or select is malformed, or when neither a string `schema`
/// nor `table` is given.
pub(crate) fn parse_subscription_params(params: &Value) -> Result<SubscriptionParams, ParamsError> {
    let action_filter = action_filter(params);
    let selected_columns = parse_select(params)?;
    let map = params.as_object();
    let get = |key: &str| map.and_then(|m| m.get(key));
    let has = |key: &str| map.is_some_and(|m| m.contains_key(key));
    let schema = get("schema").and_then(Value::as_str);
    let table = get("table").and_then(Value::as_str);
    let filter = get("filter").and_then(Value::as_str);

    match (schema, table, filter) {
        (Some(schema), Some(table), Some(filter)) => {
            let filters = parse_filters(filter).map_err(ParamsError::Filter)?;
            ok_params(action_filter, schema, table, filters, selected_columns)
        }
        (Some(schema), Some(table), _) if !has("filter") => {
            ok_params(action_filter, schema, table, Vec::new(), selected_columns)
        }
        (Some(schema), _, _) if !has("table") && !has("filter") => {
            ok_params(action_filter, schema, "*", Vec::new(), selected_columns)
        }
        (_, Some(table), _) if !has("schema") && !has("filter") => {
            ok_params(action_filter, "public", table, Vec::new(), selected_columns)
        }
        _ if has("user_token") || has("auth_token") => Err(ParamsError::MissingRedacted),
        _ => Err(ParamsError::Missing(inspect_json(params))),
    }
}

fn parse_select(params: &Value) -> Result<Option<Vec<String>>, ParamsError> {
    match params.get("select") {
        // An empty list means "primary keys only" ('{}'), distinct from nil.
        Some(Value::Array(cols)) => Ok(Some(
            cols.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        )),
        Some(Value::String(_)) => Err(ParamsError::Select),
        _ => Ok(None),
    }
}

fn ok_params(
    action_filter: &str,
    schema: &str,
    table: &str,
    filters: Vec<Filter>,
    selected_columns: Option<Vec<String>>,
) -> Result<SubscriptionParams, ParamsError> {
    if selected_columns.is_some() && (schema == "*" || table == "*") {
        return Err(ParamsError::WildcardSelect);
    }
    Ok(SubscriptionParams {
        action_filter: action_filter.to_owned(),
        schema: schema.to_owned(),
        table: table.to_owned(),
        filters,
        selected_columns,
    })
}

fn action_filter(params: &Value) -> &'static str {
    match params.get("event").and_then(Value::as_str) {
        Some(event) => match event.to_uppercase().as_str() {
            "INSERT" => "INSERT",
            "UPDATE" => "UPDATE",
            "DELETE" => "DELETE",
            _ => "*",
        },
        None => "*",
    }
}

fn parse_filters(filter: &str) -> Result<Vec<Filter>, String> {
    match filter.trim() {
        "" => Ok(Vec::new()),
        trimmed => split_top_level(trimmed)
            .into_iter()
            .map(parse_segment)
            .collect(),
    }
}

/// Splits on commas outside parentheses and double-quoted values.
fn split_top_level(filter: &str) -> Vec<&str> {
    let bytes = filter.as_bytes();
    let mut segments = Vec::new();
    let mut depth = 0usize;
    let mut quoted = false;
    let mut prev = 0u8;
    let mut start = 0usize;
    let mut i = 0usize;
    while let Some(&c) = bytes.get(i) {
        if quoted {
            match (c, bytes.get(i + 1)) {
                (b'\\', Some(&next)) => {
                    prev = next;
                    i += 2;
                    continue;
                }
                (b'"', _) => {
                    quoted = false;
                    prev = b'"';
                }
                _ => prev = c,
            }
        } else {
            match c {
                b'"' if matches!(prev, b'.' | b'(' | b',') => {
                    quoted = true;
                    prev = b'"';
                }
                b'(' => {
                    depth += 1;
                    prev = c;
                }
                b')' => {
                    depth = depth.saturating_sub(1);
                    prev = c;
                }
                b',' if depth == 0 => {
                    // `,` is ASCII, so both slice bounds are char boundaries.
                    segments.push(filter.get(start..i).unwrap_or_default());
                    start = i + 1;
                    prev = 0;
                }
                _ => prev = c,
            }
        }
        i += 1;
    }
    segments.push(filter.get(start..).unwrap_or_default());
    segments
}

fn parse_segment(segment: &str) -> Result<Filter, String> {
    let trimmed = segment.trim();
    if trimmed.is_empty() {
        return Err("filter must not contain empty segments (check for extra commas)".to_owned());
    }
    let Some((column, rest)) = trimmed.split_once('=') else {
        return Err(inspect_str_list(&[trimmed]));
    };
    let (negate, op_value) = match rest.strip_prefix("not.") {
        Some(op_value) => (true, op_value),
        None => (false, rest),
    };
    let Some((filter_type, value)) = op_value.split_once('.') else {
        return Err(inspect_str_list(&[op_value]));
    };
    if !FILTER_TYPES.contains(&filter_type) {
        return Err(inspect_str_list(&[filter_type, value]));
    }
    Ok(Filter {
        column: column.to_owned(),
        op: filter_type.to_owned(),
        value: format_filter_value(filter_type, value)?,
        negate,
    })
}

fn format_filter_value(filter_type: &str, value: &str) -> Result<String, String> {
    if filter_type == "in" {
        return match value.strip_prefix('(').and_then(|v| v.strip_suffix(')')) {
            Some(inner) => Ok(format!("{{{inner}}}")),
            None => Err("`in` filter value must be wrapped by parentheses".to_owned()),
        };
    }
    Ok(unquote_value(value))
}

/// Strips one pair of surrounding double quotes and backslash escapes; any
/// malformed quoting keeps the value literal (PostgREST behaviour).
fn unquote_value(value: &str) -> String {
    let Some(rest) = value.strip_prefix('"') else {
        return value.to_owned();
    };
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    loop {
        match (bytes.get(i), bytes.get(i + 1)) {
            (Some(b'"'), None) => {
                return String::from_utf8(out).unwrap_or_else(|_| value.to_owned());
            }
            (Some(b'"'), Some(_)) | (None, _) => return value.to_owned(),
            (Some(b'\\'), Some(&next)) => {
                out.push(next);
                i += 2;
            }
            (Some(&c), _) => {
                out.push(c);
                i += 1;
            }
        }
    }
}

/// Inserts every subscription inside one transaction (`Subscriptions.create/5`).
///
/// # Errors
///
/// Rolls back and returns [`CreateError::SubscriptionInsertFailed`] when an
/// insert matches no publication table or Postgres rejects it,
/// [`CreateError::DatabaseTimeout`] on a statement timeout, a connection
/// failure or the 10 second transaction deadline, and
/// [`CreateError::Database`] when begin or commit fails otherwise.
pub(crate) async fn create(
    pool: &PgPool,
    publication: &str,
    subscriptions: &[NewSubscription],
) -> Result<(), CreateError> {
    match tokio::time::timeout(
        TRANSACTION_TIMEOUT,
        insert_all(pool, publication, subscriptions),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(CreateError::DatabaseTimeout(None)),
    }
}

async fn insert_all(
    pool: &PgPool,
    publication: &str,
    subscriptions: &[NewSubscription],
) -> Result<(), CreateError> {
    let mut tx = pool.begin().await.map_err(outer_error)?;
    for sub in subscriptions {
        let params = &sub.params;
        let columns: Vec<&str> = params.filters.iter().map(|f| f.column.as_str()).collect();
        let ops: Vec<&str> = params.filters.iter().map(|f| f.op.as_str()).collect();
        let values: Vec<&str> = params.filters.iter().map(|f| f.value.as_str()).collect();
        let negates: Vec<bool> = params.filters.iter().map(|f| f.negate).collect();
        let result = sqlx::query(INSERT_SQL)
            .bind(publication)
            .bind(params.schema.as_str())
            .bind(params.table.as_str())
            .bind(sub.id.to_string())
            .bind(sqlx::types::Json(&sub.claims))
            .bind(columns)
            .bind(ops)
            .bind(values)
            .bind(negates)
            .bind(params.action_filter.as_str())
            .bind(params.selected_columns.as_deref())
            .execute(&mut *tx)
            .await;
        let failure = match result {
            Ok(done) if done.rows_affected() > 0 => continue,
            Ok(_) => CreateError::SubscriptionInsertFailed(format!(
                "Unable to subscribe to changes with given parameters. Please check Realtime is enabled for the given connect parameters: [{}]",
                params_to_log(params)
            )),
            Err(error) if database_timeout(&error) => CreateError::DatabaseTimeout(Some(error)),
            Err(error) => CreateError::SubscriptionInsertFailed(format!(
                "Unable to subscribe to changes with given parameters. An exception happened so please check your connect parameters: [{}]. Exception: {}",
                params_to_log(params),
                exception_message(&error)
            )),
        };
        // The rollback outcome does not change the reported reason.
        let _ = tx.rollback().await;
        return Err(failure);
    }
    tx.commit().await.map_err(outer_error)
}

/// Maps a begin/commit failure: connection errors are timeouts upstream
/// (`rescue DBConnection.ConnectionError`, `catch :exit`).
fn outer_error(error: sqlx::Error) -> CreateError {
    if database_timeout(&error) {
        CreateError::DatabaseTimeout(Some(error))
    } else {
        CreateError::Database(error)
    }
}

/// `database_timeout?/1`: a `query_canceled` (57014) or a lost connection.
fn database_timeout(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(db) => db.code().as_deref() == Some("57014"),
        sqlx::Error::Io(_)
        | sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::WorkerCrashed => true,
        _ => false,
    }
}

/// Deletes subscriptions by `subscription_id` (`Subscriptions.delete_multi/2`).
///
/// # Errors
///
/// Returns the database error when the delete fails.
pub(crate) async fn delete(pool: &PgPool, ids: &[uuid::Uuid]) -> Result<(), sqlx::Error> {
    sqlx::query(DELETE_MULTI_SQL)
        .bind(ids)
        .execute(pool)
        .await
        .map(drop)
}

/// `params_to_log/1`.
fn params_to_log(params: &SubscriptionParams) -> String {
    let select = match &params.selected_columns {
        Some(columns) => pretty_flex_strings(columns),
        None => "nil".to_owned(),
    };
    format!(
        "event: {}, schema: {}, table: {}, filters: {}, select: {}",
        params.action_filter,
        params.schema,
        params.table,
        pretty_filters(&params.filters),
        select
    )
}

/// `Exception.message/1` for the error a query returned.
fn exception_message(error: &sqlx::Error) -> String {
    let Some(pg) = error
        .as_database_error()
        .and_then(|db| db.try_downcast_ref::<PgDatabaseError>())
    else {
        return error.to_string();
    };
    // Postgrex.Error.message/1: severity, code, (condition name), message,
    // then column/constraint/hint/table metadata and the detail.
    let mut out = format!(
        "{} {} ({}) {}",
        format!("{:?}", pg.severity()).to_uppercase(),
        pg.code(),
        condition_name(pg.code()),
        pg.message()
    );
    let metadata: Vec<(&str, &str)> = [
        ("column", pg.column()),
        ("constraint", pg.constraint()),
        ("hint", pg.hint()),
        ("table", pg.table()),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.map(|value| (key, value)))
    .collect();
    if !metadata.is_empty() {
        out.push('\n');
        for (key, value) in metadata {
            out.push_str("\n    ");
            out.push_str(key);
            out.push_str(": ");
            out.push_str(value);
        }
    }
    if let Some(detail) = pg.detail() {
        out.push_str("\n\n");
        out.push_str(detail);
    }
    out
}

/// `Postgrex.ErrorCode.code_to_name/1` for the common SQLSTATEs; an unknown
/// code is `nil`, as `Atom.to_string(nil)` renders it.
fn condition_name(code: &str) -> &'static str {
    match code {
        "08000" => "connection_exception",
        "08003" => "connection_does_not_exist",
        "08006" => "connection_failure",
        "0A000" => "feature_not_supported",
        "21000" => "cardinality_violation",
        "22000" => "data_exception",
        "22001" => "string_data_right_truncation",
        "22003" => "numeric_value_out_of_range",
        "22004" => "null_value_not_allowed",
        "22007" => "invalid_datetime_format",
        "22008" => "datetime_field_overflow",
        "22012" => "division_by_zero",
        "22023" => "invalid_parameter_value",
        "22025" => "invalid_escape_sequence",
        "2202E" => "array_subscript_error",
        "22P02" => "invalid_text_representation",
        "22P05" => "untranslatable_character",
        "23000" => "integrity_constraint_violation",
        "23502" => "not_null_violation",
        "23503" => "foreign_key_violation",
        "23505" => "unique_violation",
        "23514" => "check_violation",
        "23P01" => "exclusion_violation",
        "25000" => "invalid_transaction_state",
        "25P02" => "in_failed_sql_transaction",
        "28000" => "invalid_authorization_specification",
        "2BP01" => "dependent_objects_still_exist",
        "40001" => "serialization_failure",
        "40P01" => "deadlock_detected",
        "42000" => "syntax_error_or_access_rule_violation",
        "42501" => "insufficient_privilege",
        "42601" => "syntax_error",
        "42703" => "undefined_column",
        "42704" => "undefined_object",
        "42804" => "datatype_mismatch",
        "42809" => "wrong_object_type",
        "42883" => "undefined_function",
        "42P01" => "undefined_table",
        "42P02" => "undefined_parameter",
        "3F000" => "invalid_schema_name",
        "53000" => "insufficient_resources",
        "53100" => "disk_full",
        "53200" => "out_of_memory",
        "53300" => "too_many_connections",
        "54000" => "program_limit_exceeded",
        "55P03" => "lock_not_available",
        "57014" => "query_canceled",
        "57P01" => "admin_shutdown",
        "P0000" => "plpgsql_error",
        "P0001" => "raise_exception",
        "P0002" => "no_data_found",
        "P0003" => "too_many_rows",
        "P0004" => "assert_failure",
        "XX000" => "internal_error",
        _ => "nil",
    }
}

/// `inspect(filters, pretty: true)`: one line when it fits in 80 columns,
/// else one tuple per line indented by two.
fn pretty_filters(filters: &[Filter]) -> String {
    let mut items: Vec<String> = filters
        .iter()
        .take(INSPECT_LIMIT)
        .map(|f| {
            format!(
                "{{{}, {}, {}, {}}}",
                inspect_str(&f.column),
                inspect_str(&f.op),
                inspect_str(&f.value),
                f.negate
            )
        })
        .collect();
    if filters.len() > INSPECT_LIMIT {
        items.push("...".to_owned());
    }
    let flat = format!("[{}]", items.join(", "));
    if flat.len() <= PRETTY_WIDTH {
        return flat;
    }
    format!("[\n  {}\n]", items.join(",\n  "))
}

/// `inspect(strings, pretty: true)` for a list of simple items (flex breaks).
fn pretty_flex_strings(values: &[String]) -> String {
    let mut items: Vec<String> = values
        .iter()
        .take(INSPECT_LIMIT)
        .map(|v| inspect_str(v))
        .collect();
    if values.len() > INSPECT_LIMIT {
        items.push("...".to_owned());
    }
    let mut out = String::from("[");
    let mut column = 1usize;
    let last = items.len().saturating_sub(1);
    for (index, item) in items.iter().enumerate() {
        let tail = if index == last { "]" } else { "," };
        let needed = item.chars().count() + tail.len();
        if index > 0 {
            if column + 1 + needed > PRETTY_WIDTH {
                out.push_str("\n ");
                column = 1;
            } else {
                out.push(' ');
                column += 1;
            }
        }
        out.push_str(item);
        out.push_str(tail);
        column += needed;
    }
    if items.is_empty() {
        out.push(']');
    }
    out
}

/// `inspect/1` of a list of strings.
fn inspect_str_list(values: &[&str]) -> String {
    let items: Vec<String> = values.iter().map(|v| inspect_str(v)).collect();
    format!("[{}]", items.join(", "))
}

/// Whether `String.printable?/1` accepts the character.
fn printable_char(c: char) -> bool {
    matches!(
        c,
        '\u{20}'..='\u{7E}'
            | '\u{A0}'..='\u{D7FF}'
            | '\u{E000}'..='\u{FFFD}'
            | '\u{10000}'..='\u{10FFFF}'
            | '\n'
            | '\r'
            | '\t'
            | '\u{0B}'
            | '\u{08}'
            | '\u{0C}'
            | '\u{1B}'
            | '\u{7F}'
            | '\u{07}'
    )
}

/// Appends `c` escaped as `Inspect` does inside a `"`-delimited literal.
fn push_escaped(out: &mut String, c: char, next: Option<char>) {
    match c {
        '"' => out.push_str("\\\""),
        '\\' => out.push_str("\\\\"),
        '#' if next == Some('{') => out.push_str("\\#"),
        '\n' => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        '\u{0B}' => out.push_str("\\v"),
        '\u{08}' => out.push_str("\\b"),
        '\u{0C}' => out.push_str("\\f"),
        '\u{1B}' => out.push_str("\\e"),
        '\u{7F}' => out.push_str("\\d"),
        '\u{07}' => out.push_str("\\a"),
        _ => out.push(c),
    }
}

/// `inspect/1` of a binary: a quoted literal, or `<<…>>` bytes when not printable.
fn inspect_str(value: &str) -> String {
    if !value.chars().all(printable_char) {
        let mut items: Vec<String> = value
            .bytes()
            .take(INSPECT_LIMIT)
            .map(|b| b.to_string())
            .collect();
        if value.len() > INSPECT_LIMIT {
            items.push("...".to_owned());
        }
        return format!("<<{}>>", items.join(", "));
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        push_escaped(&mut out, c, chars.peek().copied());
    }
    out.push('"');
    out
}

/// `inspect/1` of a decoded JSON value (strings, numbers, `nil`, lists, maps).
fn inspect_json(value: &Value) -> String {
    match value {
        Value::Null => "nil".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if !n.is_i64() && !n.is_u64() => inspect_float(f),
            _ => n.to_string(),
        },
        Value::String(s) => inspect_str(s),
        Value::Array(items) => {
            if let Some(charlist) = inspect_charlist(items) {
                return charlist;
            }
            let mut parts: Vec<String> =
                items.iter().take(INSPECT_LIMIT).map(inspect_json).collect();
            if items.len() > INSPECT_LIMIT {
                parts.push("...".to_owned());
            }
            format!("[{}]", parts.join(", "))
        }
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            let mut parts: Vec<String> = entries
                .iter()
                .take(INSPECT_LIMIT)
                .map(|(k, v)| format!("{} => {}", inspect_str(k), inspect_json(v)))
                .collect();
            if entries.len() > INSPECT_LIMIT {
                parts.push("...".to_owned());
            }
            format!("%{{{}}}", parts.join(", "))
        }
    }
}

/// A non-empty list of printable ASCII integers inspects as `~c"…"`.
fn inspect_charlist(items: &[Value]) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let mut chars = Vec::with_capacity(items.len());
    for item in items {
        let code = item.as_u64()?;
        let c = char::from_u32(u32::try_from(code).ok()?)?;
        let ascii_printable =
            matches!(c, '\u{20}'..='\u{7E}' | '\u{07}'..='\u{0D}' | '\u{1B}' | '\u{7F}');
        if !ascii_printable {
            return None;
        }
        chars.push(c);
    }
    let mut out = String::from("~c\"");
    for (index, c) in chars.iter().enumerate() {
        push_escaped(&mut out, *c, chars.get(index + 1).copied());
    }
    out.push('"');
    Some(out)
}

/// `Float.to_string/1` (`:erlang.float_to_binary(f, [:short])`): the shorter
/// of the decimal and scientific forms, decimal on a tie.
fn inspect_float(value: f64) -> String {
    let fixed = {
        let s = format!("{value:?}");
        if s.contains('e') {
            format!("{value}")
        } else {
            s
        }
    };
    let fixed = if fixed.contains('.') {
        fixed
    } else {
        format!("{fixed}.0")
    };
    let sci = format!("{value:e}");
    let sci = match sci.split_once('e') {
        Some((mantissa, exp)) if !mantissa.contains('.') => format!("{mantissa}.0e{exp}"),
        Some((mantissa, exp)) => format!("{mantissa}e{exp}"),
        None => sci,
    };
    if sci.len() < fixed.len() {
        sci
    } else {
        fixed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejected_role_is_a_fatal_subscribe_error() {
        let params = parse_subscription_params(&json!({"schema": "public", "table": "todos"}))
            .expect("params");
        let missing = undefined_claims_role("not existing", &params);
        assert!(missing.fatal());
        let message = missing.to_string();
        assert!(message.contains("ERROR 42704 (undefined_object)"));
        assert!(message.contains("role \"not existing\" does not exist"));
        assert!(!message.contains("22023"));

        let absent = null_claims_role(&params);
        assert!(absent.fatal());
        let message = absent.to_string();
        assert!(message.contains("ERROR 23502 (not_null_violation)"));
        assert!(message.contains(
            "null value in column \"claims_role\" of relation \"subscription\" violates not-null constraint"
        ));
        assert!(message.contains("column: claims_role"));
        assert!(message.contains("table: subscription"));

        let denied = unconfigured_existing_role("postgres", &params);
        assert!(denied.fatal());
        let message = denied.to_string();
        assert!(message.contains("ERROR 42501 (insufficient_privilege)"));
        assert!(message.contains("permission denied to set role \"postgres\""));
    }

    #[allow(clippy::needless_pass_by_value)] // Lets call sites pass `json!(..)` directly.
    fn parse(value: Value) -> Result<SubscriptionParams, ParamsError> {
        parse_subscription_params(&value)
    }

    fn filters_of(filter: &str) -> Vec<(String, String, String, bool)> {
        parse(json!({"schema": "public", "table": "test", "filter": filter}))
            .expect("parses")
            .filters
            .into_iter()
            .map(|f| (f.column, f.op, f.value, f.negate))
            .collect()
    }

    fn f(column: &str, op: &str, value: &str, negate: bool) -> (String, String, String, bool) {
        (column.to_owned(), op.to_owned(), value.to_owned(), negate)
    }

    fn filter_error(filter: &str) -> String {
        parse(json!({"schema": "public", "table": "test", "filter": filter}))
            .expect_err("rejects")
            .to_string()
    }

    // Doctests in subscriptions.ex.

    #[test]
    fn subscriptions_doc_eq_filter() {
        let parsed =
            parse(json!({"schema": "public", "table": "messages", "filter": "subject=eq.hey"}))
                .unwrap();
        assert_eq!(parsed.action_filter, "*");
        assert_eq!(parsed.schema, "public");
        assert_eq!(parsed.table, "messages");
        assert_eq!(parsed.selected_columns, None);
        assert_eq!(
            filters_of("subject=eq.hey"),
            vec![f("subject", "eq", "hey", false)]
        );
    }

    #[test]
    fn subscriptions_doc_in_negate_and_composition() {
        assert_eq!(
            filters_of("subject=in.(hidee,ho)"),
            vec![f("subject", "in", "{hidee,ho}", false)]
        );
        assert_eq!(
            filters_of("subject=not.like.hey%"),
            vec![f("subject", "like", "hey%", true)]
        );
        assert_eq!(
            filters_of("id=gt.0,id=lt.100"),
            vec![f("id", "gt", "0", false), f("id", "lt", "100", false)]
        );
    }

    #[test]
    fn subscriptions_doc_empty_and_missing_filter() {
        assert!(filters_of("").is_empty());
        assert!(filters_of("   ").is_empty());
        let parsed = parse(json!({"schema": "public", "table": "messages"})).unwrap();
        assert!(parsed.filters.is_empty());
        assert_eq!(
            (parsed.schema.as_str(), parsed.table.as_str()),
            ("public", "messages")
        );
    }

    #[test]
    fn subscriptions_doc_only_schema_or_table() {
        let parsed = parse(json!({"schema": "public"})).unwrap();
        assert_eq!(
            (parsed.schema.as_str(), parsed.table.as_str()),
            ("public", "*")
        );
        let parsed = parse(json!({"table": "messages"})).unwrap();
        assert_eq!(
            (parsed.schema.as_str(), parsed.table.as_str()),
            ("public", "messages")
        );
    }

    #[test]
    fn subscriptions_doc_errors() {
        assert_eq!(
            filter_error("subject=foo.hey"),
            r#"Error parsing `filter` params: ["foo", "hey"]"#
        );
        assert_eq!(
            filter_error("undefined"),
            r#"Error parsing `filter` params: ["undefined"]"#
        );
        assert_eq!(
            parse(json!({})).unwrap_err().to_string(),
            "No subscription params provided. Please provide at least a `schema` or `table` to subscribe to: %{}"
        );
    }

    // test/extensions/postgres_cdc_rls/subscriptions_test.exs.

    #[test]
    fn subscriptions_multi_filter_with_unsupported_operator() {
        assert_eq!(
            filter_error("id=gt.0,id=foo.100"),
            r#"Error parsing `filter` params: ["foo", "100"]"#
        );
    }

    #[test]
    fn subscriptions_in_lists_combine_with_other_filters() {
        assert_eq!(
            filters_of("id=in.(1,2,3),details=eq.active"),
            vec![
                f("id", "in", "{1,2,3}", false),
                f("details", "eq", "active", false)
            ]
        );
        assert_eq!(
            filters_of("name=in.(red,blue),quantity=gt.0"),
            vec![
                f("name", "in", "{red,blue}", false),
                f("quantity", "gt", "0", false)
            ]
        );
        assert_eq!(
            filters_of("quantity=gt.0,name=in.(red,blue)"),
            vec![
                f("quantity", "gt", "0", false),
                f("name", "in", "{red,blue}", false)
            ]
        );
        assert_eq!(
            filters_of("name=in.(red,blue,green),status=in.(active,inactive)"),
            vec![
                f("name", "in", "{red,blue,green}", false),
                f("status", "in", "{active,inactive}", false)
            ]
        );
        assert_eq!(
            filters_of("id=eq.5,id=in.(1,2)"),
            vec![f("id", "eq", "5", false), f("id", "in", "{1,2}", false)]
        );
    }

    #[test]
    fn subscriptions_closing_parenthesis_in_values() {
        assert_eq!(
            filters_of("a=eq.x),b=eq.y),c=eq.z"),
            vec![
                f("a", "eq", "x)", false),
                f("b", "eq", "y)", false),
                f("c", "eq", "z", false)
            ]
        );
    }

    #[test]
    fn subscriptions_special_characters_verbatim() {
        assert_eq!(
            filters_of(r#"name=eq.O'Brien "x""#),
            vec![f("name", "eq", r#"O'Brien "x""#, false)]
        );
        assert_eq!(
            filters_of("name=not.neq.a'b"),
            vec![f("name", "neq", "a'b", true)]
        );
        assert_eq!(
            filters_of("name=in.(O'Brien,Smith)"),
            vec![f("name", "in", "{O'Brien,Smith}", false)]
        );
    }

    #[test]
    fn subscriptions_quoted_values() {
        assert_eq!(
            filters_of(r#"name=eq."a,b""#),
            vec![f("name", "eq", "a,b", false)]
        );
        assert_eq!(
            filters_of(r#"name=eq."a,b",id=gt.0"#),
            vec![f("name", "eq", "a,b", false), f("id", "gt", "0", false)]
        );
        assert_eq!(
            filters_of(r#"name=eq."a.b:c(d)""#),
            vec![f("name", "eq", "a.b:c(d)", false)]
        );
        assert_eq!(
            filters_of(r#"name=eq."she said \"hi\"""#),
            vec![f("name", "eq", r#"she said "hi""#, false)]
        );
        assert_eq!(
            filters_of(r#"path=eq."C:\\tmp""#),
            vec![f("path", "eq", r"C:\tmp", false)]
        );
        assert_eq!(
            filters_of(r#"name=not.neq."a,b""#),
            vec![f("name", "neq", "a,b", true)]
        );
        assert_eq!(
            filters_of(r#"path=eq."C:\new""#),
            vec![f("path", "eq", "C:new", false)]
        );
        assert_eq!(
            filters_of(r#"name=eq."""#),
            vec![f("name", "eq", "", false)]
        );
        assert_eq!(
            filters_of(r#"name=eq." a ""#),
            vec![f("name", "eq", " a ", false)]
        );
    }

    #[test]
    fn subscriptions_quoted_in_list_elements() {
        assert_eq!(
            filters_of(r#"tags=in.("a,b",c)"#),
            vec![f("tags", "in", r#"{"a,b",c}"#, false)]
        );
        assert_eq!(
            filters_of(r#"tags=in.("a,b","c,d")"#),
            vec![f("tags", "in", r#"{"a,b","c,d"}"#, false)]
        );
    }

    #[test]
    fn subscriptions_malformed_quotes_stay_literal() {
        assert_eq!(
            filters_of(r#"name=eq."unterminated"#),
            vec![f("name", "eq", r#""unterminated"#, false)]
        );
        assert_eq!(
            filters_of(r#"name=eq."ab"cd"#),
            vec![f("name", "eq", r#""ab"cd"#, false)]
        );
        assert_eq!(
            filters_of(r#"desc=eq.5" tall,id=gt.0"#),
            vec![
                f("desc", "eq", r#"5" tall"#, false),
                f("id", "gt", "0", false)
            ]
        );
        // A trailing backslash inside quotes leaves the value unterminated.
        assert_eq!(
            filters_of(r#"name=eq."ab\"#),
            vec![f("name", "eq", r#""ab\"#, false)]
        );
    }

    #[test]
    fn subscriptions_empty_segments() {
        let expected = "Error parsing `filter` params: filter must not contain empty segments (check for extra commas)";
        assert_eq!(filter_error("id=gt.0,"), expected);
        assert_eq!(filter_error(",id=gt.0"), expected);
        assert_eq!(filter_error("a=eq.1,,b=eq.2"), expected);
        assert_eq!(filter_error("a=eq.1, ,b=eq.2"), expected);
    }

    #[test]
    fn subscriptions_every_operator_and_negation() {
        for op in FILTER_TYPES {
            let value = if op == "in" { "(1)" } else { "1" };
            let expected = if op == "in" { "{1}" } else { "1" };
            assert_eq!(
                filters_of(&format!("c={op}.{value}")),
                vec![f("c", op, expected, false)]
            );
            assert_eq!(
                filters_of(&format!("c=not.{op}.{value}")),
                vec![f("c", op, expected, true)]
            );
        }
        assert_eq!(
            filters_of("details=like.hel%"),
            vec![f("details", "like", "hel%", false)]
        );
        assert_eq!(
            filters_of("flag=is.true"),
            vec![f("flag", "is", "true", false)]
        );
        assert_eq!(filters_of("id=not.eq.5"), vec![f("id", "eq", "5", true)]);
    }

    #[test]
    fn subscriptions_malformed_filters() {
        assert_eq!(
            filter_error("id=eq"),
            r#"Error parsing `filter` params: ["eq"]"#
        );
        assert_eq!(
            filter_error("id=not.eq"),
            r#"Error parsing `filter` params: ["eq"]"#
        );
        assert_eq!(
            filter_error("id="),
            r#"Error parsing `filter` params: [""]"#
        );
        assert_eq!(
            filter_error("id=not.foo.1"),
            r#"Error parsing `filter` params: ["foo", "1"]"#
        );
        assert_eq!(
            filter_error(r#"id=foo."x""#),
            r#"Error parsing `filter` params: ["foo", "\"x\""]"#
        );
        assert_eq!(
            filter_error("id=in.1,2"),
            "Error parsing `filter` params: `in` filter value must be wrapped by parentheses"
        );
        assert_eq!(
            filter_error("id=in.("),
            "Error parsing `filter` params: `in` filter value must be wrapped by parentheses"
        );
        assert_eq!(filters_of("id=in.()"), vec![f("id", "in", "{}", false)]);
        // Only the first `=` and `.` split.
        assert_eq!(filters_of("a=eq.b=c.d"), vec![f("a", "eq", "b=c.d", false)]);
    }

    #[test]
    fn subscriptions_non_string_params_are_missing() {
        let error =
            parse(json!({"schema": "public", "table": "images", "filter": [123]})).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("{NO_PARAMS}%{{\"filter\" => ~c\"{{\", \"schema\" => \"public\", \"table\" => \"images\"}}")
        );
        let error = parse(json!({"schema": "public", "table": {"actually a": "map"}})).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("{NO_PARAMS}%{{\"schema\" => \"public\", \"table\" => %{{\"actually a\" => \"map\"}}}}")
        );
        let error = parse(json!({"table": "images", "schema": {"actually a": "map"}})).unwrap_err();
        assert!(error.to_string().starts_with(NO_PARAMS));
        // A null filter is present, so no fallback branch matches.
        let error = parse(json!({"schema": "public", "table": "t", "filter": null})).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "{NO_PARAMS}%{{\"filter\" => nil, \"schema\" => \"public\", \"table\" => \"t\"}}"
            )
        );
        // A schema with a filter but no table matches nothing either.
        assert!(parse(json!({"schema": "public", "filter": "id=eq.1"})).is_err());
        assert_eq!(
            parse(json!(null)).unwrap_err().to_string(),
            format!("{NO_PARAMS}nil")
        );
    }

    #[test]
    fn subscriptions_tokens_are_redacted() {
        let error = parse(json!({"user_token": "secret"})).unwrap_err();
        assert_eq!(error, ParamsError::MissingRedacted);
        assert!(error.to_string().ends_with("subscribe to: <redacted>"));
        assert_eq!(
            parse(json!({"auth_token": "x", "event": "*"})).unwrap_err(),
            ParamsError::MissingRedacted
        );
    }

    #[test]
    fn subscriptions_event_maps_to_action_filter() {
        for (event, expected) in [
            ("*", "*"),
            ("INSERT", "INSERT"),
            ("insert", "INSERT"),
            ("Update", "UPDATE"),
            ("delete", "DELETE"),
            ("truncate", "*"),
        ] {
            let parsed = parse(json!({"event": event, "schema": "public"})).unwrap();
            assert_eq!(parsed.action_filter, expected, "{event}");
        }
        assert_eq!(
            parse(json!({"event": 1, "schema": "public"}))
                .unwrap()
                .action_filter,
            "*"
        );
    }

    #[test]
    fn subscriptions_select() {
        let parsed =
            parse(json!({"schema": "public", "table": "t", "select": ["a", 1, "b"]})).unwrap();
        assert_eq!(
            parsed.selected_columns,
            Some(vec!["a".to_owned(), "b".to_owned()])
        );
        let parsed = parse(json!({"schema": "public", "table": "t", "select": []})).unwrap();
        assert_eq!(parsed.selected_columns, Some(Vec::new()));
        let parsed = parse(json!({"schema": "public", "table": "t", "select": 5})).unwrap();
        assert_eq!(parsed.selected_columns, None);
        assert_eq!(
            parse(json!({"schema": "public", "table": "t", "select": "a,b"})).unwrap_err().to_string(),
            "Error parsing `select` params: expected a list of column name strings, e.g. select: [\"col1\", \"col2\"]"
        );
        // The select check runs before the missing-params check.
        assert_eq!(
            parse(json!({"select": "a"})).unwrap_err(),
            ParamsError::Select
        );
        let wildcard = "Column selection is not supported for wildcard subscriptions. Provide an explicit schema and table name.";
        assert_eq!(
            parse(json!({"schema": "public", "select": ["a"]}))
                .unwrap_err()
                .to_string(),
            wildcard
        );
        assert_eq!(
            parse(json!({"schema": "*", "table": "t", "select": []}))
                .unwrap_err()
                .to_string(),
            wildcard
        );
    }

    #[test]
    fn subscriptions_params_to_log_matches_upstream() {
        let parsed =
            parse(json!({"schema": "public", "table": "test", "filter": "subject=eq.hey"}))
                .unwrap();
        assert_eq!(
            params_to_log(&parsed),
            r#"event: *, schema: public, table: test, filters: [{"subject", "eq", "hey", false}], select: nil"#
        );
        let parsed = parse(json!({"schema": "public", "table": "test"})).unwrap();
        assert_eq!(
            params_to_log(&parsed),
            "event: *, schema: public, table: test, filters: [], select: nil"
        );
        let parsed =
            parse(json!({"schema": "public", "table": "t", "select": ["a", "b"]})).unwrap();
        assert!(params_to_log(&parsed).ends_with(r#"select: ["a", "b"]"#));
        let parsed = parse(json!({"schema": "public", "table": "t", "filter": "a=eq.1111111111,b=eq.2222222222,c=eq.3333333333"})).unwrap();
        assert!(params_to_log(&parsed)
            .contains("filters: [\n  {\"a\", \"eq\", \"1111111111\", false},\n  {\"b\""));
    }

    #[test]
    fn subscriptions_create_error_messages_and_fatality() {
        let malformed = CreateError::from(ParamsError::Select);
        assert!(malformed.fatal());
        assert!(malformed
            .to_string()
            .starts_with("Error parsing `select` params"));
        let insert = CreateError::SubscriptionInsertFailed("boom".to_owned());
        assert!(insert.fatal());
        assert_eq!(insert.to_string(), "boom");
        let timeout = CreateError::DatabaseTimeout(None);
        assert!(!timeout.fatal());
        assert_eq!(
            timeout.to_string(),
            format!("{{:error, \"{DATABASE_TIMEOUT_REASON}\"}}")
        );
        assert!(!CreateError::Database(sqlx::Error::RowNotFound).fatal());
        assert!(database_timeout(&sqlx::Error::PoolTimedOut));
        assert!(!database_timeout(&sqlx::Error::RowNotFound));
    }

    #[test]
    fn subscriptions_inspect_matches_elixir() {
        assert_eq!(inspect_str("a\"b\\c#{d}\n"), r#""a\"b\\c\#{d}\n""#);
        assert_eq!(inspect_str("\u{1}"), "<<1>>");
        assert_eq!(
            inspect_json(&json!([1.5, 100.0, 1e20, true, "x"])),
            r#"[1.5, 100.0, 1.0e20, true, "x"]"#
        );
        assert_eq!(inspect_json(&json!([104, 105])), r#"~c"hi""#);
        assert_eq!(inspect_json(&json!([1, 2])), "[1, 2]");
        assert_eq!(
            inspect_json(&json!({"b": 1, "a": [] })),
            r#"%{"a" => [], "b" => 1}"#
        );
        let long: Vec<u32> = (1..=60).collect();
        assert!(inspect_json(&json!(long)).ends_with("50, ...]"));
    }
}
