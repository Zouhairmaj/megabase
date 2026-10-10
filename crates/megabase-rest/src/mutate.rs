// Ported from postgrest src/library/PostgREST/Query/QueryBuilder.hs (MIT), pin v16.4,
// postgrest src/library/PostgREST/ApiRequest/Payload.hs (MIT), pin v16.4,
// and postgrest src/library/PostgREST/Response.hs (MIT), pin v16.4.

//! `POST`, `PATCH`, `PUT`, and `DELETE` on `/rest/v1/{relation}`.
//!
//! Default `Prefer` is `return=minimal`: creates answer `201` with an empty
//! body, and the other mutations answer `204`. `return=representation` adds
//! the affected rows as the body (`200`, or `201` for a create), and
//! `return=headers-only` adds a `Location` header to a single-row create.
//! The other preferences stay unimplemented. Values are bound. Names are
//! `quote_ident` after a catalog lookup.

use std::collections::BTreeSet;

use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::Response;
use serde_json::Value;

use crate::filter::{quote_ident, sql_type_name};
use crate::params::{where_clause, ReadQuery};
use crate::prefer::{Preferences, Representation};
use crate::read::{
    self, begin, content_range, load_columns, pgrst, query_failure, relation_exists,
    set_request_context, table_not_found, unimplemented_unit, unsafe_type, Session,
};

const EMPTY_JSON: &str = "Empty or invalid json";
const KEYS_MUST_MATCH: &str = "All object keys must match";

enum Kind {
    Create,
    Update,
    Upsert,
    Delete,
}

struct Payload {
    /// Original JSON text, or `[]` when the value is neither an object nor an array.
    raw: String,
    /// `true` when the first non-space byte is `{` (`SqlFragment.fromJsonBodyF`).
    object: bool,
    keys: Vec<String>,
}

