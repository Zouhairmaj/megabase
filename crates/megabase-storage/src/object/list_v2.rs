//! `POST /object/list-v2/:bucketName` (`specs/storage/object.md`).

// Ported from supabase/storage src/http/routes/object/listObjectsV2.ts,
// src/storage/object.ts (`listObjectsV2`, continuation tokens) and
// src/storage/database/pg.ts (`listObjectsV2`) (Apache-2.0), pin v1.80.2.

use std::borrow::Cow;

use axum::{
    body::Bytes,
    extract::{OriginalUri, Path, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use sqlx::{Postgres, QueryBuilder, Row};

use super::{apply_pg, optional_int, optional_str, request_jwt, Actor, Scope};
use crate::{
    fastify_json, iso, limits::is_valid_bucket_name, not_implemented, StorageError, StorageState,
};

/// `ROUTE_OPERATIONS.LIST_OBJECTS_V2`.
const OP_LIST_V2: &str = "storage.object.list_v2";

const VERSION_FILTERS: &[&str] = &["exclude", "include", "only"];
const SORT_COLUMNS: &[&str] = &["name", "updated_at", "created_at"];
const SORT_ORDERS: &[&str] = &["asc", "desc"];

/// `search_v2` in the final (versioning + sort) signature. `id` is text so
/// a folder's null id stays null. `$11` is text so `'infinity'` binds.
const SEARCH_V2_SQL: &str = "SELECT key, name, id::text AS id, updated_at, created_at, \
     last_accessed_at, metadata, version, archived_at, is_delete_marker, is_versioned \
     FROM storage.search_v2($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
     $11::text::timestamptz, $12, $13)";

#[derive(Clone, Copy, PartialEq, Eq)]
enum SqlSort {
    UpdatedAt,
    CreatedAt,
}

impl SqlSort {
    fn expr(self) -> &'static str {
        match self {
            Self::UpdatedAt => {
                "COALESCE(date_trunc('milliseconds', \"updated_at\"), 'epoch'::timestamptz)"
            }
            Self::CreatedAt => {
                "COALESCE(date_trunc('milliseconds', \"created_at\"), 'epoch'::timestamptz)"
            }
        }
    }
}

/// A decoded continuation token. Sort and version filters carry the
/// defaults `decodeContinuationToken` fills in.
struct Cursor {
    start_after: String,
    sort_order: String,
    sort_column: String,
    sort_column_after: Option<String>,
    after_version: Option<String>,
    after_archived_at: Option<String>,
    noncurrent: String,
    delete_markers: String,
    exact_match: String,
}

/// One `listObjectsV2` row, before the folder/object split.
#[derive(Clone)]
struct Listed {
    /// Set for a `search_v2` row. Folders built in Rust leave it unset.
    key: Option<String>,
    from_search_v2: bool,
    name: String,
    id: Option<String>,
    bucket_id: Option<String>,
    updated_at: Option<DateTime<Utc>>,
    created_at: Option<DateTime<Utc>>,
    last_accessed_at: Option<DateTime<Utc>>,
    metadata: Option<Value>,
    version: Option<String>,
    archived_at: Option<DateTime<Utc>>,
    is_delete_marker: Option<bool>,
    is_versioned: Option<bool>,
}

struct ListBody {
    prefix: Option<String>,
    /// `true` when the JSON key was present, so a cursor can tell an omitted
    /// filter from an explicit `false`.
    exact_was_set: bool,
    exact_match: bool,
    limit: Option<i32>,
    cursor: Option<String>,
    with_delimiter: bool,
    sort_column: Option<String>,
    sort_order: Option<String>,
    noncurrent: Option<String>,
    delete_markers: Option<String>,
}

