// Ported from postgrest src/library/PostgREST/Response/OpenAPI.hs (MIT), pin v16.4,
// and postgrest src/library/PostgREST/Response.hs (MIT), pin v16.4.

//! `GET` and `HEAD /rest/v1/`: the OpenAPI 2.0 document for one schema.
//!
//! `openapi-mode` is `follow-privileges`, so the table list runs after
//! `SET ROLE`. `server-host` `!4` and port `3000` match the PostgREST
//! defaults shipped with the pinned Supabase stack.

use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Map, Value};

use crate::read::{
    self, begin, pgrst, query_failure, set_request_context, unimplemented_unit, Session, JSON_UTF8,
};

const OPENAPI_UTF8: &str = "application/openapi+json; charset=utf-8";
const PRETTY_VERSION: &str = "16.4";
const DOCS_VERSION: &str = "v16";

struct TableDoc {
    name: String,
    description: Option<String>,
    insertable: bool,
    updatable: bool,
    deletable: bool,
    columns: Vec<ColumnDoc>,
}

struct ColumnDoc {
    name: String,
    pg_type: String,
    nullable: bool,
    primary_key: bool,
    description: Option<String>,
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
    if let Err(error) = tx.commit().await {
        return query_failure(&error, session.anon);
    }
    let body = swagger_document(comment.as_deref(), &tables);
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
    let first = text.split(',').next().unwrap_or("").trim();
    if first.is_empty() || first == "*/*" {
        return Ok(OPENAPI_UTF8);
    }
    let essence = first
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if essence == "application/openapi+json" {
        return Ok(OPENAPI_UTF8);
    }
    if let Some(unit) = read::reject_accept(Some(value)) {
        if unit.is_empty() {
            return Err(Box::new(pgrst(
                StatusCode::NOT_ACCEPTABLE,
                "PGRST107",
                &format!("None of these media types are available: {essence}"),
                None,
                None,
            )));
        }
        return Err(Box::new(unimplemented_unit(
            &axum::http::Method::GET,
            "/rest/v1/",
            unit,
        )));
    }
    if essence == "application/json" || essence == "application/*" {
        return Ok(JSON_UTF8);
    }
    Ok(OPENAPI_UTF8)
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

async fn load_tables(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    schema: &str,
    anon: bool,
) -> Result<Vec<TableDoc>, Box<Response>> {
    let rows: Vec<(String, Option<String>, bool, bool, bool)> = sqlx::query_as(
        "SELECT c.relname::text,
                pg_catalog.obj_description(c.oid, 'pg_class'),
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

    let columns: Vec<(String, String, String, bool, bool, Option<String>)> = sqlx::query_as(
        "SELECT c.relname::text,
                a.attname::text,
                format_type(a.atttypid, a.atttypmod),
                NOT a.attnotnull,
                EXISTS (
                  SELECT 1
                  FROM pg_constraint AS k
                  WHERE k.conrelid = c.oid
                    AND k.contype = 'p'
                    AND a.attnum = ANY (k.conkey)
                ),
                pg_catalog.col_description(a.attrelid, a.attnum)
         FROM pg_attribute AS a
         JOIN pg_class AS c ON c.oid = a.attrelid
         JOIN pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname = $1
           AND a.attnum > 0
           AND NOT a.attisdropped
           AND c.relkind IN ('v', 'r', 'm', 'f', 'p')
           AND NOT c.relispartition
         ORDER BY c.relname, a.attnum",
    )
    .bind(schema)
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| Box::new(query_failure(&error, anon)))?;

    let mut tables = Vec::new();
    for (name, description, insertable, updatable, deletable) in rows {
        let cols = columns
            .iter()
            .filter(|(table, ..)| table == &name)
            .map(
                |(_, column, pg_type, nullable, primary_key, description)| ColumnDoc {
                    name: column.clone(),
                    pg_type: pg_type.clone(),
                    nullable: *nullable,
                    primary_key: *primary_key,
                    description: description.clone(),
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

fn swagger_document(schema_comment: Option<&str>, tables: &[TableDoc]) -> Value {
    let (title, description) = split_comment(
        schema_comment,
        "PostgREST API",
        "This is a dynamic API generated by PostgREST",
    );
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
    json!({
        "swagger": "2.0",
        "info": {
            "title": title,
            "description": description,
            "version": PRETTY_VERSION,
        },
        "host": "!4:3000",
        "basePath": "/",
        "schemes": ["http"],
        "consumes": mimes(),
        "produces": mimes(),
        "externalDocs": {
            "description": "PostgREST Documentation",
            "url": format!("https://postgrest.org/en/{DOCS_VERSION}/references/api.html"),
        },
        "paths": paths,
        "definitions": definitions,
        "parameters": parameters,
        "securityDefinitions": {},
        "security": [],
    })
}

fn mimes() -> Value {
    json!([
        "application/json",
        "application/vnd.pgrst.object+json;nulls=stripped",
        "application/vnd.pgrst.object+json",
        "text/csv",
    ])
}

fn split_comment<'a>(
    comment: Option<&'a str>,
    default_title: &'a str,
    default_description: &'a str,
) -> (&'a str, &'a str) {
    let Some(comment) = comment else {
        return (default_title, default_description);
    };
    match comment.split_once('\n') {
        Some((title, rest)) => (title, rest.trim_start_matches('\n')),
        None => (comment, default_description),
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

fn table_path(table: &TableDoc) -> Value {
    let (summary, description) = split_comment(table.description.as_deref(), "", "");
    let mut operation = Map::new();
    operation.insert("tags".to_string(), json!([table.name]));
    if !summary.is_empty() {
        operation.insert("summary".to_string(), json!(summary));
    }
    if !description.is_empty() {
        operation.insert("description".to_string(), json!(description));
    }
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
    if table.insertable {
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
    }
    if table.updatable {
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
    }
    if table.deletable {
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
    definition.insert("properties".to_string(), Value::Object(properties));
    if !required.is_empty() {
        definition.insert("required".to_string(), json!(required));
    }
    if let Some(description) = &table.description {
        definition.insert("description".to_string(), json!(description));
    }
    Value::Object(definition)
}

fn column_schema(column: &ColumnDoc) -> Value {
    let mut schema = Map::new();
    if let Some(kind) = swagger_type(&column.pg_type) {
        schema.insert("type".to_string(), json!(kind));
    }
    schema.insert("format".to_string(), json!(swagger_format(&column.pg_type)));
    if column.pg_type.ends_with("[]") {
        let item = column.pg_type.trim_end_matches("[]");
        let mut items = Map::new();
        if let Some(kind) = swagger_type(item) {
            items.insert("type".to_string(), json!(kind));
        }
        schema.insert("items".to_string(), Value::Object(items));
    }
    if let Some(description) = column_description(column) {
        schema.insert("description".to_string(), json!(description));
    }
    Value::Object(schema)
}

fn column_description(column: &ColumnDoc) -> Option<String> {
    if !column.primary_key {
        return column.description.clone();
    }
    let note = "Note:\nThis is a Primary Key.<pk/>";
    Some(match &column.description {
        Some(text) if !text.is_empty() => format!("{text}\n\n{note}"),
        _ => note.to_string(),
    })
}

fn swagger_type(pg_type: &str) -> Option<&'static str> {
    let base = pg_type.split('(').next().unwrap_or(pg_type).trim();
    if base.ends_with("[]") {
        return Some("array");
    }
    match base {
        "boolean" => Some("boolean"),
        "smallint" | "integer" | "bigint" => Some("integer"),
        "numeric" | "real" | "double precision" => Some("number"),
        "json" | "jsonb" => None,
        _ => Some("string"),
    }
}

fn swagger_format(pg_type: &str) -> String {
    let base = pg_type.split('(').next().unwrap_or(pg_type).trim();
    match base {
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

    #[test]
    fn swagger_document_uses_the_pinned_postgrest_version() {
        let table = TableDoc {
            name: "todos".to_string(),
            description: Some("Todos\nA list".to_string()),
            insertable: true,
            updatable: true,
            deletable: true,
            columns: vec![ColumnDoc {
                name: "id".to_string(),
                pg_type: "integer".to_string(),
                nullable: false,
                primary_key: true,
                description: None,
            }],
        };
        let document = swagger_document(None, &[table]);
        assert_eq!(document["swagger"], "2.0");
        assert_eq!(document["info"]["version"], "16.4");
        assert_eq!(document["info"]["title"], "PostgREST API");
        assert_eq!(document["host"], "!4:3000");
        assert_eq!(document["basePath"], "/");
        assert_eq!(
            document["externalDocs"]["url"],
            "https://postgrest.org/en/v16/references/api.html"
        );
        assert_eq!(
            document["paths"]["/todos"]["get"]["responses"]["206"]["description"],
            "Partial Content"
        );
        assert_eq!(
            document["definitions"]["todos"]["properties"]["id"]["format"],
            "int32"
        );
        assert_eq!(
            document["definitions"]["todos"]["properties"]["id"]["description"],
            "Note:\nThis is a Primary Key.<pk/>"
        );
        assert!(document["securityDefinitions"]
            .as_object()
            .unwrap()
            .is_empty());
        assert!(document["paths"]["/todos"].get("post").is_some());
        assert!(document["paths"]["/todos"].get("patch").is_some());
        assert!(document["paths"]["/todos"].get("delete").is_some());
    }

    #[test]
    fn mutation_methods_follow_each_privilege() {
        let column = ColumnDoc {
            name: "id".to_string(),
            pg_type: "integer".to_string(),
            nullable: false,
            primary_key: true,
            description: None,
        };
        let cases = [
            (true, false, false, true, false, false),
            (false, true, false, false, true, false),
            (false, false, true, false, false, true),
        ];
        for (insertable, updatable, deletable, post, patch, delete) in cases {
            let table = TableDoc {
                name: "todos".to_string(),
                description: None,
                insertable,
                updatable,
                deletable,
                columns: vec![ColumnDoc {
                    name: column.name.clone(),
                    pg_type: column.pg_type.clone(),
                    nullable: column.nullable,
                    primary_key: column.primary_key,
                    description: None,
                }],
            };
            let path = swagger_document(None, &[table])["paths"]["/todos"].clone();
            assert_eq!(path.get("post").is_some(), post);
            assert_eq!(path.get("patch").is_some(), patch);
            assert_eq!(path.get("delete").is_some(), delete);
        }
    }
}