/// Run one mutation. `query` is the already-parsed query string.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn mutate(
    pool: Option<&sqlx::PgPool>,
    schema: &str,
    relation: &str,
    session: &Session,
    method: &Method,
    query: &ReadQuery,
    body: &[u8],
    content_type: Option<&HeaderValue>,
    prefs: &Preferences,
) -> Response {
    let kind = match *method {
        Method::POST => Kind::Create,
        Method::PATCH => Kind::Update,
        Method::PUT => Kind::Upsert,
        Method::DELETE => Kind::Delete,
        _ => {
            return pgrst(
                StatusCode::METHOD_NOT_ALLOWED,
                "PGRST117",
                &format!("Unsupported HTTP method: {method}"),
                None,
                None,
            );
        }
    };
    // Aggregates in `select=` shape a `RETURNING` projection, which is not
    // served. Ignoring them would hide the request instead of answering 501.
    if query.has_aggregate() {
        return unimplemented_unit(method, &format!("/rest/v1/{relation}"), "");
    }
    if matches!(kind, Kind::Upsert) && query.limits_rows() {
        return pgrst(
            StatusCode::BAD_REQUEST,
            "PGRST114",
            "limit/offset querystring parameters are not allowed for PUT",
            None,
            None,
        );
    }
    // A limited write is not served. `where_clause` drops the page, so
    // executing it would change every matching row.
    if matches!(kind, Kind::Update | Kind::Delete) && query.limits_rows() {
        return unimplemented_unit(method, &format!("/rest/v1/{relation}"), "");
    }
    let payload = if matches!(kind, Kind::Delete) {
        None
    } else {
        let relaxed = query.column_list().is_some() && matches!(kind, Kind::Create | Kind::Update);
        match json_payload(content_type, body, relaxed) {
            Ok(payload) => Some(payload),
            Err(response) => return *response,
        }
    };
    let eq_filters = if matches!(kind, Kind::Upsert) {
        match query.eq_column_filters() {
            Some(filters) if !filters.is_empty() => Some(filters),
            _ => {
                return pgrst(
                    StatusCode::METHOD_NOT_ALLOWED,
                    "PGRST105",
                    "Filters must include all and only primary key columns with 'eq' operators",
                    None,
                    None,
                );
            }
        }
    } else {
        None
    };

    let Some(pool) = pool else {
        return read::database_unavailable();
    };
    let mut tx = match begin(pool).await {
        Ok(tx) => tx,
        Err(response) => return *response,
    };
    let exists = match relation_exists(&mut tx, schema, relation, session.anon).await {
        Ok(exists) => exists,
        Err(response) => return *response,
    };
    if !exists {
        return table_not_found(schema, relation);
    }
    let columns = match load_columns(&mut tx, schema, relation, session.anon).await {
        Ok(columns) => columns,
        Err(response) => return *response,
    };
    let column_types: Vec<(&str, &str)> = columns
        .iter()
        .map(|(name, pg_type)| (name.as_str(), pg_type.as_str()))
        .collect();

    let keys = if let Some(list) = query.column_list() {
        if matches!(kind, Kind::Create | Kind::Update) {
            list.to_vec()
        } else {
            payload
                .as_ref()
                .map(|item| item.keys.clone())
                .unwrap_or_default()
        }
    } else {
        payload
            .as_ref()
            .map(|item| item.keys.clone())
            .unwrap_or_default()
    };
    if let Some(missing) = keys
        .iter()
        .find(|name| !column_types.iter().any(|(column, _)| column == name))
    {
        return pgrst(
            StatusCode::BAD_REQUEST,
            "PGRST204",
            &format!("Could not find the '{missing}' column of '{relation}' in the schema cache"),
            None,
            None,
        );
    }

    let pk = if matches!(kind, Kind::Upsert) {
        match primary_key(&mut tx, schema, relation, session.anon).await {
            Ok(pk) => pk,
            Err(response) => return *response,
        }
    } else {
        Vec::new()
    };
    if let Some(filters) = &eq_filters {
        if !pk_matches(filters, &pk) {
            return pgrst(
                StatusCode::METHOD_NOT_ALLOWED,
                "PGRST105",
                "Filters must include all and only primary key columns with 'eq' operators",
                None,
                None,
            );
        }
    }

    // megabase:unit rest:prefer:return=headers-only
    let ret = match prefs.representation {
        Some(Representation::Full) => Ret::Full,
        Some(Representation::HeadersOnly) if matches!(kind, Kind::Create) => {
            match primary_key(&mut tx, schema, relation, session.anon).await {
                Ok(pk) if !pk.is_empty() => Ret::Keys(pk),
                Ok(_) => Ret::One,
                Err(response) => return *response,
            }
        }
        _ => Ret::One,
    };

    let mut built = match kind {
        Kind::Update if keys.is_empty() => MutationSql {
            sql: format!(
                "WITH mutated AS (SELECT {} FROM {}.{} WHERE false)",
                returning(relation, &kind, &column_types, &ret),
                quote_ident(schema),
                quote_ident(relation)
            ),
            params: Vec::new(),
        },
        Kind::Create | Kind::Update | Kind::Upsert => {
            let Some(payload) = payload.as_ref() else {
                return pgrst(StatusCode::BAD_REQUEST, "PGRST102", EMPTY_JSON, None, None);
            };
            match write_sql(
                schema,
                relation,
                &kind,
                &keys,
                payload,
                query,
                &column_types,
                &pk,
                eq_filters.as_deref().unwrap_or(&[]),
                &ret,
            ) {
                Ok(sql) => sql,
                Err(response) => return *response,
            }
        }
        Kind::Delete => match delete_sql(schema, relation, query, &column_types, &ret) {
            Ok(sql) => sql,
            Err(response) => return *response,
        },
    };

    if let Err(response) = set_request_context(
        &mut tx,
        session,
        method.as_str(),
        &format!("/{relation}"),
        schema,
    )
    .await
    {
        return *response;
    }
    finish_sql(&mut built, relation, query, &column_types, &ret, &kind);
    let mut statement =
        sqlx::query_as::<sqlx::Postgres, (i64, i64, String, Option<String>)>(&built.sql);
    for param in &built.params {
        statement = statement.bind(param);
    }
    let (count, inserted, body, location) = match statement.fetch_one(&mut *tx).await {
        Ok(row) => row,
        Err(error) => return query_failure(&error, session.anon),
    };
    if matches!(kind, Kind::Upsert) && count != 1 {
        return pgrst(
            StatusCode::BAD_REQUEST,
            "PGRST115",
            "Payload values do not match URL in primary key column(s)",
            None,
            None,
        );
    }
    if let Err(error) = tx.commit().await {
        return query_failure(&error, session.anon);
    }
    // megabase:unit rest:prefer:return=representation
    // megabase:unit rest:prefer:return=minimal
    mutation_response(
        &kind,
        &Outcome {
            count,
            inserted,
            body,
            location,
        },
        schema,
        relation,
        prefs.representation,
    )
}