/// A `searchRequestBodySchema` body Megabase serves. A present field of the
/// wrong type is `None` (Fastify's validation message is not ported).
/// Extra keys are ignored.
fn list_v2_body(body: &[u8]) -> Option<ListBody> {
    let body: Value = serde_json::from_slice(body).ok()?;
    let body = body.as_object()?;
    let (exact_match, exact_was_set) = match body.get("exactMatch") {
        None => (false, false),
        Some(Value::Bool(value)) => (*value, true),
        Some(_) => return None,
    };
    let with_delimiter = match body.get("with_delimiter") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => return None,
    };
    let (sort_column, sort_order) = match body.get("sortBy") {
        None => (None, None),
        Some(Value::Object(sort_by)) => {
            if !sort_by.contains_key("column") {
                return None;
            }
            (
                Some(optional_str(sort_by, "column", Some(SORT_COLUMNS)).ok()??),
                optional_str(sort_by, "order", Some(SORT_ORDERS)).ok()?,
            )
        }
        Some(_) => return None,
    };
    let cursor = match body.get("cursor") {
        None => None,
        Some(Value::String(cursor)) => Some(cursor.clone()),
        Some(_) => return None,
    };
    Some(ListBody {
        prefix: optional_str(body, "prefix", None).ok()?,
        exact_was_set,
        exact_match,
        limit: optional_int(body, "limit", 1).ok()?,
        cursor,
        with_delimiter,
        sort_column,
        sort_order,
        noncurrent: optional_str(body, "noncurrentVersions", Some(VERSION_FILTERS)).ok()?,
        delete_markers: optional_str(body, "deleteMarkers", Some(VERSION_FILTERS)).ok()?,
    })
}

fn part_key(line: &str) -> Option<(char, &str)> {
    let mut chars = line.chars();
    let key = chars.next()?;
    if key.is_whitespace() || chars.next() != Some(':') {
        return None;
    }
    Some((key, &line[key.len_utf8() + 1..]))
}

fn allowed_part(key: char, value: &str) -> bool {
    let allowed: &[&str] = match key {
        'o' => &["asc", "desc"],
        'c' => &["name", "created_at", "updated_at"],
        'n' | 'd' => VERSION_FILTERS,
        'e' => &["true", "false"],
        'l' | 'a' | 'v' | 'r' => return true,
        _ => return false,
    };
    allowed.contains(&value)
}

/// `decodeContinuationToken` for the object route (every key, not the S3
/// subset).
fn decode_cursor(token: &str) -> Result<Cursor, ()> {
    let bytes = STANDARD.decode(token).map_err(|_| ())?;
    let text = String::from_utf8(bytes).map_err(|_| ())?;
    let mut cursor = Cursor {
        start_after: String::new(),
        sort_order: "asc".to_string(),
        sort_column: "name".to_string(),
        sort_column_after: None,
        after_version: None,
        after_archived_at: None,
        noncurrent: "exclude".to_string(),
        delete_markers: "exclude".to_string(),
        exact_match: "false".to_string(),
    };
    if text.is_empty() {
        return Err(());
    }
    for line in text.split('\n') {
        let Some((key, value)) = part_key(line) else {
            return Err(());
        };
        if !allowed_part(key, value) {
            return Err(());
        }
        match key {
            'l' => cursor.start_after = value.to_string(),
            'o' => cursor.sort_order = value.to_string(),
            'c' => cursor.sort_column = value.to_string(),
            'a' => cursor.sort_column_after = Some(value.to_string()),
            'v' => cursor.after_version = Some(value.to_string()),
            'r' => cursor.after_archived_at = Some(value.to_string()),
            'n' => cursor.noncurrent = value.to_string(),
            'd' => cursor.delete_markers = value.to_string(),
            'e' => cursor.exact_match = value.to_string(),
            _ => return Err(()),
        }
    }
    Ok(cursor)
}

/// Fields of `encodeContinuationToken`. Default-valued ones are left out.
struct TokenFields<'a> {
    start_after: &'a str,
    sort_order: Option<&'a str>,
    sort_column: Option<&'a str>,
    sort_column_after: Option<&'a str>,
    after_version: Option<&'a str>,
    after_archived_at: Option<&'a str>,
    noncurrent: &'a str,
    delete_markers: &'a str,
    exact_match: bool,
}

