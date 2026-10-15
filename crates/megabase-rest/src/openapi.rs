// Ported from postgrest src/library/PostgREST/Response/OpenAPI.hs (MIT), pin v16.4,
// postgrest src/library/PostgREST/SchemaCache.hs (MIT), pin v16.4,
// postgrest src/library/PostgREST/Query/SqlFragment.hs (MIT), pin v16.4,
// and postgrest src/library/PostgREST/Response.hs (MIT), pin v16.4.

//! `GET` and `HEAD /rest/v1/`: the OpenAPI 2.0 document for one schema.
//!
//! `openapi-mode` is `follow-privileges`, so the table and function lists
//! run after `SET ROLE`. `server-host` `!4` and port `3000` match the
//! PostgREST defaults shipped with the pinned Supabase stack; `escapeHostName`
//! (`Network.hs`) writes `!4` as `0.0.0.0` in `host`, and
//! `openapi-security-active` is off, so no security definitions are emitted.
//!
//! The `swagger2` encoder omits a field whose value is empty (`Nothing`, an
//! empty list or an empty map), so this module only inserts a key when it
//! has content.

use std::collections::BTreeMap;

use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Map, Value};

use crate::read::{self, begin, pgrst, query_failure, set_request_context, Session, JSON_UTF8};

const OPENAPI_UTF8: &str = "application/openapi+json; charset=utf-8";
const PRETTY_VERSION: &str = "16.4";
const DOCS_VERSION: &str = "v16";

/// Domains resolve to their innermost base type (`baseTypesCte`, PG < 17
/// variant; it gives the same rows as `pg_basetype()` on 17).
const BASE_TYPES_CTE: &str = "base_types AS (
    WITH RECURSIVE
    recurse AS (
      SELECT
        oid,
        typbasetype,
        typnamespace AS base_namespace,
        COALESCE(NULLIF(typbasetype, 0), oid) AS base_type
      FROM pg_type
      UNION
      SELECT
        t.oid,
        b.typbasetype,
        b.typnamespace AS base_namespace,
        COALESCE(NULLIF(b.typbasetype, 0), b.oid) AS base_type
      FROM recurse t
      JOIN pg_type b ON t.typbasetype = b.oid
    )
    SELECT
      oid,
      base_namespace,
      base_type
    FROM recurse
    WHERE typbasetype = 0
  )";

struct TableDoc {
    name: String,
    description: Option<String>,
    insertable: bool,
    updatable: bool,
    deletable: bool,
    columns: Vec<ColumnDoc>,
}

#[derive(Default)]
struct ColumnDoc {
    name: String,
    /// `data_type` from the schema cache: no type modifier for `pg_catalog`
    /// types, and a domain over a `pg_catalog` type reports its base type.
    pg_type: String,
    nullable: bool,
    max_length: Option<i32>,
    default: Option<String>,
    enum_values: Vec<String>,
    primary_key: bool,
    /// First single-column foreign key: (foreign table, foreign column).
    foreign_key: Option<(String, String)>,
    description: Option<String>,
}

struct RoutineDoc {
    name: String,
    description: Option<String>,
    volatile: bool,
    params: Vec<RoutineParam>,
}

struct RoutineParam {
    name: String,
    pg_type: String,
    required: bool,
    variadic: bool,
}

/// OpenAPI document for `schema`, or an error response.
pub(crate) async fn document(
    pool: &sqlx::PgPool,
    schema: &str,
    session: &Session,
    headers_only: bool,
    headers: &HeaderMap,
) -> Response {
    let content_type = match openapi_media(headers) {
        Ok(content_type) => content_type,
        Err(response) => return *response,
    };
    let mut tx = match begin(pool).await {
        Ok(tx) => tx,
        Err(response) => return *response,
    };
    if let Err(response) = set_request_context(&mut tx, session, "GET", "/", schema).await {
        return *response;
    }
    let comment = match schema_comment(&mut tx, schema, session.anon).await {
        Ok(comment) => comment,
        Err(response) => return *response,
    };
    let tables = match load_tables(&mut tx, schema, session.anon).await {
        Ok(tables) => tables,
        Err(response) => return *response,
    };
    let routines = match load_routines(&mut tx, schema, session.anon).await {
        Ok(routines) => routines,
        Err(response) => return *response,
    };
    if let Err(error) = tx.commit().await {
        return query_failure(&error, session.anon);
    }
    let body = swagger_document(comment.as_deref(), &tables, &routines);
    let encoded = body.to_string();
    let mut response = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, content_type)],
        if headers_only { String::new() } else { encoded },
    )
        .into_response();
    read::profile_header(&mut response, schema);
    response
}

