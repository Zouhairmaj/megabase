// Ported from postgrest src/library/PostgREST/Query/QueryBuilder.hs (MIT), pin v16.4,
// postgrest src/library/PostgREST/ApiRequest/Payload.hs (MIT), pin v16.4,
// and postgrest src/library/PostgREST/Response.hs (MIT), pin v16.4.

//! `POST`, `PATCH`, `PUT`, and `DELETE` on `/rest/v1/{relation}`.
//!
//! Default `Prefer` is `return=minimal`: creates answer `201` with an empty
//! body, and the other mutations answer `204`. `Prefer` itself stays
//! unimplemented. Values are bound. Names are `quote_ident` after a catalog
//! lookup.

use std::collections::BTreeSet;

use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::Response;
use serde_json::Value;

use crate::filter::{quote_ident, sql_type_name};
use crate::params::{where_clause, ReadQuery};
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

    let built = match kind {
        Kind::Update if keys.is_empty() => MutationSql {
            sql: format!(
                "SELECT count(*)::bigint FROM (SELECT NULL FROM {}.{} WHERE false) AS _megabase_empty",
                quote_ident(schema),
                quote_ident(relation)
            ),
            params: Vec::new(),
        },
        Kind::Create | Kind::Update | Kind::Upsert => {
            let Some(payload) = payload.as_ref() else {
                return pgrst(
                    StatusCode::BAD_REQUEST,
                    "PGRST102",
                    EMPTY_JSON,
                    None,
                    None,
                );
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
            ) {
                Ok(sql) => sql,
                Err(response) => return *response,
            }
        }
        Kind::Delete => match delete_sql(schema, relation, query, &column_types) {
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
    let mut statement = sqlx::query_scalar::<sqlx::Postgres, i64>(&built.sql);
    for param in &built.params {
        statement = statement.bind(param);
    }
    let count = match statement.fetch_one(&mut *tx).await {
        Ok(count) => count,
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
    mutation_response(&kind, count, schema)
}

struct MutationSql {
    sql: String,
    params: Vec<String>,
}

fn mutation_response(kind: &Kind, count: i64, schema: &str) -> Response {
    let (status, range) = match kind {
        Kind::Create => (StatusCode::CREATED, Some("*/*".to_string())),
        Kind::Update => (StatusCode::NO_CONTENT, Some(content_range(0, count))),
        Kind::Delete => (StatusCode::NO_CONTENT, Some("*/*".to_string())),
        Kind::Upsert => (StatusCode::NO_CONTENT, None),
    };
    let mut response = Response::builder()
        .status(status)
        .body(axum::body::Body::empty())
        .unwrap_or_else(|_| Response::new(axum::body::Body::empty()));
    if let Some(range) = range {
        if let Ok(value) = HeaderValue::from_str(&range) {
            response.headers_mut().insert(header::CONTENT_RANGE, value);
        }
    }
    read::profile_header(&mut response, schema);
    response
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
            let mut sql = format!("WITH inserted AS (INSERT INTO {qualified} ");
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
            sql.push_str(" RETURNING 1) SELECT count(*)::bigint FROM inserted");
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
            let mut sql = format!("WITH updated AS (UPDATE {qualified} SET {sets} FROM {record}");
            if let Some(filter) = where_clause(relation, query, column_types)
                .map_err(|error| Box::new(unsafe_type(&error)))?
            {
                // The filter placeholders were numbered from `$1`, but `$1` is the JSON body.
                let shifted = shift_placeholders(&filter.sql, 1);
                sql.push_str(" WHERE ");
                sql.push_str(&shifted);
                params.extend(filter.params);
            }
            sql.push_str(" RETURNING 1) SELECT count(*)::bigint FROM updated");
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
) -> Result<MutationSql, Box<Response>> {
    let qualified = format!("{}.{}", quote_ident(schema), quote_ident(relation));
    let mut sql = format!("WITH deleted AS (DELETE FROM {qualified}");
    let params = if let Some(filter) = where_clause(relation, query, column_types)
        .map_err(|error| Box::new(unsafe_type(&error)))?
    {
        sql.push_str(" WHERE ");
        sql.push_str(&filter.sql);
        filter.params
    } else {
        Vec::new()
    };
    sql.push_str(" RETURNING 1) SELECT count(*)::bigint FROM deleted");
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
        out.push(byte as char);
        index += 1;
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
         JOIN pg_attribute AS a
           ON a.attrelid = k.conrelid AND a.attnum = ANY (k.conkey)
         JOIN pg_class AS c ON c.oid = k.conrelid
         JOIN pg_namespace AS n ON n.oid = c.relnamespace
         WHERE k.contype = 'p'
           AND n.nspname = $1
           AND c.relname = $2
           AND NOT a.attisdropped
         ORDER BY a.attname",
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
        )
        .expect("sql");
        assert!(
            built.sql.contains("$1::json"),
            "empty object insert must reference the bound body: {}",
            built.sql
        );
        assert!(built.sql.contains("VALUES (1)"));
        assert_eq!(built.params, vec!["{}".to_string()]);
    }

    #[test]
    fn shift_placeholders_leaves_dollars_inside_identifiers() {
        let shifted = shift_placeholders(r#""todos"."a$1" = $1 AND "a""b$2" = $2"#, 1);
        assert_eq!(shifted, r#""todos"."a$1" = $2 AND "a""b$2" = $3"#);

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