/// `encodeContinuationToken`. Default-valued fields are left out.
fn encode_cursor(token: &TokenFields<'_>) -> String {
    let exact = if token.exact_match { "true" } else { "false" };
    let fields = [
        ('l', Some(token.start_after), None),
        ('o', token.sort_order, Some("asc")),
        ('c', token.sort_column, Some("name")),
        ('a', token.sort_column_after, None),
        ('v', token.after_version, None),
        ('r', token.after_archived_at, None),
        ('n', Some(token.noncurrent), Some("exclude")),
        ('d', Some(token.delete_markers), Some("exclude")),
        ('e', Some(exact), Some("false")),
    ];
    let mut raw = String::new();
    for (key, value, default) in fields {
        let Some(value) = value.filter(|value| !value.is_empty()) else {
            continue;
        };
        if default.is_some_and(|default| value == default) {
            continue;
        }
        if !raw.is_empty() {
            raw.push('\n');
        }
        raw.push(key);
        raw.push(':');
        raw.push_str(value);
    }
    STANDARD.encode(raw)
}

fn locked(name: &str, stored: &str, requested: Option<&str>) -> Result<(), StorageError> {
    if requested.is_some_and(|requested| requested != stored) {
        return Err(StorageError::invalid_parameter(Cow::Owned(format!(
            "{name} must match the value used to obtain this continuation token (expected \"{stored}\")"
        ))));
    }
    Ok(())
}

fn invalid_cursor() -> StorageError {
    StorageError::invalid_parameter(Cow::Borrowed("Invalid Parameter continuation token"))
}

/// `escapeLike` from `pg.ts` (backslash first, then `%` and `_`).
fn escape_like(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if c == '\\' {
            out.push_str("\\\\");
        } else if c == '%' || c == '_' {
            out.push('\\');
            out.push(c);
        } else {
            out.push(c);
        }
    }
    out
}

fn sql_sort(column: &str) -> Option<SqlSort> {
    match column {
        "updated_at" => Some(SqlSort::UpdatedAt),
        "created_at" => Some(SqlSort::CreatedAt),
        _ => None,
    }
}

fn prefer<'a>(cursor: Option<&'a str>, request: Option<&'a str>, default: &'a str) -> &'a str {
    cursor
        .filter(|value| !value.is_empty())
        .or(request)
        .unwrap_or(default)
}

struct Resolved<'a> {
    prefix: &'a str,
    limit: i32,
    exact_match: bool,
    with_delimiter: bool,
    sort_column: &'a str,
    sort_order: &'a str,
    noncurrent: &'a str,
    delete_markers: &'a str,
    /// `cursor.startAfter` when a token is present, else none.
    next_token: Option<&'a str>,
    sort_after: Option<&'a str>,
    after_version: Option<&'a str>,
    after_archived_at: Option<&'a str>,
}

fn resolve<'a>(
    body: &'a ListBody,
    cursor: Option<&'a Cursor>,
) -> Result<Resolved<'a>, StorageError> {
    let (noncurrent, delete_markers, exact_stored) = match cursor {
        Some(cursor) => {
            locked(
                "noncurrentVersions",
                &cursor.noncurrent,
                body.noncurrent.as_deref(),
            )?;
            locked(
                "deleteMarkers",
                &cursor.delete_markers,
                body.delete_markers.as_deref(),
            )?;
            locked(
                "exactMatch",
                &cursor.exact_match,
                body.exact_was_set
                    .then_some(if body.exact_match { "true" } else { "false" }),
            )?;
            (
                cursor.noncurrent.as_str(),
                cursor.delete_markers.as_str(),
                cursor.exact_match.as_str(),
            )
        }
        None => (
            body.noncurrent.as_deref().unwrap_or("exclude"),
            body.delete_markers.as_deref().unwrap_or("exclude"),
            if body.exact_match { "true" } else { "false" },
        ),
    };
    let exact_match = exact_stored == "true";
    if exact_match
        && cursor.is_some_and(|cursor| {
            body.prefix
                .as_ref()
                .is_some_and(|prefix| prefix != &cursor.start_after)
        })
    {
        let expected = cursor
            .map(|cursor| cursor.start_after.as_str())
            .unwrap_or("");
        return Err(StorageError::invalid_parameter(Cow::Owned(format!(
            "prefix must match the value used to obtain this continuation token (expected \"{expected}\")"
        ))));
    }
    let sort_column = prefer(
        cursor.map(|cursor| cursor.sort_column.as_str()),
        body.sort_column.as_deref(),
        "name",
    );
    let sort_order = prefer(
        cursor.map(|cursor| cursor.sort_order.as_str()),
        body.sort_order.as_deref(),
        "asc",
    );
    // `Math.min(maxKeys || 1000, 1000)`.
    let limit = body.limit.unwrap_or(1000).min(1000);
    Ok(Resolved {
        prefix: body.prefix.as_deref().unwrap_or(""),
        limit,
        exact_match,
        with_delimiter: body.with_delimiter,
        sort_column,
        sort_order,
        noncurrent,
        delete_markers,
        next_token: cursor.map(|cursor| cursor.start_after.as_str()),
        sort_after: cursor.and_then(|cursor| cursor.sort_column_after.as_deref()),
        after_version: cursor.and_then(|cursor| cursor.after_version.as_deref()),
        after_archived_at: cursor.and_then(|cursor| cursor.after_archived_at.as_deref()),
    })
}