fn openapi_media(headers: &HeaderMap) -> Result<&'static str, Box<Response>> {
    let Some(value) = headers.get(header::ACCEPT) else {
        return Ok(OPENAPI_UTF8);
    };
    let text = value.to_str().unwrap_or("");
    if text.trim().is_empty() {
        return Ok(OPENAPI_UTF8);
    }
    // The root serves only the OpenAPI and JSON types. The request is
    // acceptable when any entry with a non-zero quality names one of them.
    let mut json = false;
    for entry in text.split(',') {
        let mut pieces = entry.split(';');
        let essence = pieces.next().unwrap_or("").trim().to_ascii_lowercase();
        let zero_quality = pieces.any(|param| {
            let compact: String = param.split_whitespace().collect();
            compact
                .split_once('=')
                .filter(|(name, _)| name.eq_ignore_ascii_case("q"))
                .is_some_and(|(_, number)| number.parse::<f32>().is_ok_and(|q| q <= 0.0))
        });
        if zero_quality {
            continue;
        }
        match essence.as_str() {
            // megabase:unit rest:media-type:application/openapi+json
            "application/openapi+json" | "*/*" | "" => return Ok(OPENAPI_UTF8),
            "application/json" | "application/*" => json = true,
            _ => {}
        }
    }
    if json {
        return Ok(JSON_UTF8);
    }
    let first = text.split(',').next().unwrap_or("");
    let essence = first
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    Err(Box::new(pgrst(
        StatusCode::NOT_ACCEPTABLE,
        "PGRST107",
        &format!("None of these media types are available: {essence}"),
        None,
        None,
    )))
}

async fn schema_comment(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    schema: &str,
    anon: bool,
) -> Result<Option<String>, Box<Response>> {
    let comment: Option<Option<String>> = sqlx::query_scalar(
        "SELECT pg_catalog.obj_description(n.oid, 'pg_namespace')
         FROM pg_namespace AS n
         WHERE n.nspname = $1",
    )
    .bind(schema)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| Box::new(query_failure(&error, anon)))?;
    Ok(comment.flatten())
}

type ColumnRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    bool,
    String,
    Option<i32>,
    Vec<String>,
);