struct MutationSql {
    sql: String,
    params: Vec<String>,
}

/// One row of the final `SELECT`.
struct Outcome {
    count: i64,
    /// Rows a create or upsert inserted (not updated).
    inserted: i64,
    /// JSON array of the affected rows; `[]` unless `return=representation`.
    body: String,
    /// `["col=eq.value", …]` for `return=headers-only` on a single-row create.
    location: Option<String>,
}

/// What the mutation's `RETURNING` clause must produce.
///
/// PostgreSQL needs `SELECT` privilege on every column a `RETURNING` names, so a
/// write-only role keeps working only if minimal responses return no columns.
/// PostgREST's `returningF` is empty in that case (`QueryBuilder.hs`).
enum Ret {
    /// `RETURNING 1`: the default and `return=minimal`.
    One,
    /// Primary-key columns for the `Location` of `return=headers-only`.
    Keys(Vec<String>),
    /// Every column for `return=representation`.
    Full,
}

/// Alias of the `xmax = 0` flag. It sits next to the user's columns, so it is
/// prefixed like the other generated names.
const INSERTED: &str = "_megabase_inserted";

/// `RETURNING` list for `ret`.
///
/// Only a full upsert reads `xmax` to tell inserted rows from updated ones. A
/// view has no such system column.
fn returning(relation: &str, kind: &Kind, column_types: &[(&str, &str)], ret: &Ret) -> String {
    let table = quote_ident(relation);
    match ret {
        Ret::One => "1".to_string(),
        Ret::Keys(pk) => pk
            .iter()
            .map(|name| format!("{table}.{}", quote_ident(name)))
            .collect::<Vec<_>>()
            .join(", "),
        Ret::Full => {
            let mut parts: Vec<String> = if column_types.is_empty() {
                vec![format!("{table}.*")]
            } else {
                column_types
                    .iter()
                    .map(|(name, _)| format!("{table}.{}", quote_ident(name)))
                    .collect()
            };
            if matches!(kind, Kind::Upsert) {
                parts.push(format!("({table}.xmax = 0) AS {INSERTED}"));
            }
            parts.join(", ")
        }
    }
}

/// Append the final `SELECT` to the `WITH mutated AS (…)` head.
///
/// Columns: affected rows, inserted rows, JSON body, `Location` pairs.
fn finish_sql(
    built: &mut MutationSql,
    relation: &str,
    query: &ReadQuery,
    column_types: &[(&str, &str)],
    ret: &Ret,
    kind: &Kind,
) {
    let (src, inserted, body, location) = match ret {
        Ret::One => (
            String::new(),
            "0::bigint".to_string(),
            "'[]'::text".to_string(),
            "NULL::text".to_string(),
        ),
        Ret::Keys(pk) => {
            let mut marks = Vec::new();
            for name in pk {
                built.params.push(name.clone());
                marks.push(format!("${}", built.params.len()));
            }
            // Pairs follow the primary-key order, not the order `row_to_json`
            // emits the columns in.
            let location = format!(
                "CASE WHEN (SELECT count(*) FROM mutated) = 1 THEN (SELECT array_to_json(\
                 array_agg(j.key || '=' || coalesce('eq.' || j.value, 'is.null') ORDER BY k.ord))::text \
                 FROM (SELECT row_to_json(_) AS row FROM _megabase_src AS _ LIMIT 1) d \
                 CROSS JOIN json_each_text(d.row) AS j \
                 JOIN unnest(ARRAY[{}]::text[]) WITH ORDINALITY AS k(name, ord) ON k.name = j.key) END",
                marks.join(", ")
            );
            (
                format!(
                    ", _megabase_src AS (SELECT {} FROM mutated)",
                    quote_list(pk)
                ),
                "0::bigint".to_string(),
                "'[]'::text".to_string(),
                location,
            )
        }
        Ret::Full => {
            let cols = if column_types.is_empty() {
                "*".to_string()
            } else {
                quote_list(
                    &column_types
                        .iter()
                        .map(|(name, _)| (*name).to_string())
                        .collect::<Vec<_>>(),
                )
            };
            let select = query.representation_select("_megabase_src", relation, &mut built.params);
            let inserted = if matches!(kind, Kind::Upsert) {
                format!("(SELECT count(*) FROM mutated WHERE {INSERTED})::bigint")
            } else {
                "0::bigint".to_string()
            };
            (
                format!(", _megabase_src AS (SELECT {cols} FROM mutated)"),
                inserted,
                format!(
                    "coalesce((SELECT json_agg(t.row ORDER BY t.rn) FROM \
                     (SELECT to_json(s) AS row, row_number() OVER () AS rn FROM ({select}) s) t), \
                     '[]'::json)::text"
                ),
                "NULL::text".to_string(),
            )
        }
    };
    built.sql = format!(
        "{}{src} SELECT (SELECT count(*) FROM mutated)::bigint, {inserted}, {body}, {location}",
        built.sql
    );
}