fn read_row(row: &sqlx::postgres::PgRow, from_search_v2: bool) -> Result<Listed, sqlx::Error> {
    let name = row
        .try_get::<Option<String>, _>("name")?
        .unwrap_or_default();
    Ok(Listed {
        key: if from_search_v2 {
            Some(row.try_get::<Option<String>, _>("key")?.unwrap_or_default())
        } else {
            None
        },
        from_search_v2,
        id: row.try_get("id")?,
        name,
        bucket_id: None,
        updated_at: row.try_get("updated_at")?,
        created_at: row.try_get("created_at")?,
        last_accessed_at: row.try_get("last_accessed_at")?,
        metadata: row.try_get("metadata")?,
        version: row.try_get("version")?,
        archived_at: row.try_get("archived_at")?,
        is_delete_marker: row.try_get("is_delete_marker")?,
        is_versioned: row.try_get("is_versioned")?,
    })
}

fn js_units(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

fn js_slice(value: &str, start: usize, end: Option<usize>) -> String {
    let units = js_units(value);
    let start = start.min(units.len());
    let end = end.unwrap_or(units.len()).min(units.len());
    if start >= end {
        return String::new();
    }
    String::from_utf16_lossy(&units[start..end])
}

fn js_find(haystack: &str, needle: &str) -> Option<usize> {
    let hay = js_units(haystack);
    let needle = js_units(needle);
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len())
        .position(|window| window == needle.as_slice())
}

/// The delimiter collapse in `ObjectStorage.listObjectsV2`.
fn collapse(rows: Vec<Listed>, prefix: &str, bucket_id: &str) -> Vec<Listed> {
    let prefix_len = js_units(prefix).len();
    let mut out = Vec::with_capacity(rows.len());
    let mut previous = String::new();
    for object in rows {
        let rest = js_slice(&object.name, prefix_len, None);
        if let Some(idx) = js_find(&rest, "/") {
            let end = prefix_len + idx + 1;
            let current = js_slice(&object.name, 0, Some(end));
            if current == previous {
                continue;
            }
            previous = current.clone();
            out.push(Listed {
                key: None,
                from_search_v2: false,
                name: current,
                id: None,
                bucket_id: Some(bucket_id.to_string()),
                updated_at: object.updated_at,
                created_at: object.created_at,
                last_accessed_at: object.last_accessed_at,
                metadata: None,
                version: None,
                archived_at: None,
                is_delete_marker: None,
                is_versioned: None,
            });
        } else {
            out.push(object);
        }
    }
    out
}

fn timestamp(at: Option<DateTime<Utc>>) -> Value {
    at.map_or(Value::Null, |at| json!(iso(Some(at))))
}

fn object_json(row: &Listed) -> Value {
    let mut out = Map::new();
    if row.from_search_v2 {
        out.insert("key".into(), json!(row.key.as_deref().unwrap_or("")));
    }
    if row.from_search_v2 {
        out.insert("name".into(), json!(row.name.as_str()));
        out.insert("id".into(), json!(row.id));
        out.insert("updated_at".into(), timestamp(row.updated_at));
        out.insert("created_at".into(), timestamp(row.created_at));
        out.insert("last_accessed_at".into(), timestamp(row.last_accessed_at));
        out.insert("metadata".into(), json!(row.metadata));
    } else {
        out.insert("id".into(), json!(row.id));
        out.insert("name".into(), json!(row.name.as_str()));
        out.insert("metadata".into(), json!(row.metadata));
        out.insert("updated_at".into(), timestamp(row.updated_at));
        out.insert("created_at".into(), timestamp(row.created_at));
        out.insert("last_accessed_at".into(), timestamp(row.last_accessed_at));
    }
    out.insert("version".into(), json!(row.version));
    out.insert("archived_at".into(), timestamp(row.archived_at));
    out.insert("is_delete_marker".into(), json!(row.is_delete_marker));
    out.insert("is_versioned".into(), json!(row.is_versioned));
    Value::Object(out)
}