async fn load_tables(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    schema: &str,
    anon: bool,
) -> Result<Vec<TableDoc>, Box<Response>> {
    // `accessibleTables` joined with the flags of `tablesSqlQuery`.
    let rows: Vec<(String, Option<String>, bool, bool, bool)> = sqlx::query_as(
        "SELECT c.relname::text,
                d.description,
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
                )
         FROM pg_class AS c
         JOIN pg_namespace AS n ON n.oid = c.relnamespace
         LEFT JOIN pg_description AS d
           ON d.objoid = c.oid AND d.objsubid = 0 AND d.classoid = 'pg_class'::regclass
         WHERE n.nspname = $1
           AND c.relkind IN ('v', 'r', 'm', 'f', 'p')
           AND NOT c.relispartition
           AND (
             pg_has_role(c.relowner, 'USAGE')
             OR has_table_privilege(c.oid, 'SELECT, INSERT, UPDATE, DELETE, TRUNCATE, REFERENCES, TRIGGER')
             OR has_any_column_privilege(c.oid, 'SELECT, INSERT, UPDATE, REFERENCES')
           )
         ORDER BY c.relname",
    )
    .bind(schema)
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| Box::new(query_failure(&error, anon)))?;

    // The `columns` CTE of `tablesSqlQuery`.
    let columns: Vec<ColumnRow> = sqlx::query_as(&format!(
        "WITH {BASE_TYPES_CTE}
         SELECT
           c.relname::text,
           a.attname::text,
           d.description,
           CASE
             WHEN (t.typbasetype != 0) AND (ad.adbin IS NULL) THEN pg_get_expr(t.typdefaultbin, 0)
             WHEN a.attidentity = 'd' THEN format('nextval(%L)', seq.objid::regclass)
             WHEN a.attgenerated = 's' THEN null
             ELSE pg_get_expr(ad.adbin, ad.adrelid)::text
           END AS column_default,
           NOT (a.attnotnull OR t.typtype = 'd' AND t.typnotnull) AS is_nullable,
           CASE
             WHEN t.typtype = 'd' THEN
               CASE
                 WHEN bt.base_namespace = 'pg_catalog'::regnamespace THEN format_type(bt.base_type, NULL::integer)
                 ELSE format_type(a.atttypid, a.atttypmod)
               END
             ELSE
               CASE
                 WHEN t.typnamespace = 'pg_catalog'::regnamespace THEN format_type(a.atttypid, NULL::integer)
                 ELSE format_type(a.atttypid, a.atttypmod)
               END
           END::text AS data_type,
           information_schema._pg_char_max_length(
             information_schema._pg_truetypid(a.*, t.*),
             information_schema._pg_truetypmod(a.*, t.*)
           )::integer AS character_maximum_length,
           COALESCE(
             (SELECT array_agg(enumlabel::text ORDER BY enumsortorder) FROM pg_enum WHERE enumtypid = bt.base_type),
             '{{}}'
           )::text[] AS enum_labels
         FROM pg_attribute a
         LEFT JOIN pg_description AS d
           ON d.objoid = a.attrelid AND d.objsubid = a.attnum AND d.classoid = 'pg_class'::regclass
         LEFT JOIN pg_attrdef ad
           ON a.attrelid = ad.adrelid AND a.attnum = ad.adnum
         JOIN pg_class c ON a.attrelid = c.oid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         JOIN pg_type t ON a.atttypid = t.oid
         LEFT JOIN base_types bt ON t.oid = bt.oid
         LEFT JOIN pg_depend seq
           ON seq.refobjid = a.attrelid AND seq.refobjsubid = a.attnum AND seq.deptype = 'i'
         WHERE n.nspname = $1
           AND a.attnum > 0
           AND NOT a.attisdropped
           AND c.relkind IN ('r', 'v', 'f', 'm', 'p')
           AND NOT c.relispartition
         ORDER BY c.relname, a.attnum"
    ))
    .bind(schema)
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| Box::new(query_failure(&error, anon)))?;

    // `tbl_pk_cols`: primary keys of tables (views get none here).
    let primary_keys: Vec<(String, String)> = sqlx::query_as(
        "SELECT r.relname::text, a.attname::text
         FROM pg_class r
         JOIN pg_namespace n ON n.oid = r.relnamespace
         JOIN pg_constraint c ON r.oid = c.conrelid
         JOIN pg_attribute a ON a.attrelid = r.oid AND a.attnum = ANY (c.conkey)
         WHERE c.contype IN ('p')
           AND r.relkind IN ('r', 'p')
           AND n.nspname = $1
           AND NOT a.attisdropped",
    )
    .bind(schema)
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| Box::new(query_failure(&error, anon)))?;

    // Single-column M2O and O2O relationships from `allM2OandO2ORels` whose
    // foreign table lives in the same schema (the `RelationshipsMap` key).
    // The order is the derived `Ord` of `Relationship`: foreign table,
    // self flag, then M2O before O2O, then constraint name.
    let foreign_keys: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT tab.relname::text, cols.attname::text, other.relname::text, refs.attname::text
         FROM pg_constraint traint
         JOIN pg_namespace ns1 ON ns1.oid = traint.connamespace
         JOIN pg_class tab ON tab.oid = traint.conrelid
         JOIN pg_class other ON other.oid = traint.confrelid
         JOIN pg_namespace ns2 ON ns2.oid = other.relnamespace
         JOIN pg_attribute cols ON cols.attrelid = traint.conrelid AND cols.attnum = traint.conkey[1]
         JOIN pg_attribute refs ON refs.attrelid = traint.confrelid AND refs.attnum = traint.confkey[1]
         WHERE traint.contype = 'f'
           AND traint.conparentid = 0
           AND cardinality(traint.conkey) = 1
           AND ns1.nspname = $1
           AND ns2.nspname = $1
         ORDER BY
           tab.relname COLLATE \"C\",
           other.relname COLLATE \"C\",
           traint.conrelid = traint.confrelid,
           EXISTS (
             SELECT 1
             FROM pg_constraint u
             WHERE u.conrelid = traint.conrelid
               AND u.contype IN ('p', 'u')
               AND u.connamespace <> 'pg_catalog'::regnamespace
               AND u.conkey = traint.conkey
           ),
           traint.conname COLLATE \"C\"",
    )
    .bind(schema)
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| Box::new(query_failure(&error, anon)))?;

    let mut tables = Vec::new();
    for (name, description, insertable, updatable, deletable) in rows {
        let cols = columns
            .iter()
            .filter(|row| row.0 == name)
            .map(
                |(_, column, description, default, nullable, pg_type, max_length, labels)| {
                    ColumnDoc {
                        name: column.clone(),
                        pg_type: pg_type.clone(),
                        nullable: *nullable,
                        max_length: *max_length,
                        default: default.clone(),
                        enum_values: labels.clone(),
                        primary_key: primary_keys
                            .iter()
                            .any(|(table, key)| table == &name && key == column),
                        foreign_key: foreign_keys
                            .iter()
                            .find(|(table, key, ..)| table == &name && key == column)
                            .map(|(_, _, other, other_column)| {
                                (other.clone(), other_column.clone())
                            }),
                        description: description.clone(),
                    }
                },
            )
            .collect();
        tables.push(TableDoc {
            name,
            description,
            insertable,
            updatable,
            deletable,
            columns: cols,
        });
    }
    Ok(tables)
}

/// `accessibleFuncs`: functions in `schema` the current role may execute.
async fn load_routines(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    schema: &str,
    anon: bool,
) -> Result<Vec<RoutineDoc>, Box<Response>> {
    let rows: Vec<(String, Option<String>, String, String)> = sqlx::query_as(&format!(
        "WITH {BASE_TYPES_CTE},
         arguments AS (
           SELECT
             oid,
             json_agg(json_build_array(
               COALESCE(name, ''),
               type::regtype::text,
               idx <= (pronargs - pronargdefaults),
               COALESCE(mode = 'v', FALSE)
             ) ORDER BY idx) AS args,
             CASE COUNT(*) - COUNT(name)
               WHEN 0 THEN true
               WHEN 1 THEN (array_agg(type))[1] IN ('bytea'::regtype, 'json'::regtype, 'jsonb'::regtype, 'text'::regtype, 'xml'::regtype)
               ELSE false
             END AS callable
           FROM pg_proc,
                unnest(proargnames, proargtypes, proargmodes)
                  WITH ORDINALITY AS _ (name, type, mode, idx)
           WHERE type IS NOT NULL
           GROUP BY oid
         )
         SELECT
           p.proname::text,
           d.description,
           COALESCE(a.args, '[]'::json)::text,
           p.provolatile::text
         FROM pg_proc p
         LEFT JOIN arguments a ON a.oid = p.oid
         JOIN pg_namespace pn ON pn.oid = p.pronamespace
         JOIN base_types bt ON bt.oid = p.prorettype
         JOIN pg_type t ON t.oid = bt.base_type
         LEFT JOIN pg_description AS d ON d.objoid = p.oid AND d.classoid = 'pg_proc'::regclass
         WHERE t.oid <> 'trigger'::regtype AND COALESCE(a.callable, true)
           AND has_function_privilege(p.oid, 'execute')
           AND prokind = 'f'
           AND pn.nspname = $1
         ORDER BY p.oid"
    ))
    .bind(schema)
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| Box::new(query_failure(&error, anon)))?;
    let mut routines = Vec::with_capacity(rows.len());
    for (name, description, args, volatility) in rows {
        let params = parse_params(&args).ok_or_else(|| {
            Box::new(pgrst(
                StatusCode::INTERNAL_SERVER_ERROR,
                "PGRST000",
                "Could not decode the function arguments",
                None,
                None,
            ))
        })?;
        routines.push(RoutineDoc {
            name,
            description,
            volatile: volatility == "v",
            params,
        });
    }
    Ok(routines)
}