fn mutation_response(
    kind: &Kind,
    outcome: &Outcome,
    schema: &str,
    relation: &str,
    representation: Option<Representation>,
) -> Response {
    let full = representation == Some(Representation::Full);
    let (status, range) = match kind {
        Kind::Create => (StatusCode::CREATED, Some("*/*".to_string())),
        Kind::Update if full => (StatusCode::OK, Some(content_range(0, outcome.count))),
        Kind::Update => (
            StatusCode::NO_CONTENT,
            Some(content_range(0, outcome.count)),
        ),
        Kind::Delete if full => (StatusCode::OK, Some("*/*".to_string())),
        Kind::Delete => (StatusCode::NO_CONTENT, Some("*/*".to_string())),
        Kind::Upsert if full && outcome.inserted > 0 => (StatusCode::CREATED, None),
        Kind::Upsert if full => (StatusCode::OK, None),
        Kind::Upsert => (StatusCode::NO_CONTENT, None),
    };
    let body = if full {
        axum::body::Body::from(outcome.body.clone())
    } else {
        axum::body::Body::empty()
    };
    let mut response = Response::builder()
        .status(status)
        .body(body)
        .unwrap_or_else(|_| Response::new(axum::body::Body::empty()));
    if full {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(read::JSON_UTF8),
        );
    }
    if let Some(range) = range {
        if let Ok(value) = HeaderValue::from_str(&range) {
            response.headers_mut().insert(header::CONTENT_RANGE, value);
        }
    }
    if let Some(location) = location_header(relation, outcome.location.as_deref()) {
        response.headers_mut().insert(header::LOCATION, location);
    }
    read::profile_header(&mut response, schema);
    response
}

/// `/relation?col=eq.value`, with each pair percent-encoded like
/// `renderSimpleQuery True`.
fn location_header(relation: &str, pairs: Option<&str>) -> Option<HeaderValue> {
    let pairs: Vec<String> = serde_json::from_str(pairs?).ok()?;
    if pairs.is_empty() {
        return None;
    }
    let query = pairs
        .iter()
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            format!("{}={}", percent_encode(key), percent_encode(value))
        })
        .collect::<Vec<_>>()
        .join("&");
    HeaderValue::from_bytes(format!("/{relation}?{query}").as_bytes()).ok()
}

fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn write_sql(
    schema: &str,
    relation: &str,
    kind: &Kind,
    keys: &[String],
    payload: &Payload,
    query: &ReadQuery,
    column_types: &[(&str, &str)],
    pk: &[String],
    eq_filters: &[(String, String)],
    ret: &Ret,
) -> Result<MutationSql, Box<Response>> {
    let mut params = Vec::new();
    params.push(payload.raw.clone());
    let body_index = params.len();
    let record = if keys.is_empty() && !matches!(kind, Kind::Upsert) {
        if payload.object {
            // `{}` is one default row. The placeholder stays so the bound body is used.
            // `SqlFragment.fromJsonBodyF` emptyFieldsSource.
            format!(
                "(SELECT ${body_index}::json AS json_data) AS pgrst_payload, LATERAL (SELECT FROM (VALUES (1)) AS _) AS pgrst_body"
            )
        } else {
            format!("json_array_elements(${body_index}::json) AS _")
        }
    } else {
        let mut record_cols = keys.to_vec();
        if matches!(kind, Kind::Upsert) {
            for name in pk {
                if !record_cols.iter().any(|column| column == name) {
                    record_cols.push(name.clone());
                }
            }
        }
        let typed = typed_columns(&record_cols, column_types)?;
        let source = if payload.object {
            "json_to_record"
        } else {
            "json_to_recordset"
        };
        format!("{source}(${body_index}::json) AS pgrst_body({typed})")
    };

    let qualified = format!("{}.{}", quote_ident(schema), quote_ident(relation));
    let sql = match kind {
        Kind::Create | Kind::Upsert => {
            let mut sql = format!("WITH mutated AS (INSERT INTO {qualified} ");
            if keys.is_empty() {
                sql.push_str("SELECT FROM ");
                sql.push_str(&record);
            } else {
                let idents = quote_list(keys);
                let selected = keys
                    .iter()
                    .map(|name| format!("pgrst_body.{}", quote_ident(name)))
                    .collect::<Vec<_>>()
                    .join(", ");
                sql.push('(');
                sql.push_str(&idents);
                sql.push_str(") SELECT ");
                sql.push_str(&selected);
                sql.push_str(" FROM ");
                sql.push_str(&record);
            }
            if matches!(kind, Kind::Upsert) {
                sql.push_str(" WHERE ");
                push_pk_match(&mut sql, &mut params, pk, eq_filters, column_types)?;
                sql.push_str(" ON CONFLICT (");
                sql.push_str(&quote_list(pk));
                sql.push_str(") ");
                if keys.is_empty() {
                    sql.push_str("DO NOTHING");
                } else {
                    sql.push_str("DO UPDATE SET ");
                    let sets = keys
                        .iter()
                        .map(|name| {
                            let ident = quote_ident(name);
                            format!("{ident} = EXCLUDED.{ident}")
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    sql.push_str(&sets);
                }
            }
            sql.push_str(&format!(
                " RETURNING {})",
                returning(relation, kind, column_types, ret)
            ));
            sql
        }
        Kind::Update => {
            let sets = keys
                .iter()
                .map(|name| {
                    let ident = quote_ident(name);
                    format!("{ident} = pgrst_body.{ident}")
                })
                .collect::<Vec<_>>()
                .join(", ");
            let mut sql = format!("WITH mutated AS (UPDATE {qualified} SET {sets} FROM {record}");
            if let Some(filter) = where_clause(relation, query, column_types)
                .map_err(|error| Box::new(unsafe_type(&error)))?
            {
                // The filter placeholders were numbered from `$1`, but `$1` is the JSON body.
                let shifted = shift_placeholders(&filter.sql, 1);
                sql.push_str(" WHERE ");
                sql.push_str(&shifted);
                params.extend(filter.params);
            }
            sql.push_str(&format!(
                " RETURNING {})",
                returning(relation, kind, column_types, ret)
            ));
            sql
        }
        Kind::Delete => String::new(),
    };
    Ok(MutationSql { sql, params })
}

fn delete_sql(
    schema: &str,
    relation: &str,
    query: &ReadQuery,
    column_types: &[(&str, &str)],
    ret: &Ret,
) -> Result<MutationSql, Box<Response>> {
    let qualified = format!("{}.{}", quote_ident(schema), quote_ident(relation));
    let mut sql = format!("WITH mutated AS (DELETE FROM {qualified}");
    let params = if let Some(filter) = where_clause(relation, query, column_types)
        .map_err(|error| Box::new(unsafe_type(&error)))?
    {
        sql.push_str(" WHERE ");
        sql.push_str(&filter.sql);
        filter.params
    } else {
        Vec::new()
    };
    sql.push_str(&format!(
        " RETURNING {})",
        returning(relation, &Kind::Delete, column_types, ret)
    ));
    Ok(MutationSql { sql, params })
}

fn push_pk_match(
    sql: &mut String,
    params: &mut Vec<String>,
    pk: &[String],
    filters: &[(String, String)],
    column_types: &[(&str, &str)],
) -> Result<(), Box<Response>> {
    for (index, name) in pk.iter().enumerate() {
        if index > 0 {
            sql.push_str(" AND ");
        }
        let value = filters
            .iter()
            .find(|(column, _)| column == name)
            .map(|(_, value)| value.as_str())
            .unwrap_or("");
        let pg_type = column_types
            .iter()
            .find(|(column, _)| *column == name)
            .map(|(_, pg_type)| *pg_type)
            .unwrap_or("text");
        let cast = sql_type_name(pg_type).map_err(|error| Box::new(unsafe_type(&error)))?;
        params.push(value.to_string());
        let placeholder = params.len();
        sql.push_str("pgrst_body.");
        sql.push_str(&quote_ident(name));
        sql.push_str(" = ($");
        sql.push_str(&placeholder.to_string());
        sql.push_str("::text)::");
        sql.push_str(cast);
    }
    Ok(())
}

fn typed_columns(names: &[String], column_types: &[(&str, &str)]) -> Result<String, Box<Response>> {
    let mut parts = Vec::new();
    for name in names {
        let pg_type = column_types
            .iter()
            .find(|(column, _)| *column == name)
            .map(|(_, pg_type)| *pg_type)
            .unwrap_or("text");
        let cast = sql_type_name(pg_type).map_err(|error| Box::new(unsafe_type(&error)))?;
        parts.push(format!("{} {cast}", quote_ident(name)));
    }
    Ok(parts.join(", "))
}

fn quote_list(names: &[String]) -> String {
    names
        .iter()
        .map(|name| quote_ident(name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `json_to_record` is `$1`. Filter placeholders that started at `$1` move up.
///
/// `$digits` inside a double-quoted identifier stays put. A doubled quote
/// (`""`) is still that identifier, not its end.
fn shift_placeholders(sql: &str, by: usize) -> String {
    let mut out = String::with_capacity(sql.len());
    let bytes = sql.as_bytes();
    let mut index = 0;
    let mut quoted = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'"' {
            out.push('"');
            if quoted && index + 1 < bytes.len() && bytes[index + 1] == b'"' {
                out.push('"');
                index += 2;
                continue;
            }
            quoted = !quoted;
            index += 1;
            continue;
        }
        if !quoted && byte == b'$' {
            let start = index + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > start {
                let number: usize = sql[start..end].parse().unwrap_or(0);
                out.push('$');
                out.push_str(&(number + by).to_string());
                index = end;
                continue;
            }
        }
        let Some(ch) = sql[index..].chars().next() else {
            break;
        };
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

fn pk_matches(filters: &[(String, String)], pk: &[String]) -> bool {
    if filters.len() != pk.len() || pk.is_empty() {
        return false;
    }
    let mut names: Vec<&str> = filters.iter().map(|(name, _)| name.as_str()).collect();
    names.sort_unstable();
    if names.windows(2).any(|pair| pair[0] == pair[1]) {
        return false;
    }
    let mut expected: Vec<&str> = pk.iter().map(String::as_str).collect();
    expected.sort_unstable();
    names == expected
}

async fn primary_key(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    schema: &str,
    relation: &str,
    anon: bool,
) -> Result<Vec<String>, Box<Response>> {
    sqlx::query_scalar(
        "SELECT a.attname::text
         FROM pg_constraint AS k
         CROSS JOIN LATERAL unnest(k.conkey) WITH ORDINALITY AS u(attnum, ord)
         JOIN pg_attribute AS a
           ON a.attrelid = k.conrelid AND a.attnum = u.attnum
         JOIN pg_class AS c ON c.oid = k.conrelid
         JOIN pg_namespace AS n ON n.oid = c.relnamespace
         WHERE k.contype = 'p'
           AND n.nspname = $1
           AND c.relname = $2
           AND NOT a.attisdropped
         ORDER BY u.ord",
    )
    .bind(schema)
    .bind(relation)
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| Box::new(query_failure(&error, anon)))
}

fn json_payload(
    content_type: Option<&HeaderValue>,
    body: &[u8],
    relaxed: bool,
) -> Result<Payload, Box<Response>> {
    if let Some(unit) = read::reject_accept(content_type) {
        if !unit.is_empty() {
            return Err(Box::new(unimplemented_unit(&Method::POST, "", unit)));
        }
        let mime = content_type
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        return Err(Box::new(pgrst(
            StatusCode::BAD_REQUEST,
            "PGRST102",
            &format!("Content-Type not acceptable: {mime}"),
            None,
            None,
        )));
    }
    if body.is_empty() {
        return Err(Box::new(pgrst(
            StatusCode::BAD_REQUEST,
            "PGRST102",
            EMPTY_JSON,
            None,
            None,
        )));
    }
    let value: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(_) => {
            return Err(Box::new(pgrst(
                StatusCode::BAD_REQUEST,
                "PGRST102",
                EMPTY_JSON,
                None,
                None,
            )));
        }
    };
    match value {
        Value::Object(map) => Ok(Payload {
            raw: String::from_utf8_lossy(body).into_owned(),
            object: true,
            keys: map.keys().cloned().collect(),
        }),
        Value::Array(_) if relaxed => Ok(Payload {
            raw: String::from_utf8_lossy(body).into_owned(),
            object: false,
            keys: Vec::new(),
        }),
        Value::Array(items) => array_payload(body, &items),
        _ => Ok(Payload {
            raw: "[]".to_string(),
            object: false,
            keys: Vec::new(),
        }),
    }
}

fn array_payload(body: &[u8], items: &[Value]) -> Result<Payload, Box<Response>> {
    if items.is_empty() {
        return Ok(Payload {
            raw: String::from_utf8_lossy(body).into_owned(),
            object: false,
            keys: Vec::new(),
        });
    }
    let mut keys: Option<BTreeSet<String>> = None;
    for item in items {
        let Value::Object(map) = item else {
            return Err(Box::new(pgrst(
                StatusCode::BAD_REQUEST,
                "PGRST102",
                KEYS_MUST_MATCH,
                None,
                None,
            )));
        };
        let row: BTreeSet<String> = map.keys().cloned().collect();
        if let Some(expected) = &keys {
            if expected != &row {
                return Err(Box::new(pgrst(
                    StatusCode::BAD_REQUEST,
                    "PGRST102",
                    KEYS_MUST_MATCH,
                    None,
                    None,
                )));
            }
        } else {
            keys = Some(row);
        }
    }
    let object = body
        .iter()
        .find(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
        .is_some_and(|byte| *byte == b'{');
    let keys = keys.unwrap_or_default().into_iter().collect::<Vec<_>>();
    // Preserve the first object's key order when the set comparison succeeded.
    let ordered = if let Some(Value::Object(map)) = items.first() {
        map.keys().cloned().collect()
    } else {
        keys
    };
    Ok(Payload {
        raw: String::from_utf8_lossy(body).into_owned(),
        object,
        keys: ordered,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::parse_get_query;

    #[test]
    fn empty_object_insert_binds_the_json_body() {
        let query = parse_get_query("").expect("blank query");
        let payload = Payload {
            raw: "{}".to_string(),
            object: true,
            keys: Vec::new(),
        };
        let built = write_sql(
            "public",
            "todos",
            &Kind::Create,
            &[],
            &payload,
            &query,
            &[],
            &[],
            &[],
            &Ret::One,
        )
        .expect("sql");
        assert!(
            built.sql.contains("RETURNING 1)"),
            "minimal must not name columns: {}",
            built.sql
        );
        assert!(
            built.sql.contains("$1::json"),
            "empty object insert must reference the bound body: {}",
            built.sql
        );
        assert!(built.sql.contains("VALUES (1)"));
        assert_eq!(built.params, vec!["{}".to_string()]);
    }

    fn outcome(count: i64, inserted: i64) -> Outcome {
        Outcome {
            count,
            inserted,
            body: r#"[{"id":1}]"#.to_string(),
            location: Some(r#"["id=eq.1","a b=eq.x/y"]"#.to_string()),
        }
    }

    #[test]
    fn representation_changes_status_body_and_content_type() {
        for (kind, status) in [
            (Kind::Create, StatusCode::CREATED),
            (Kind::Update, StatusCode::OK),
            (Kind::Delete, StatusCode::OK),
        ] {
            let full = mutation_response(
                &kind,
                &outcome(1, 1),
                "public",
                "todos",
                Some(Representation::Full),
            );
            assert_eq!(full.status(), status);
            assert_eq!(
                full.headers().get(header::CONTENT_TYPE),
                Some(&HeaderValue::from_static(read::JSON_UTF8))
            );
            assert!(full.headers().get(header::LOCATION).is_some());
        }
        let upsert_new = mutation_response(
            &Kind::Upsert,
            &outcome(1, 1),
            "public",
            "todos",
            Some(Representation::Full),
        );
        assert_eq!(upsert_new.status(), StatusCode::CREATED);
        let upsert_old = mutation_response(
            &Kind::Upsert,
            &outcome(1, 0),
            "public",
            "todos",
            Some(Representation::Full),
        );
        assert_eq!(upsert_old.status(), StatusCode::OK);
    }

    #[test]
    fn minimal_and_headers_only_have_no_body() {
        for rep in [
            None,
            Some(Representation::Minimal),
            Some(Representation::HeadersOnly),
        ] {
            let update = mutation_response(&Kind::Update, &outcome(2, 0), "public", "todos", rep);
            assert_eq!(update.status(), StatusCode::NO_CONTENT);
            assert_eq!(
                update.headers().get(header::CONTENT_RANGE),
                Some(&HeaderValue::from_static("0-1/*"))
            );
            assert!(update.headers().get(header::CONTENT_TYPE).is_none());
            let create = mutation_response(&Kind::Create, &outcome(1, 1), "public", "todos", rep);
            assert_eq!(create.status(), StatusCode::CREATED);
            assert!(create.headers().get(header::CONTENT_TYPE).is_none());
        }
    }

    #[test]
    fn location_is_percent_encoded() {
        let value = location_header("todos", Some(r#"["id=eq.1","a b=eq.x/y"]"#));
        assert_eq!(
            value,
            Some(HeaderValue::from_static("/todos?id=eq.1&a%20b=eq.x%2Fy"))
        );
        assert_eq!(location_header("todos", Some("[]")), None);
        assert_eq!(location_header("todos", None), None);
    }

    #[test]
    fn final_select_appends_body_and_location_params() {
        let query = parse_get_query("select=id&order=id.desc").expect("query");
        let mut built = MutationSql {
            sql: "WITH mutated AS (DELETE FROM t RETURNING 1)".to_string(),
            params: vec!["x".to_string()],
        };
        finish_sql(
            &mut built,
            "todos",
            &query,
            &[("id", "bigint")],
            &Ret::Full,
            &Kind::Delete,
        );
        assert!(built
            .sql
            .contains("FROM _megabase_src AS \"todos\" ORDER BY"));

        let mut keyed = MutationSql {
            sql: "WITH mutated AS (INSERT INTO t DEFAULT VALUES RETURNING \"t\".\"id\")"
                .to_string(),
            params: Vec::new(),
        };
        finish_sql(
            &mut keyed,
            "t",
            &query,
            &[("id", "bigint")],
            &Ret::Keys(vec!["id".to_string()]),
            &Kind::Create,
        );
        assert!(keyed.sql.contains("ORDER BY k.ord"));
        assert!(keyed.sql.contains("ARRAY[$1]"));
        assert_eq!(keyed.params, vec!["id".to_string()]);
    }

    #[test]
    fn returning_names_columns_only_when_needed() {
        let columns = [("id", "bigint"), ("secret", "text")];
        assert_eq!(returning("t", &Kind::Create, &columns, &Ret::One), "1");
        assert_eq!(
            returning("t", &Kind::Create, &columns, &Ret::Keys(vec!["id".into()])),
            "\"t\".\"id\""
        );
        let full = returning("t", &Kind::Upsert, &columns, &Ret::Full);
        assert!(full.contains("\"t\".\"secret\""));
        assert!(full.contains("xmax = 0"));
        let mut minimal = MutationSql {
            sql: "WITH mutated AS (DELETE FROM t RETURNING 1)".to_string(),
            params: Vec::new(),
        };
        let query = parse_get_query("").expect("blank query");
        finish_sql(
            &mut minimal,
            "t",
            &query,
            &columns,
            &Ret::One,
            &Kind::Delete,
        );
        assert!(!minimal.sql.contains("_megabase_src"));
    }

    #[test]
    fn shift_placeholders_leaves_dollars_inside_identifiers() {
        let shifted = shift_placeholders(r#""todos"."a$1" = $1 AND "a""b$2" = $2"#, 1);
        assert_eq!(shifted, r#""todos"."a$1" = $2 AND "a""b$2" = $3"#);
        assert_eq!(shift_placeholders("\"café$1\" = $1", 1), "\"café$1\" = $2");

        let query = parse_get_query("a$1=eq.1").expect("filter");
        let payload = Payload {
            raw: r#"{"title":"x"}"#.to_string(),
            object: true,
            keys: vec!["title".to_string()],
        };
        let built = write_sql(
            "public",
            "todos",
            &Kind::Update,
            &["title".to_string()],
            &payload,
            &query,
            &[("a$1", "text"), ("title", "text")],
            &[],
            &[],
            &Ret::One,
        )
        .expect("sql");
        assert!(
            built.sql.contains(r#""a$1""#),
            "identifier must keep its dollar: {}",
            built.sql
        );
        assert!(
            !built.sql.contains(r#""a$2""#),
            "identifier must not be renumbered: {}",
            built.sql
        );
        assert!(built.sql.contains("$2"));
        assert_eq!(built.params, vec![r#"{"title":"x"}"#, "1"]);
    }
}