fn folder_json(row: &Listed) -> Value {
    json!({
        "id": null,
        "name": row.name,
        "bucket_id": row.bucket_id.as_deref().unwrap_or(""),
        "updated_at": timestamp(row.updated_at),
        "created_at": timestamp(row.created_at),
        "last_accessed_at": timestamp(row.last_accessed_at),
    })
}

fn page_body(
    rows: &[Listed],
    token: Option<&str>,
    token_key: Option<&str>,
    delimiter: bool,
) -> Value {
    let mut folders = Vec::new();
    let mut objects = Vec::new();
    for row in rows {
        let name = if delimiter && row.id.is_none() && !row.name.ends_with('/') {
            format!("{}/", row.name)
        } else {
            row.name.clone()
        };
        let row = Listed {
            name,
            ..row.clone()
        };
        // id null is a folder, including a collapsed prefix.
        if row.id.is_none() {
            folders.push(folder_json(&row));
        } else {
            objects.push(object_json(&row));
        }
    }
    let mut out = Map::new();
    let truncated = token.is_some();
    out.insert("hasNext".into(), json!(truncated));
    if let Some(token) = token {
        out.insert("nextCursor".into(), json!(token));
        out.insert("nextCursorKey".into(), json!(token_key.unwrap_or("")));
    }
    out.insert("folders".into(), Value::Array(folders));
    out.insert("objects".into(), Value::Array(objects));
    Value::Object(out)
}

fn versioning(noncurrent: &str, delete_markers: &str, sql: &mut QueryBuilder<'_, Postgres>) {
    if noncurrent == "only" {
        sql.push(" AND archived_at IS NOT NULL");
    } else if noncurrent != "include" {
        sql.push(" AND archived_at IS NULL");
    }
    if delete_markers == "only" {
        sql.push(" AND is_delete_marker");
    } else if delete_markers != "include" {
        sql.push(" AND NOT is_delete_marker");
    }
}

fn push_flat(query: &Resolved<'_>, bucket_id: &str) -> QueryBuilder<'static, Postgres> {
    let mut sql: QueryBuilder<'static, Postgres> = QueryBuilder::new(
        "SELECT id::text AS id, name, metadata, updated_at, created_at, last_accessed_at, \
         version, archived_at, is_delete_marker, is_versioned \
         FROM storage.objects WHERE bucket_id = ",
    );
    sql.push_bind(bucket_id.to_string());
    versioning(query.noncurrent, query.delete_markers, &mut sql);
    let sort = sql_sort(query.sort_column);
    let asc = query.sort_order != "desc";
    let op = if asc { ">" } else { "<" };
    let multi = matches!(query.noncurrent, "include" | "only");
    if query.exact_match {
        let name = query.next_token.unwrap_or(query.prefix);
        sql.push(" AND name COLLATE \"C\" = ");
        sql.push_bind(name.to_string());
    } else if !query.prefix.is_empty() {
        sql.push(" AND name COLLATE \"C\" LIKE ");
        sql.push_bind(format!("{}%", escape_like(query.prefix)));
    }
    if let Some(token) = query.next_token {
        push_seek(&mut sql, query, token, sort, op, asc, multi);
    }
    sql.push(" ORDER BY ");
    if let Some(sort) = sort {
        sql.push(sort.expr());
        sql.push(if asc { " ASC, " } else { " DESC, " });
    }
    sql.push("name COLLATE \"C\" ");
    sql.push(if asc { "ASC" } else { "DESC" });
    if sort.is_none() && multi {
        sql.push(
            ", COALESCE(date_trunc('milliseconds', archived_at), 'infinity'::timestamptz) DESC, \
             COALESCE(version, '') ASC",
        );
    }
    if sort.is_some() && multi {
        sql.push(", COALESCE(version, '') ");
        sql.push(if asc { "ASC" } else { "DESC" });
    }
    sql.push(" LIMIT ");
    sql.push_bind(query.limit + 1);
    sql
}