fn parse_params(text: &str) -> Option<Vec<RoutineParam>> {
    let value: Value = serde_json::from_str(text).ok()?;
    value
        .as_array()?
        .iter()
        .map(|item| {
            let item = item.as_array()?;
            Some(RoutineParam {
                name: item.first()?.as_str()?.to_string(),
                pg_type: item.get(1)?.as_str()?.to_string(),
                required: item.get(2)?.as_bool()?,
                variadic: item.get(3)?.as_bool()?,
            })
        })
        .collect()
}

fn swagger_document(
    schema_comment: Option<&str>,
    tables: &[TableDoc],
    routines: &[RoutineDoc],
) -> Value {
    let (title, description) = match schema_comment {
        Some(comment) => break_on_newline(comment),
        None => (
            "PostgREST API",
            "This is a dynamic API generated by PostgREST",
        ),
    };
    let mut paths = Map::new();
    paths.insert("/".to_string(), root_path());
    let mut definitions = Map::new();
    let mut parameters = global_parameters();
    for table in tables {
        paths.insert(format!("/{}", table.name), table_path(table));
        definitions.insert(table.name.clone(), table_definition(table));
        parameters.insert(format!("body.{}", table.name), body_parameter(&table.name));
        for column in &table.columns {
            parameters.insert(
                format!("rowFilter.{}.{}", table.name, column.name),
                row_filter(column),
            );
        }
    }
    // Overloads share one path; `InsOrdHashMap.fromList` keeps the last,
    // and `decodeFuncs` sorts overloads by ascending parameter count.
    let mut by_name: BTreeMap<&str, &RoutineDoc> = BTreeMap::new();
    for routine in routines {
        let keep = by_name
            .get(routine.name.as_str())
            .is_some_and(|kept| kept.params.len() > routine.params.len());
        if !keep {
            by_name.insert(&routine.name, routine);
        }
    }
    for (name, routine) in by_name {
        paths.insert(format!("/rpc/{name}"), routine_path(routine));
    }
    let mut document = Map::new();
    document.insert("swagger".to_string(), json!("2.0"));
    document.insert(
        "info".to_string(),
        json!({
            "title": title,
            "description": description,
            "version": PRETTY_VERSION,
        }),
    );
    document.insert("host".to_string(), json!("0.0.0.0:3000"));
    document.insert("basePath".to_string(), json!("/"));
    document.insert("schemes".to_string(), json!(["http"]));
    document.insert("consumes".to_string(), mimes());
    document.insert("produces".to_string(), mimes());
    document.insert(
        "externalDocs".to_string(),
        json!({
            "description": "PostgREST Documentation",
            "url": format!("https://postgrest.org/en/{DOCS_VERSION}/references/api.html"),
        }),
    );
    document.insert("paths".to_string(), Value::Object(paths));
    if !definitions.is_empty() {
        document.insert("definitions".to_string(), Value::Object(definitions));
    }
    document.insert("parameters".to_string(), Value::Object(parameters));
    Value::Object(document)
}

fn mimes() -> Value {
    json!([
        "application/json",
        "application/vnd.pgrst.object+json;nulls=stripped",
        "application/vnd.pgrst.object+json",
        "text/csv",
    ])
}

/// `T.breakOn "\n"` followed by dropping the leading newlines of the rest.
/// Without a newline the whole text is the first part and the rest is empty.
fn break_on_newline(text: &str) -> (&str, &str) {
    match text.split_once('\n') {
        Some((first, rest)) => (first, rest.trim_start_matches('\n')),
        None => (text, ""),
    }
}

fn root_path() -> Value {
    json!({
        "get": {
            "tags": ["Introspection"],
            "summary": "OpenAPI description (this document)",
            "produces": ["application/openapi+json", "application/json"],
            "responses": {
                "200": {"description": "OK"}
            }
        }
    })
}