fn push_seek(
    sql: &mut QueryBuilder<'static, Postgres>,
    query: &Resolved<'_>,
    token: &str,
    sort: Option<SqlSort>,
    op: &str,
    asc: bool,
    multi: bool,
) {
    if let Some(sort) = sort {
        sql.push(" AND ROW(");
        sql.push(sort.expr());
        sql.push(", name COLLATE \"C\", ");
        if multi {
            sql.push("COALESCE(version, '')");
        } else {
            sql.push("''");
        }
        sql.push(") ");
        sql.push(op);
        sql.push(" ROW(date_trunc('milliseconds', COALESCE(NULLIF(");
        sql.push_bind(query.sort_after.map(str::to_string));
        sql.push("::text, '')::timestamptz, 'epoch'::timestamptz)), ");
        sql.push_bind(token.to_string());
        sql.push(", ");
        if multi {
            sql.push_bind(query.after_version.unwrap_or("").to_string());
        } else {
            sql.push("''");
        }
        sql.push(")");
        return;
    }
    if multi {
        let name_op = if asc { ">=" } else { "<=" };
        sql.push(" AND name COLLATE \"C\" ");
        sql.push(name_op);
        sql.push(" ");
        sql.push_bind(token.to_string());
        sql.push(" AND (name COLLATE \"C\" ");
        sql.push(op);
        sql.push(" ");
        sql.push_bind(token.to_string());
        sql.push(" OR (name COLLATE \"C\" = ");
        sql.push_bind(token.to_string());
        sql.push(" AND (NULLIF(");
        sql.push_bind(query.after_archived_at.unwrap_or("").to_string());
        sql.push(
            "::text, '') IS NULL OR COALESCE(date_trunc('milliseconds', archived_at), \
             'infinity'::timestamptz) < NULLIF(",
        );
        sql.push_bind(query.after_archived_at.unwrap_or("").to_string());
        sql.push(
            "::text, '')::timestamptz OR (COALESCE(date_trunc('milliseconds', archived_at), \
             'infinity'::timestamptz) = NULLIF(",
        );
        sql.push_bind(query.after_archived_at.unwrap_or("").to_string());
        sql.push("::text, '')::timestamptz AND COALESCE(version, '') > ");
        sql.push_bind(query.after_version.unwrap_or("").to_string());
        sql.push("))))");
        return;
    }
    sql.push(" AND name COLLATE \"C\" ");
    sql.push(op);
    sql.push(" ");
    sql.push_bind(token.to_string());
}

async fn fetch_rows(
    scope: &Scope<'_>,
    pool: &sqlx::PgPool,
    bucket_id: &str,
    query: &Resolved<'_>,
) -> Result<Vec<Listed>, super::ApplyError> {
    let mut tx = scope.begin(pool).await?;
    let rows = if query.with_delimiter && !query.exact_match {
        let levels = i32::try_from(query.prefix.split('/').count()).unwrap_or(i32::MAX);
        let sort_order = if query.sort_order == "desc" {
            "desc"
        } else {
            "asc"
        };
        let sort_column = match query.sort_column {
            "updated_at" | "created_at" => query.sort_column,
            _ => "name",
        };
        // `sortBy.after || null`: an empty tie-break is null.
        let sort_after = query.sort_after.filter(|value| !value.is_empty());
        let after_archived = query.after_archived_at.filter(|value| !value.is_empty());
        let continuation = query.next_token.is_some();
        let start_after = query.next_token.unwrap_or("");
        sqlx::query(SEARCH_V2_SQL)
            .bind(query.prefix)
            .bind(bucket_id)
            .bind(query.limit + 1)
            .bind(levels)
            .bind(start_after)
            .bind(sort_order)
            .bind(sort_column)
            .bind(sort_after)
            .bind(query.noncurrent)
            .bind(query.delete_markers)
            .bind(after_archived)
            .bind(query.after_version.unwrap_or(""))
            .bind(continuation)
            .fetch_all(&mut *tx)
            .await?
    } else {
        push_flat(query, bucket_id)
            .build()
            .fetch_all(&mut *tx)
            .await?
    };
    tx.commit().await?;
    let from_search = query.with_delimiter && !query.exact_match;
    rows.iter()
        .map(|row| read_row(row, from_search).map_err(super::ApplyError::from))
        .collect()
}