/// Tags, summary and description shared by every operation of a path.
fn base_operation(tag: &str, comment: Option<&str>) -> Map<String, Value> {
    let mut operation = Map::new();
    operation.insert("tags".to_string(), json!([tag]));
    if let Some(comment) = comment {
        let (summary, description) = break_on_newline(comment);
        operation.insert("summary".to_string(), json!(summary));
        if !description.is_empty() {
            operation.insert("description".to_string(), json!(description));
        }
    }
    operation
}

fn table_path(table: &TableDoc) -> Value {
    let operation = base_operation(&table.name, table.description.as_deref());
    let filters: Vec<Value> = table
        .columns
        .iter()
        .map(|column| json!({"$ref": format!("#/parameters/rowFilter.{}.{}", table.name, column.name)}))
        .collect();
    let mut get_params = filters.clone();
    for name in [
        "select",
        "order",
        "range",
        "rangeUnit",
        "offset",
        "limit",
        "preferCount",
    ] {
        get_params.push(json!({"$ref": format!("#/parameters/{name}")}));
    }
    let mut get = operation.clone();
    get.insert("parameters".to_string(), json!(get_params));
    get.insert(
        "responses".to_string(),
        json!({
            "200": {
                "description": "OK",
                "schema": {
                    "type": "array",
                    "items": {"$ref": format!("#/definitions/{}", table.name)}
                }
            },
            "206": {"description": "Partial Content"}
        }),
    );
    let mut path = Map::new();
    path.insert("get".to_string(), Value::Object(get));
    // `makePathItem` adds every write operation when any one is allowed.
    if table.insertable || table.updatable || table.deletable {
        let post_params = vec![
            json!({"$ref": format!("#/parameters/body.{}", table.name)}),
            json!({"$ref": "#/parameters/select"}),
            json!({"$ref": "#/parameters/preferPost"}),
        ];
        let mut post = operation.clone();
        post.insert("parameters".to_string(), json!(post_params));
        post.insert(
            "responses".to_string(),
            json!({"201": {"description": "Created"}}),
        );
        path.insert("post".to_string(), Value::Object(post));

        let mut patch_params = filters.clone();
        patch_params.push(json!({"$ref": format!("#/parameters/body.{}", table.name)}));
        patch_params.push(json!({"$ref": "#/parameters/preferReturn"}));
        let mut patch = operation.clone();
        patch.insert("parameters".to_string(), json!(patch_params));
        patch.insert(
            "responses".to_string(),
            json!({"204": {"description": "No Content"}}),
        );
        path.insert("patch".to_string(), Value::Object(patch));

        let mut delete_params = filters;
        delete_params.push(json!({"$ref": "#/parameters/preferReturn"}));
        let mut delete = operation;
        delete.insert("parameters".to_string(), json!(delete_params));
        delete.insert(
            "responses".to_string(),
            json!({"204": {"description": "No Content"}}),
        );
        path.insert("delete".to_string(), Value::Object(delete));
    }
    Value::Object(path)
}

fn routine_path(routine: &RoutineDoc) -> Value {
    let mut operation = base_operation(
        &format!("(rpc) {}", routine.name),
        routine.description.as_deref(),
    );
    operation.insert(
        "produces".to_string(),
        json!([
            "application/json",
            "application/vnd.pgrst.object+json;nulls=stripped",
            "application/vnd.pgrst.object+json",
        ]),
    );
    operation.insert(
        "responses".to_string(),
        json!({"200": {"description": "OK"}}),
    );
    let mut path = Map::new();
    if !routine.volatile {
        let get_params: Vec<Value> = routine.params.iter().map(routine_get_param).collect();
        let mut get = operation.clone();
        if !get_params.is_empty() {
            get.insert("parameters".to_string(), Value::Array(get_params));
        }
        path.insert("get".to_string(), Value::Object(get));
    }
    let mut post = operation;
    post.insert(
        "parameters".to_string(),
        json!([
            {
                "name": "args",
                "required": true,
                "in": "body",
                "schema": routine_schema(routine),
            },
            {"$ref": "#/parameters/preferParams"},
        ]),
    );
    path.insert("post".to_string(), Value::Object(post));
    Value::Object(path)
}

/// `makeProcGetParam`.
fn routine_get_param(param: &RoutineParam) -> Value {
    let mut value = Map::new();
    value.insert("name".to_string(), json!(param.name));
    value.insert("required".to_string(), json!(param.required));
    value.insert("in".to_string(), json!("query"));
    if param.variadic {
        value.insert(
            "type".to_string(),
            json!(swagger_type(&param.pg_type).unwrap_or("string")),
        );
        let element = type_from_array(&param.pg_type);
        let mut items = Map::new();
        if let Some(kind) = swagger_type(element) {
            items.insert("type".to_string(), json!(kind));
        }
        items.insert("format".to_string(), json!(swagger_format(element)));
        value.insert("items".to_string(), Value::Object(items));
        value.insert("collectionFormat".to_string(), json!("multi"));
    } else {
        value.insert("format".to_string(), json!(swagger_format(&param.pg_type)));
        // Arrays use `{}` in query parameters, and a type is required there.
        let kind = match swagger_type(&param.pg_type) {
            Some("array") | None => "string",
            Some(kind) => kind,
        };
        value.insert("type".to_string(), json!(kind));
    }
    Value::Object(value)
}

/// `makeProcSchema`.
fn routine_schema(routine: &RoutineDoc) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for param in &routine.params {
        let mut property = Map::new();
        insert_type(&mut property, &param.pg_type);
        property.insert("format".to_string(), json!(swagger_format(&param.pg_type)));
        properties.insert(param.name.clone(), Value::Object(property));
        if param.required {
            required.push(Value::String(param.name.clone()));
        }
    }
    let mut schema = Map::new();
    if let Some(description) = &routine.description {
        schema.insert("description".to_string(), json!(description));
    }
    schema.insert("type".to_string(), json!("object"));
    if !properties.is_empty() {
        schema.insert("properties".to_string(), Value::Object(properties));
    }
    if !required.is_empty() {
        schema.insert("required".to_string(), Value::Array(required));
    }
    Value::Object(schema)
}

fn table_definition(table: &TableDoc) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for column in &table.columns {
        properties.insert(column.name.clone(), column_schema(column));
        if !column.nullable {
            required.push(Value::String(column.name.clone()));
        }
    }
    let mut definition = Map::new();
    definition.insert("type".to_string(), json!("object"));
    if !properties.is_empty() {
        definition.insert("properties".to_string(), Value::Object(properties));
    }
    if !required.is_empty() {
        definition.insert("required".to_string(), json!(required));
    }
    if let Some(description) = &table.description {
        definition.insert("description".to_string(), json!(description));
    }
    Value::Object(definition)
}

/// `makeProperty`.
fn column_schema(column: &ColumnDoc) -> Value {
    let mut schema = Map::new();
    if let Some(default) = column
        .default
        .as_deref()
        .and_then(|text| parse_default(&column.pg_type, text))
    {
        schema.insert("default".to_string(), default);
    }
    if let Some(description) = column_description(column) {
        schema.insert("description".to_string(), json!(description));
    }
    if !column.enum_values.is_empty() {
        schema.insert("enum".to_string(), json!(column.enum_values));
    }
    schema.insert("format".to_string(), json!(swagger_format(&column.pg_type)));
    if let Some(length) = column.max_length {
        schema.insert("maxLength".to_string(), json!(length));
    }
    insert_type(&mut schema, &column.pg_type);
    Value::Object(schema)
}

/// Sets `type`, and `items` for an array type (`makePropertyItems`).
fn insert_type(schema: &mut Map<String, Value>, pg_type: &str) {
    let kind = swagger_type(pg_type);
    if let Some(kind) = kind {
        schema.insert("type".to_string(), json!(kind));
    }
    if kind == Some("array") {
        let mut items = Map::new();
        if let Some(element) = swagger_type(type_from_array(pg_type)) {
            items.insert("type".to_string(), json!(element));
        }
        schema.insert("items".to_string(), Value::Object(items));
    }
}

/// `parseDefault`: a string-typed default drops its `::type` cast and the
/// surrounding quotes, then the text is decoded as JSON. A default that is
/// not valid JSON (`nextval(…)`, `'{}'::jsonb`) is left out.
fn parse_default(pg_type: &str, default: &str) -> Option<Value> {
    let text = if swagger_type(pg_type) == Some("string") {
        let bare = default
            .strip_suffix(&format!("::{pg_type}"))
            .map_or(default, |value| value.trim_matches('\''));
        format!("\"{bare}\"")
    } else {
        default.to_string()
    };
    serde_json::from_str(&text).ok()
}

fn column_description(column: &ColumnDoc) -> Option<String> {
    let mut notes = vec!["Note:".to_string()];
    if column.primary_key {
        notes.push("This is a Primary Key.<pk/>".to_string());
    }
    if let Some((table, foreign)) = &column.foreign_key {
        notes.push(format!(
            "This is a Foreign Key to `{table}.{foreign}`.<fk table='{table}' column='{foreign}'/>"
        ));
    }
    if notes.len() == 1 {
        return column.description.clone();
    }
    let prefix = column
        .description
        .as_ref()
        .map_or(String::new(), |text| format!("{text}\n\n"));
    Some(format!("{prefix}{}", notes.join("\n")))
}

/// `toSwaggerType`; `None` for `json` and `jsonb`.
fn swagger_type(pg_type: &str) -> Option<&'static str> {
    match pg_type {
        "character varying" | "character" | "text" => Some("string"),
        "boolean" => Some("boolean"),
        "smallint" | "integer" | "bigint" => Some("integer"),
        "numeric" | "real" | "double precision" => Some("number"),
        "json" | "jsonb" => None,
        _ if pg_type.ends_with("[]") => Some("array"),
        _ => Some("string"),
    }
}

fn type_from_array(pg_type: &str) -> &str {
    pg_type.strip_suffix("[]").unwrap_or(pg_type)
}

fn swagger_format(pg_type: &str) -> String {
    match pg_type {
        "smallint" | "integer" => "int32".to_string(),
        "bigint" => "int64".to_string(),
        _ => pg_type.to_string(),
    }
}