fn next_token(row: &Listed, query: &Resolved<'_>) -> (String, String) {
    let multi = matches!(query.noncurrent, "include" | "only");
    let sort_column = match query.sort_column {
        "updated_at" | "created_at" => Some(query.sort_column),
        _ => None,
    };
    let tie = sort_column.and_then(|column| match column {
        "updated_at" => row.updated_at,
        "created_at" => row.created_at,
        _ => None,
    });
    let sort_after = tie.map(|at| iso(Some(at)));
    let after_version = multi.then(|| row.version.clone()).flatten();
    let after_archived = multi.then(|| match row.archived_at {
        Some(at) => iso(Some(at)),
        None => "infinity".to_string(),
    });
    let token = encode_cursor(&TokenFields {
        start_after: &row.name,
        sort_order: Some(query.sort_order),
        sort_column: Some(query.sort_column),
        sort_column_after: sort_after.as_deref(),
        after_version: after_version.as_deref(),
        after_archived_at: after_archived.as_deref(),
        noncurrent: query.noncurrent,
        delete_markers: query.delete_markers,
        exact_match: query.exact_match,
    });
    (token, row.name.clone())
}

/// `POST /object/list-v2/:bucketName`.
// megabase:unit storage:route:POST /storage/v1/object/list-v2/{bucketName}
pub(crate) async fn list_objects_v2(
    State(state): State<StorageState>,
    Path(bucket_id): Path<String>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(body) = list_v2_body(&body) else {
        return not_implemented(&method, &uri);
    };
    let Some((claims, jwt)) = request_jwt(&state, &headers) else {
        return not_implemented(&method, &uri);
    };
    if !is_valid_bucket_name(&bucket_id) {
        return StorageError::invalid_bucket_name().into_response();
    }
    let cursor = match body.cursor.as_deref() {
        Some(token) => match decode_cursor(token) {
            Ok(cursor) => Some(cursor),
            Err(()) => return invalid_cursor().into_response(),
        },
        None => None,
    };
    let query = match resolve(&body, cursor.as_ref()) {
        Ok(query) => query,
        Err(error) => return error.into_response(),
    };
    let Some(pool) = state.pool.as_ref() else {
        return not_implemented(&method, &uri);
    };
    let scope = Scope {
        actor: Actor::Request {
            claims: &claims,
            jwt,
        },
        method: &method,
        uri: &uri,
        headers: &headers,
        operation: OP_LIST_V2,
        roles: &state.api_roles,
    };
    let fetched = match fetch_rows(&scope, pool, &bucket_id, &query).await {
        Ok(rows) => rows,
        Err(error) => return apply_pg(error, StorageError::resource_already_exists),
    };
    let rows = if query.with_delimiter && !query.exact_match {
        collapse(fetched, query.prefix, &bucket_id)
    } else {
        fetched
    };
    let limit = usize::try_from(query.limit).unwrap_or(usize::MAX);
    let truncated = rows.len() > limit;
    let count = if truncated { limit } else { rows.len() };
    let (token, token_key) = truncated
        .then(|| next_token(&rows[count - 1], &query))
        .unzip();
    fastify_json(
        StatusCode::OK,
        &page_body(
            &rows[..count],
            token.as_deref(),
            token_key.as_deref(),
            query.with_delimiter,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_tokens_match_storage_api() {
        let token = |start_after,
                     sort_order,
                     sort_column,
                     sort_column_after,
                     after_version,
                     after_archived_at,
                     noncurrent,
                     delete_markers,
                     exact_match| {
            encode_cursor(&TokenFields {
                start_after,
                sort_order,
                sort_column,
                sort_column_after,
                after_version,
                after_archived_at,
                noncurrent,
                delete_markers,
                exact_match,
            })
        };
        assert_eq!(
            token(
                "photos/a.png",
                None,
                None,
                None,
                None,
                None,
                "exclude",
                "exclude",
                false
            ),
            "bDpwaG90b3MvYS5wbmc="
        );
        assert_eq!(
            token(
                "a/b",
                Some("desc"),
                Some("updated_at"),
                Some("2020-01-02T03:04:05.006Z"),
                None,
                None,
                "exclude",
                "exclude",
                false,
            ),
            "bDphL2IKbzpkZXNjCmM6dXBkYXRlZF9hdAphOjIwMjAtMDEtMDJUMDM6MDQ6MDUuMDA2Wg=="
        );
        assert_eq!(
            token(
                "k",
                Some("asc"),
                Some("name"),
                None,
                Some("ver"),
                Some("infinity"),
                "include",
                "only",
                true,
            ),
            "bDprCnY6dmVyCnI6aW5maW5pdHkKbjppbmNsdWRlCmQ6b25seQplOnRydWU="
        );
        let cursor = decode_cursor("bDpwaG90b3MvYS5wbmc=").unwrap();
        assert_eq!(cursor.start_after, "photos/a.png");
        assert_eq!(cursor.sort_order, "asc");
        assert_eq!(cursor.sort_column, "name");
        assert_eq!(cursor.noncurrent, "exclude");
        assert_eq!(cursor.exact_match, "false");
        assert!(decode_cursor("!!!").is_err());
        assert!(decode_cursor(&STANDARD.encode("x:1")).is_err());
        assert!(decode_cursor(&STANDARD.encode("o:sideways")).is_err());
    }

    #[test]
    fn list_v2_bodies_follow_the_schema() {
        let body = list_v2_body(
            br#"{"prefix":"a","limit":10,"with_delimiter":true,"sortBy":{"column":"name","order":"desc"}}"#,
        )
        .unwrap();
        assert_eq!(body.prefix.as_deref(), Some("a"));
        assert_eq!(body.limit, Some(10));
        assert!(body.with_delimiter);
        assert_eq!(body.sort_column.as_deref(), Some("name"));
        assert_eq!(body.sort_order.as_deref(), Some("desc"));
        assert!(!body.exact_match);
        assert!(list_v2_body(b"{}").is_some());
        for raw in [
            b"[]".as_slice(),
            b"not json",
            br#"{"limit":0}"#,
            br#"{"limit":1.5}"#,
            br#"{"sortBy":{"order":"asc"}}"#,
            br#"{"sortBy":{"column":"last_accessed_at"}}"#,
            br#"{"with_delimiter":"true"}"#,
            br#"{"noncurrentVersions":"all"}"#,
            br#"{"cursor":1}"#,
        ] {
            assert!(
                list_v2_body(raw).is_none(),
                "{}",
                String::from_utf8_lossy(raw)
            );
        }
    }

    #[test]
    fn delimiter_collapse_keeps_one_folder() {
        let file = |name: &str| Listed {
            key: Some(name.to_string()),
            from_search_v2: true,
            name: name.to_string(),
            id: Some("id".to_string()),
            bucket_id: None,
            updated_at: None,
            created_at: None,
            last_accessed_at: None,
            metadata: None,
            version: None,
            archived_at: None,
            is_delete_marker: Some(false),
            is_versioned: Some(false),
        };
        let rows = collapse(
            vec![
                file("photos/a.jpg"),
                file("photos/b.jpg"),
                file("notes.txt"),
            ],
            "",
            "bucket",
        );
        assert_eq!(rows.len(), 2);
        assert!(rows[0].id.is_none());
        assert_eq!(rows[0].name, "photos/");
        assert_eq!(rows[0].bucket_id.as_deref(), Some("bucket"));
        assert_eq!(rows[1].name, "notes.txt");
        assert!(rows[1].id.is_some());
    }

    #[test]
    fn a_changed_version_filter_is_rejected() {
        let body = list_v2_body(br#"{"noncurrentVersions":"include"}"#).unwrap();
        let cursor = decode_cursor("bDpwaG90b3MvYS5wbmc=").unwrap();
        let Err(error) = resolve(&body, Some(&cursor)) else {
            panic!("a changed filter must be rejected");
        };
        assert_eq!(error.code, "InvalidParameter");
        assert!(error.message.contains("noncurrentVersions"));
    }
}