fn global_parameters() -> Map<String, Value> {
    let mut parameters = Map::new();
    parameters.insert(
        "preferParams".to_string(),
        prefer_parameter("Prefer", "Preference", &[]),
    );
    parameters.insert(
        "preferReturn".to_string(),
        prefer_parameter(
            "Prefer",
            "Preference",
            &["return=representation", "return=minimal", "return=none"],
        ),
    );
    parameters.insert(
        "preferCount".to_string(),
        prefer_parameter("Prefer", "Preference", &["count=none"]),
    );
    parameters.insert(
        "preferPost".to_string(),
        prefer_parameter(
            "Prefer",
            "Preference",
            &[
                "return=representation",
                "return=minimal",
                "return=none",
                "resolution=ignore-duplicates",
                "resolution=merge-duplicates",
            ],
        ),
    );
    for (name, description) in [
        ("select", "Filtering Columns"),
        ("on_conflict", "On Conflict"),
        ("order", "Ordering"),
        ("offset", "Limiting and Pagination"),
        ("limit", "Limiting and Pagination"),
    ] {
        parameters.insert(
            name.to_string(),
            query_parameter(name, description, "query", None),
        );
    }
    parameters.insert(
        "range".to_string(),
        query_parameter("Range", "Limiting and Pagination", "header", None),
    );
    parameters.insert(
        "rangeUnit".to_string(),
        query_parameter(
            "Range-Unit",
            "Limiting and Pagination",
            "header",
            Some("items"),
        ),
    );
    parameters
}

fn prefer_parameter(name: &str, description: &str, values: &[&str]) -> Value {
    let mut value = json!({
        "name": name,
        "description": description,
        "required": false,
        "type": "string",
        "in": "header",
    });
    if !values.is_empty() {
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "enum".to_string(),
                Value::Array(values.iter().map(|item| json!(item)).collect()),
            );
        }
    }
    value
}

fn query_parameter(name: &str, description: &str, location: &str, default: Option<&str>) -> Value {
    let mut value = json!({
        "name": name,
        "description": description,
        "required": false,
        "type": "string",
        "in": location,
    });
    if let Some(default) = default {
        if let Some(object) = value.as_object_mut() {
            object.insert("default".to_string(), json!(default));
        }
    }
    value
}

fn body_parameter(table: &str) -> Value {
    json!({
        "name": table,
        "description": table,
        "required": false,
        "in": "body",
        "schema": {"$ref": format!("#/definitions/{table}")},
    })
}

fn row_filter(column: &ColumnDoc) -> Value {
    let mut value = json!({
        "name": column.name,
        "required": false,
        "type": "string",
        "in": "query",
    });
    if let Some(description) = &column.description {
        if let Some(object) = value.as_object_mut() {
            object.insert("description".to_string(), json!(description));
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id_column() -> ColumnDoc {
        ColumnDoc {
            name: "id".to_string(),
            pg_type: "integer".to_string(),
            primary_key: true,
            ..ColumnDoc::default()
        }
    }

    fn todos(insertable: bool, updatable: bool, deletable: bool) -> TableDoc {
        TableDoc {
            name: "todos".to_string(),
            description: None,
            insertable,
            updatable,
            deletable,
            columns: vec![id_column()],
        }
    }

    #[test]
    fn swagger_document_uses_the_pinned_postgrest_version() {
        let mut table = todos(true, true, true);
        table.description = Some("Todos\nA list".to_string());
        let document = swagger_document(None, &[table], &[]);
        assert_eq!(document["swagger"], "2.0");
        assert_eq!(document["info"]["version"], "16.4");
        assert_eq!(document["info"]["title"], "PostgREST API");
        assert_eq!(
            document["info"]["description"],
            "This is a dynamic API generated by PostgREST"
        );
        assert_eq!(document["host"], "0.0.0.0:3000");
        assert_eq!(document["basePath"], "/");
        assert_eq!(
            document["externalDocs"]["url"],
            "https://postgrest.org/en/v16/references/api.html"
        );
        assert_eq!(
            document["paths"]["/todos"]["get"]["responses"]["206"]["description"],
            "Partial Content"
        );
        assert_eq!(document["paths"]["/todos"]["get"]["summary"], "Todos");
        assert_eq!(document["paths"]["/todos"]["get"]["description"], "A list");
        assert_eq!(
            document["definitions"]["todos"]["properties"]["id"]["format"],
            "int32"
        );
        assert_eq!(
            document["definitions"]["todos"]["properties"]["id"]["description"],
            "Note:\nThis is a Primary Key.<pk/>"
        );
        // `openapi-security-active` is off: swagger2 drops the empty values.
        assert!(document.get("securityDefinitions").is_none());
        assert!(document.get("security").is_none());
    }

    #[test]
    fn schema_comment_without_newline_has_an_empty_description() {
        let document = swagger_document(Some("standard public schema"), &[], &[]);
        assert_eq!(document["info"]["title"], "standard public schema");
        assert_eq!(document["info"]["description"], "");
        let document = swagger_document(Some("Title\n\nBody"), &[], &[]);
        assert_eq!(document["info"]["title"], "Title");
        assert_eq!(document["info"]["description"], "Body");
        assert!(document.get("definitions").is_none());
    }

    #[test]
    fn any_write_privilege_adds_every_write_operation() {
        let cases = [
            (false, false, false, false),
            (true, false, false, true),
            (false, true, false, true),
            (false, false, true, true),
        ];
        for (insertable, updatable, deletable, writes) in cases {
            let table = todos(insertable, updatable, deletable);
            let path = swagger_document(None, &[table], &[])["paths"]["/todos"].clone();
            assert!(path.get("get").is_some());
            assert_eq!(path.get("post").is_some(), writes);
            assert_eq!(path.get("patch").is_some(), writes);
            assert_eq!(path.get("delete").is_some(), writes);
        }
    }

    #[test]
    fn defaults_follow_parse_default() {
        assert_eq!(
            parse_default("uuid", "gen_random_uuid()"),
            Some(json!("gen_random_uuid()"))
        );
        assert_eq!(parse_default("text", "'hello'::text"), Some(json!("hello")));
        assert_eq!(
            parse_default("character varying", "'x'::character varying"),
            Some(json!("x"))
        );
        assert_eq!(parse_default("boolean", "false"), Some(json!(false)));
        assert_eq!(parse_default("integer", "0"), Some(json!(0)));
        assert_eq!(parse_default("bigint", "nextval('todos_id_seq')"), None);
        assert_eq!(parse_default("jsonb", "'{}'::jsonb"), None);
    }

    #[test]
    fn column_schema_carries_default_enum_length_and_keys() {
        let column = ColumnDoc {
            name: "note_id".to_string(),
            pg_type: "uuid".to_string(),
            default: Some("gen_random_uuid()".to_string()),
            primary_key: true,
            foreign_key: Some(("notes".to_string(), "id".to_string())),
            description: Some("Parent".to_string()),
            ..ColumnDoc::default()
        };
        assert_eq!(
            column_schema(&column),
            json!({
                "default": "gen_random_uuid()",
                "description": "Parent\n\nNote:\nThis is a Primary Key.<pk/>\nThis is a Foreign Key to `notes.id`.<fk table='notes' column='id'/>",
                "format": "uuid",
                "type": "string",
            })
        );
        let column = ColumnDoc {
            name: "mood".to_string(),
            pg_type: "character varying".to_string(),
            max_length: Some(10),
            enum_values: vec!["happy".to_string(), "sad".to_string()],
            ..ColumnDoc::default()
        };
        assert_eq!(
            column_schema(&column),
            json!({
                "enum": ["happy", "sad"],
                "format": "character varying",
                "maxLength": 10,
                "type": "string",
            })
        );
        let column = ColumnDoc {
            name: "tags".to_string(),
            pg_type: "text[]".to_string(),
            ..ColumnDoc::default()
        };
        assert_eq!(
            column_schema(&column),
            json!({"format": "text[]", "type": "array", "items": {"type": "string"}})
        );
    }

    #[test]
    fn routines_become_rpc_paths() {
        let param = |name: &str, required| RoutineParam {
            name: name.to_string(),
            pg_type: "integer".to_string(),
            required,
            variadic: false,
        };
        let routines = [
            RoutineDoc {
                name: "add_numbers".to_string(),
                description: None,
                volatile: false,
                params: vec![param("a", true), param("b", true)],
            },
            RoutineDoc {
                name: "touch".to_string(),
                description: Some("Touch\nthe row".to_string()),
                volatile: true,
                params: vec![],
            },
        ];
        let document = swagger_document(None, &[], &routines);
        let path = &document["paths"]["/rpc/add_numbers"];
        assert_eq!(path["get"]["tags"], json!(["(rpc) add_numbers"]));
        assert_eq!(
            path["get"]["parameters"][0],
            json!({"name": "a", "required": true, "in": "query", "format": "int32", "type": "integer"})
        );
        assert_eq!(
            path["post"]["parameters"],
            json!([
                {
                    "name": "args",
                    "required": true,
                    "in": "body",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "a": {"format": "int32", "type": "integer"},
                            "b": {"format": "int32", "type": "integer"},
                        },
                        "required": ["a", "b"],
                    },
                },
                {"$ref": "#/parameters/preferParams"},
            ])
        );
        let touch = &document["paths"]["/rpc/touch"];
        assert!(touch.get("get").is_none());
        assert_eq!(touch["post"]["summary"], "Touch");
        assert_eq!(touch["post"]["description"], "the row");
        assert_eq!(
            touch["post"]["parameters"][0]["schema"],
            json!({"description": "Touch\nthe row", "type": "object"})
        );
    }

    #[test]
    fn variadic_get_parameter_repeats() {
        let param = RoutineParam {
            name: "v".to_string(),
            pg_type: "integer[]".to_string(),
            required: true,
            variadic: true,
        };
        assert_eq!(
            routine_get_param(&param),
            json!({
                "name": "v",
                "required": true,
                "in": "query",
                "type": "array",
                "items": {"type": "integer", "format": "int32"},
                "collectionFormat": "multi",
            })
        );
    }

    #[test]
    fn function_arguments_decode_from_json() {
        let params = parse_params(r#"[["", "bytea", true, false]]"#).unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "");
        assert_eq!(params[0].pg_type, "bytea");
        assert!(params[0].required);
        assert!(parse_params("[[1]]").is_none());
    }
}
