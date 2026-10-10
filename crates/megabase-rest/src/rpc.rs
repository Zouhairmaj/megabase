// Ported from postgrest src/library/PostgREST/ApiRequest.hs (MIT), pin v16.4,
// postgrest src/library/PostgREST/Plan.hs (MIT), pin v16.4,
// postgrest src/library/PostgREST/Query/QueryBuilder.hs (MIT), pin v16.4,
// postgrest src/library/PostgREST/Query/SqlFragment.hs (MIT), pin v16.4,
// postgrest src/library/PostgREST/ApiRequest/Payload.hs (MIT), pin v16.4,
// postgrest src/library/PostgREST/Response.hs (MIT), pin v16.4,
// and postgrest src/library/PostgREST/Error.hs (MIT), pin v16.4.

//! `GET`, `HEAD`, `OPTIONS`, and `POST` on `/rest/v1/rpc/{function}`.
//!
//! Specified in `specs/rest/rpc.md`. Argument values are bound. The function
//! name and argument names are `quote_ident` after a bound catalog lookup.
//! Cast types pass [`sql_type_name`].

use std::collections::{BTreeMap, BTreeSet};

use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};

use crate::filter::{parse_filter_value, quote_ident, sql_type_name, UnsafeType};
use crate::query::percent_decode;
use crate::read::{
    begin, content_range, database_unavailable, negotiated_schema, not_implemented,
    options_response, pgrst, profile_header, query_failure, reject_accept, reject_prefer, session,
    set_request_context, unimplemented_unit, unsafe_type, Session, JSON_UTF8,
};
use crate::RestState;

const EMPTY_JSON: &str = "Empty or invalid json";
const KEYS_MUST_MATCH: &str = "All object keys must match";

/// Catalog lookup. Names stay in `$1` and `$2`. Domain bases are resolved so a
/// domain over a composite is composite. `bit` and `character` lose their
/// length, matching `funcsSqlQuery`.
const LOOKUP_SQL: &str = "WITH RECURSIVE base_walk AS (
    SELECT oid, oid AS base_type
    FROM pg_type
    WHERE typbasetype = 0
    UNION ALL
    SELECT child.oid, walk.base_type
    FROM pg_type AS child
    JOIN base_walk AS walk ON child.typbasetype = walk.oid
),
expanded AS (
    SELECT
        p.oid,
        p.proname,
        p.provolatile::text AS volatility,
        p.proretset,
        p.pronargs,
        p.pronargdefaults,
        x.name,
        x.type,
        x.mode,
        x.idx
    FROM pg_proc AS p
    JOIN pg_namespace AS pn ON pn.oid = p.pronamespace
    LEFT JOIN LATERAL unnest(
        COALESCE(p.proargnames, ARRAY[]::text[]),
        p.proargtypes::oid[],
        COALESCE(
            p.proargmodes,
            array_fill('i'::\"char\", ARRAY[COALESCE(p.pronargs, 0)::int])
        )
    ) WITH ORDINALITY AS x(name, type, mode, idx) ON true
    WHERE pn.nspname = $1
      AND p.proname = $2
      AND p.prokind = 'f'
),
grouped AS (
    SELECT
        e.oid,
        e.proname,
        e.volatility,
        e.proretset,
        COALESCE(
            json_agg(
                json_build_object(
                    'name', COALESCE(e.name, ''),
                    'type', e.type::regtype::text,
                    'cast', CASE e.type
                        WHEN 'bit'::regtype THEN 'bit varying'
                        WHEN 'bit[]'::regtype THEN 'bit varying[]'
                        WHEN 'character'::regtype THEN 'character varying'
                        WHEN 'character[]'::regtype THEN 'character varying[]'
                        ELSE e.type::regtype::text
                    END,
                    'required', e.idx <= (e.pronargs - e.pronargdefaults),
                    'variadic', COALESCE(e.mode = 'v', false)
                )
                ORDER BY e.idx
            ) FILTER (WHERE e.type IS NOT NULL),
            '[]'::json
        ) AS args,
        CASE
            WHEN COUNT(*) FILTER (WHERE e.type IS NOT NULL) = 0 THEN true
            ELSE CASE (
                COUNT(*) FILTER (WHERE e.type IS NOT NULL)
                - COUNT(e.name) FILTER (WHERE e.type IS NOT NULL)
            )
                WHEN 0 THEN true
                WHEN 1 THEN (
                    array_agg(e.type ORDER BY e.idx) FILTER (WHERE e.type IS NOT NULL)
                )[1] IN (
                    'bytea'::regtype,
                    'json'::regtype,
                    'jsonb'::regtype,
                    'text'::regtype,
                    'xml'::regtype
                )
                ELSE false
            END
        END AS callable
    FROM expanded AS e
    GROUP BY e.oid, e.proname, e.volatility, e.proretset
)
SELECT
    g.proname,
    g.volatility,
    g.proretset,
    (
        t.typtype = 'c'
        OR COALESCE(p.proargmodes::text[] && '{t,b,o}', false)
    ) AS returns_composite,
    tn.nspname AS ret_schema,
    COALESCE(comp.relname, t.typname) AS ret_name,
    g.args
FROM grouped AS g
JOIN pg_proc AS p ON p.oid = g.oid
JOIN base_walk AS bw ON bw.oid = p.prorettype
JOIN pg_type AS t ON t.oid = bw.base_type
JOIN pg_namespace AS tn ON tn.oid = t.typnamespace
LEFT JOIN pg_class AS comp ON comp.oid = t.typrelid
WHERE g.callable
  AND t.oid <> 'trigger'::regtype";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Volatility {
    Volatile,
    Stable,
    Immutable,
}

struct Param {
    name: String,
    /// `regtype` text, used in `PGRST203` and the unnamed-json check.
    pg_type: String,
    /// Type spliced into the cast after the `bit` / `character` rewrite.
    cast_type: String,
    required: bool,
    variadic: bool,
}

struct Proc {
    schema: String,
    name: String,
    volatility: Volatility,
    returns_set: bool,
    returns_composite: bool,
    ret_schema: String,
    ret_name: String,
    params: Vec<Param>,
}

impl Proc {
    fn is_void(&self) -> bool {
        !self.returns_set
            && !self.returns_composite
            && self.ret_schema == "pg_catalog"
            && self.ret_name == "void"
    }

    fn signature(&self) -> String {
        let params = self
            .params
            .iter()
            .map(|param| format!("{} => {}", param.name, param.pg_type))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{}.{}({params})", self.schema, self.name)
    }
}

enum Found<'a> {
    One(&'a Proc),
    None,
    Ambiguous(Vec<&'a Proc>),
}

struct Invocation {
    keys: BTreeSet<String>,
    get_values: BTreeMap<String, Vec<String>>,
    /// Raw JSON object for `POST`. Absent for `GET` and `HEAD`.
    json_body: Option<String>,
}

enum Bind {
    Text(String),
    TextArray(Vec<String>),
}

struct Built {
    sql: String,
    binds: Vec<Bind>,
}

/// Serve one RPC route.
pub(crate) async fn call(
    state: &RestState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    function: &str,
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
    if !is_rpc_method(method) {
        return invalid_method(method);
    }
    if method == Method::OPTIONS {
        // megabase:unit rest:route:OPTIONS /rest/v1/rpc/{function}
        return routine_options(state, method, uri, &session, &schema, function).await;
    }
    if let Some(unit) = reject_prefer(headers.get("prefer")) {
        return unimplemented_unit(method, path, unit);
    }
    if let Some(unit) = reject_accept(headers.get(header::ACCEPT)) {
        return unimplemented_unit(method, path, unit);
    }
    if method == Method::GET && headers.get(header::RANGE).is_some() {
        return not_implemented(method, path);
    }
    let invocation = match parse_invocation(method, path, uri, headers, body) {
        Ok(invocation) => invocation,
        Err(response) => return *response,
    };
    match *method {
        Method::GET => {
            // megabase:unit rest:route:GET /rest/v1/rpc/{function}
            invoke(
                state,
                &session,
                &schema,
                "GET",
                function,
                &invocation,
                false,
            )
            .await
        }
        Method::HEAD => {
            // megabase:unit rest:route:HEAD /rest/v1/rpc/{function}
            invoke(
                state,
                &session,
                &schema,
                "HEAD",
                function,
                &invocation,
                true,
            )
            .await
        }
        Method::POST => {
            // megabase:unit rest:route:POST /rest/v1/rpc/{function}
            invoke(
                state,
                &session,
                &schema,
                "POST",
                function,
                &invocation,
                false,
            )
            .await
        }
        _ => invalid_method(method),
    }
}

fn is_rpc_method(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::POST | Method::OPTIONS
    )
}

fn invalid_method(method: &Method) -> Response {
    pgrst(
        StatusCode::METHOD_NOT_ALLOWED,
        "PGRST101",
        &format!("Cannot use the {method} method on RPC"),
        None,
        None,
    )
}

async fn routine_options(
    state: &RestState,
    method: &Method,
    uri: &Uri,
    session: &Session,
    schema: &str,
    function: &str,
) -> Response {
    let path = uri.path();
    let keys = match get_arguments(uri.query().unwrap_or("")) {
        Ok(values) => values.into_keys().collect(),
        Err(()) => return not_implemented(method, path),
    };
    let Some(pool) = state.pool.as_ref() else {
        return database_unavailable();
    };
    let procs = match load_procs(pool, schema, function, session.anon).await {
        Ok(procs) => procs,
        Err(response) => return *response,
    };
    match find_proc(&procs, &keys, false) {
        Found::One(proc) => options_response(allow_list(proc.volatility)),
        Found::None => no_rpc(schema, function, &keys, false),
        Found::Ambiguous(procs) => ambiguous(&procs),
    }
}

fn allow_list(volatility: Volatility) -> &'static str {
    match volatility {
        Volatility::Volatile => "OPTIONS,POST",
        Volatility::Stable | Volatility::Immutable => "OPTIONS,GET,HEAD,POST",
    }
}

async fn invoke(
    state: &RestState,
    session: &Session,
    schema: &str,
    method: &str,
    function: &str,
    invocation: &Invocation,
    head: bool,
) -> Response {
    let Some(pool) = state.pool.as_ref() else {
        return database_unavailable();
    };
    let procs = match load_procs(pool, schema, function, session.anon).await {
        Ok(procs) => procs,
        Err(response) => return *response,
    };
    let proc = match find_proc(&procs, &invocation.keys, invocation.json_body.is_some()) {
        Found::One(proc) => proc,
        Found::None => {
            return no_rpc(
                schema,
                function,
                &invocation.keys,
                invocation.json_body.is_some(),
            );
        }
        Found::Ambiguous(procs) => return ambiguous(&procs),
    };
    let built = match statement(proc, invocation) {
        Ok(built) => built,
        Err(error) => return unsafe_type(&error),
    };
    match execute(pool, session, schema, method, proc, &built, head).await {
        Ok(response) => response,
        Err(response) => *response,
    }
}

async fn load_procs(
    pool: &sqlx::PgPool,
    schema: &str,
    function: &str,
    anon: bool,
) -> Result<Vec<Proc>, Box<Response>> {
    let rows: Vec<(
        String,
        String,
        bool,
        bool,
        String,
        String,
        serde_json::Value,
    )> = sqlx::query_as(LOOKUP_SQL)
        .bind(schema)
        .bind(function)
        .fetch_all(pool)
        .await
        .map_err(|error| Box::new(query_failure(&error, anon)))?;
    let mut procs = Vec::with_capacity(rows.len());
    for (name, volatility, returns_set, returns_composite, ret_schema, ret_name, args) in rows {
        let params = parse_args(&args).map_err(|()| {
            tracing::error!("rpc catalog arguments were not the expected json array");
            Box::new(pgrst(
                StatusCode::INTERNAL_SERVER_ERROR,
                "PGRST000",
                "Database connection error.",
                Some("routine arguments could not be read"),
                None,
            ))
        })?;
        procs.push(Proc {
            schema: schema.to_string(),
            name,
            volatility: volatility_from(&volatility),
            returns_set,
            returns_composite,
            ret_schema,
            ret_name,
            params,
        });
    }
    Ok(procs)
}

fn volatility_from(text: &str) -> Volatility {
    match text {
        "s" => Volatility::Stable,
        "i" => Volatility::Immutable,
        _ => Volatility::Volatile,
    }
}

fn parse_args(value: &Value) -> Result<Vec<Param>, ()> {
    let items = value.as_array().ok_or(())?;
    let mut params = Vec::with_capacity(items.len());
    for item in items {
        let name = item.get("name").and_then(Value::as_str).ok_or(())?;
        let pg_type = item.get("type").and_then(Value::as_str).ok_or(())?;
        let cast_type = item.get("cast").and_then(Value::as_str).ok_or(())?;
        let required = item.get("required").and_then(Value::as_bool).ok_or(())?;
        let variadic = item.get("variadic").and_then(Value::as_bool).ok_or(())?;
        params.push(Param {
            name: name.to_string(),
            pg_type: pg_type.to_string(),
            cast_type: cast_type.to_string(),
            required,
            variadic,
        });
    }
    Ok(params)
}

fn find_proc<'a>(procs: &'a [Proc], keys: &BTreeSet<String>, post_json: bool) -> Found<'a> {
    let mut named = Vec::new();
    let mut fallback = Vec::new();
    for proc in procs {
        if matches_params(proc, keys) {
            named.push(proc);
        } else if post_json && single_unnamed_json(proc) {
            fallback.push(proc);
        }
    }
    match (named.len(), fallback.len()) {
        (0, 0) => Found::None,
        (0, 1) => Found::One(fallback[0]),
        (0, _) => Found::Ambiguous(fallback),
        (1, _) => Found::One(named[0]),
        _ => Found::Ambiguous(named),
    }
}

fn matches_params(proc: &Proc, keys: &BTreeSet<String>) -> bool {
    if proc.params.is_empty() {
        return keys.is_empty();
    }
    let mut required = BTreeSet::new();
    let mut optional = BTreeSet::new();
    for param in &proc.params {
        if param.required {
            required.insert(param.name.as_str());
        } else {
            optional.insert(param.name.as_str());
        }
    }
    if optional.is_empty() {
        return keys.len() == required.len()
            && keys.iter().all(|key| required.contains(key.as_str()));
    }
    if required.is_empty() {
        return keys.iter().all(|key| optional.contains(key.as_str()));
    }
    required.iter().all(|name| keys.contains(*name))
        && keys
            .iter()
            .all(|key| required.contains(key.as_str()) || optional.contains(key.as_str()))
}

fn single_unnamed_json(proc: &Proc) -> bool {
    match proc.params.as_slice() {
        [param] => param.name.is_empty() && matches!(param.pg_type.as_str(), "json" | "jsonb"),
        _ => false,
    }
}

fn no_rpc(schema: &str, name: &str, keys: &BTreeSet<String>, post_json: bool) -> Response {
    let list = keys
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    let func = format!("{schema}.{name}");
    let message = if keys.is_empty() {
        format!("Could not find the function {func} without parameters in the schema cache")
    } else {
        format!("Could not find the function {func}({list}) in the schema cache")
    };
    let detail_params = if keys.is_empty() {
        " without parameters".to_string()
    } else if keys.len() == 1 {
        format!(" with parameter {list}")
    } else {
        format!(" with parameters {list}")
    };
    let details = if post_json {
        format!(
            "Searched for the function {func}{detail_params} or with a single unnamed json/jsonb parameter, but no matches were found in the schema cache."
        )
    } else {
        format!(
            "Searched for the function {func}{detail_params}, but no matches were found in the schema cache."
        )
    };
    pgrst(
        StatusCode::NOT_FOUND,
        "PGRST202",
        &message,
        Some(&details),
        None,
    )
}

fn ambiguous(procs: &[&Proc]) -> Response {
    let list = procs
        .iter()
        .map(|proc| proc.signature())
        .collect::<Vec<_>>()
        .join(", ");
    pgrst(
        StatusCode::MULTIPLE_CHOICES,
        "PGRST203",
        &format!("Could not choose the best candidate function between: {list}"),
        None,
        Some(
            "Try renaming the parameters or the function itself in the database so function overloading can be resolved",
        ),
    )
}

fn parse_invocation(
    method: &Method,
    path: &str,
    uri: &Uri,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<Invocation, Box<Response>> {
    if method == Method::POST {
        if query_present(uri) {
            return Err(Box::new(not_implemented(method, path)));
        }
        reject_json_type(method, path, headers.get(header::CONTENT_TYPE))?;
        let (raw, keys) = json_object(path, body)?;
        return Ok(Invocation {
            keys,
            get_values: BTreeMap::new(),
            json_body: Some(raw),
        });
    }
    let get_values = get_arguments(uri.query().unwrap_or(""))
        .map_err(|()| Box::new(not_implemented(method, path)))?;
    Ok(Invocation {
        keys: get_values.keys().cloned().collect(),
        get_values,
        json_body: None,
    })
}

fn query_present(uri: &Uri) -> bool {
    uri.query()
        .is_some_and(|raw| raw.split('&').any(|part| !part.is_empty()))
}

fn reject_json_type(
    method: &Method,
    path: &str,
    content_type: Option<&HeaderValue>,
) -> Result<(), Box<Response>> {
    match reject_accept(content_type) {
        None => Ok(()),
        Some("") => {
            let mime = content_type
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .split(';')
                .next()
                .unwrap_or("")
                .trim();
            Err(Box::new(pgrst(
                StatusCode::BAD_REQUEST,
                "PGRST102",
                &format!("Content-Type not acceptable: {mime}"),
                None,
                None,
            )))
        }
        Some(unit) => Err(Box::new(unimplemented_unit(method, path, unit))),
    }
}

fn json_object(path: &str, body: &[u8]) -> Result<(String, BTreeSet<String>), Box<Response>> {
    if body.is_empty() {
        return Ok(("{}".to_string(), BTreeSet::new()));
    }
    let value: Value = serde_json::from_slice(body).map_err(|_| {
        Box::new(pgrst(
            StatusCode::BAD_REQUEST,
            "PGRST102",
            EMPTY_JSON,
            None,
            None,
        ))
    })?;
    match value {
        Value::Object(map) => {
            let raw = String::from_utf8_lossy(body).into_owned();
            let keys = map.keys().cloned().collect();
            Ok((raw, keys))
        }
        Value::Array(items) => match array_keys(&items) {
            ArrayKeys::Differ => Err(Box::new(pgrst(
                StatusCode::BAD_REQUEST,
                "PGRST102",
                KEYS_MUST_MATCH,
                None,
                None,
            ))),
            ArrayKeys::Uniform => Err(Box::new(not_implemented(&Method::POST, path))),
        },
        _ => Err(Box::new(not_implemented(&Method::POST, path))),
    }
}

enum ArrayKeys {
    Differ,
    Uniform,
}

fn array_keys(items: &[Value]) -> ArrayKeys {
    let Some(Value::Object(first)) = items.first() else {
        return if items.is_empty() {
            ArrayKeys::Uniform
        } else {
            ArrayKeys::Differ
        };
    };
    let canonical: BTreeSet<&str> = first.keys().map(String::as_str).collect();
    let uniform = items.iter().all(|item| match item {
        Value::Object(map) => {
            let keys: BTreeSet<&str> = map.keys().map(String::as_str).collect();
            keys == canonical
        }
        _ => false,
    });
    if uniform {
        ArrayKeys::Uniform
    } else {
        ArrayKeys::Differ
    }
}

/// `GET` / `HEAD` / `OPTIONS` arguments. `Err` means a shaping parameter or a
/// filter, which this unit does not apply.
fn get_arguments(raw: &str) -> Result<BTreeMap<String, Vec<String>>, ()> {
    let mut values = BTreeMap::new();
    if raw.is_empty() {
        return Ok(values);
    }
    for part in raw.split('&') {
        if part.is_empty() {
            continue;
        }
        let Some((raw_key, raw_value)) = part.split_once('=') else {
            continue;
        };
        let key = percent_decode(raw_key);
        let value = percent_decode(raw_value);
        if key == "select" && value == "*" {
            continue;
        }
        if shaping_key(&key) || key.is_empty() || key.contains('.') || key.contains("->") {
            return Err(());
        }
        if parse_filter_value(&value).is_ok() {
            return Err(());
        }
        values.entry(key).or_default().push(value);
    }
    Ok(values)
}

fn shaping_key(key: &str) -> bool {
    match key {
        "select" | "columns" | "on_conflict" => true,
        _ => matches!(
            key.rsplit('.').next().unwrap_or(key),
            "order" | "limit" | "offset" | "and" | "or"
        ),
    }
}

fn statement(proc: &Proc, invocation: &Invocation) -> Result<Built, UnsafeType> {
    let positional = matches!(proc.params.as_slice(), [param] if param.name.is_empty());
    let specified: Vec<&Param> = if positional {
        Vec::new()
    } else {
        proc.params
            .iter()
            .filter(|param| invocation.keys.contains(&param.name))
            .collect()
    };
    let (source, binds) = if positional {
        positional_source(proc, invocation)?
    } else if invocation.json_body.is_some() && !specified.is_empty() {
        json_source(proc, invocation, &specified)?
    } else {
        direct_source(proc, invocation, &specified)?
    };
    if proc.is_void() {
        let sql = format!(
            "SELECT \
                nullif(current_setting('response.status', true), '') AS response_status, \
                nullif(current_setting('response.headers', true), '') AS response_headers, \
                1::bigint AS page_total, \
                NULL::text AS body \
             FROM ({source}) AS _call"
        );
        return Ok(Built { sql, binds });
    }
    let agg = aggregate_expr(proc);
    let page = if proc.returns_set {
        "pg_catalog.count(_postgrest_t)::bigint"
    } else {
        "1::bigint"
    };
    let sql = format!(
        "WITH pgrst_source AS ({source}) \
         SELECT \
            nullif(current_setting('response.status', true), '') AS response_status, \
            nullif(current_setting('response.headers', true), '') AS response_headers, \
            {page} AS page_total, \
            {agg} AS body \
         FROM (SELECT pgrst_source.* FROM pgrst_source) _postgrest_t"
    );
    Ok(Built { sql, binds })
}

fn aggregate_expr(proc: &Proc) -> &'static str {
    match (proc.returns_composite, proc.returns_set) {
        (false, false) => {
            "coalesce((pg_catalog.json_agg(_postgrest_t.pgrst_scalar)->0)::text, 'null')"
        }
        (false, true) => "coalesce(pg_catalog.json_agg(_postgrest_t.pgrst_scalar)::text, '[]')",
        (true, false) => "coalesce((pg_catalog.json_agg(_postgrest_t)->0)::text, 'null')",
        (true, true) => "coalesce(pg_catalog.json_agg(_postgrest_t)::text, '[]')",
    }
}

fn qualified(proc: &Proc) -> String {
    format!("{}.{}", quote_ident(&proc.schema), quote_ident(&proc.name))
}

fn positional_source(
    proc: &Proc,
    invocation: &Invocation,
) -> Result<(String, Vec<Bind>), UnsafeType> {
    let cast = sql_type_name(&proc.params[0].cast_type)?;
    let raw = invocation.json_body.clone().unwrap_or_default();
    let call = format!("{}(($1::text)::{cast})", qualified(proc));
    Ok((select_call(proc, &call), vec![Bind::Text(raw)]))
}

fn json_source(
    proc: &Proc,
    invocation: &Invocation,
    specified: &[&Param],
) -> Result<(String, Vec<Bind>), UnsafeType> {
    let typed = record_types(specified)?;
    let cols = specified
        .iter()
        .map(|param| quote_ident(&param.name))
        .collect::<Vec<_>>()
        .join(", ");
    let args = call_assignments(specified);
    let raw = invocation.json_body.clone().unwrap_or_default();
    let call = format!("{}({args})", qualified(proc));
    let tail = format!(
        "(SELECT $1::json AS json_data) pgrst_payload, \
         LATERAL (SELECT {cols} FROM pg_catalog.json_to_record(pgrst_payload.json_data) AS _({typed}) LIMIT 1) pgrst_body"
    );
    let source = if proc.is_void() {
        format!("SELECT {call} FROM {tail}")
    } else if proc.returns_composite {
        format!("SELECT pgrst_call.* FROM {tail}, LATERAL {call} pgrst_call")
    } else {
        format!(
            "SELECT pgrst_call.pgrst_scalar FROM {tail}, LATERAL (SELECT {call} AS pgrst_scalar) pgrst_call"
        )
    };
    Ok((source, vec![Bind::Text(raw)]))
}

fn select_call(proc: &Proc, call: &str) -> String {
    if proc.is_void() {
        format!("SELECT {call}")
    } else if proc.returns_composite {
        format!("SELECT pgrst_call.* FROM {call} pgrst_call")
    } else {
        format!("SELECT pgrst_call.pgrst_scalar FROM (SELECT {call} AS pgrst_scalar) pgrst_call")
    }
}

fn direct_source(
    proc: &Proc,
    invocation: &Invocation,
    specified: &[&Param],
) -> Result<(String, Vec<Bind>), UnsafeType> {
    let mut binds = Vec::new();
    let args = direct_assignments(specified, invocation, &mut binds)?;
    let call = format!("{}({args})", qualified(proc));
    Ok((select_call(proc, &call), binds))
}

fn record_types(params: &[&Param]) -> Result<String, UnsafeType> {
    let mut parts = Vec::with_capacity(params.len());
    for param in params {
        let cast = sql_type_name(&param.cast_type)?;
        parts.push(format!("{} {cast}", quote_ident(&param.name)));
    }
    Ok(parts.join(", "))
}

fn call_assignments(params: &[&Param]) -> String {
    params
        .iter()
        .map(|param| {
            let name = quote_ident(&param.name);
            let value = format!("pgrst_body.{name}");
            if param.variadic {
                format!("VARIADIC {name} := {value}")
            } else {
                format!("{name} := {value}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn direct_assignments(
    params: &[&Param],
    invocation: &Invocation,
    binds: &mut Vec<Bind>,
) -> Result<String, UnsafeType> {
    let mut parts = Vec::with_capacity(params.len());
    for param in params {
        let cast = sql_type_name(&param.cast_type)?;
        let name = quote_ident(&param.name);
        let stored = invocation.get_values.get(&param.name);
        if param.variadic {
            let collected = stored.cloned().unwrap_or_default();
            binds.push(Bind::TextArray(collected));
            let index = binds.len();
            parts.push(format!("VARIADIC {name} := (${index}::text[])::{cast}"));
        } else {
            let value = stored
                .and_then(|values| values.last())
                .map(String::as_str)
                .unwrap_or("")
                .to_string();
            binds.push(Bind::Text(value));
            let index = binds.len();
            parts.push(format!("{name} := (${index}::text)::{cast}"));
        }
    }
    Ok(parts.join(", "))
}

async fn execute(
    pool: &sqlx::PgPool,
    session: &Session,
    schema: &str,
    method: &str,
    proc: &Proc,
    built: &Built,
    head: bool,
) -> Result<Response, Box<Response>> {
    let mut tx = begin(pool).await?;
    if read_only(method, proc) {
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(|error| Box::new(query_failure(&error, session.anon)))?;
    }
    let request_path = format!("/rpc/{}", proc.name);
    set_request_context(&mut tx, session, method, &request_path, schema).await?;
    let mut query = sqlx::query_as::<
        sqlx::Postgres,
        (Option<String>, Option<String>, i64, Option<String>),
    >(&built.sql);
    for bind in &built.binds {
        query = match bind {
            Bind::Text(value) => query.bind(value),
            Bind::TextArray(values) => query.bind(values),
        };
    }
    let (guc_status, guc_headers, page_total, body) = query
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| Box::new(query_failure(&error, session.anon)))?;
    let response = call_response(
        schema,
        head,
        proc,
        page_total,
        body.as_deref().unwrap_or(""),
    );
    let response = apply_guc(guc_status, guc_headers, response)?;
    tx.commit()
        .await
        .map_err(|error| Box::new(query_failure(&error, session.anon)))?;
    Ok(response)
}

fn read_only(method: &str, proc: &Proc) -> bool {
    method != "POST" || proc.volatility != Volatility::Volatile
}

fn call_response(schema: &str, head: bool, proc: &Proc, page_total: i64, body: &str) -> Response {
    if proc.is_void() {
        let range = content_range(0, 1);
        return bare_response(StatusCode::NO_CONTENT, Some(&range), None, None);
    }
    let range = content_range(0, page_total);
    let payload = if head { None } else { Some(body) };
    let mut response = bare_response(StatusCode::OK, Some(&range), Some(JSON_UTF8), payload);
    profile_header(&mut response, schema);
    response
}

fn bare_response(
    status: StatusCode,
    range: Option<&str>,
    content_type: Option<&str>,
    body: Option<&str>,
) -> Response {
    let mut response = if let Some(body) = body {
        (status, body.to_string()).into_response()
    } else {
        Response::builder()
            .status(status)
            .body(axum::body::Body::empty())
            .unwrap_or_else(|_| Response::new(axum::body::Body::empty()))
    };
    if let Some(content_type) = content_type {
        if let Ok(value) = HeaderValue::from_str(content_type) {
            response.headers_mut().insert(header::CONTENT_TYPE, value);
        }
    }
    if let Some(range) = range {
        if let Ok(value) = HeaderValue::from_str(range) {
            response.headers_mut().insert(header::CONTENT_RANGE, value);
        }
    }
    response
}

fn apply_guc(
    status_text: Option<String>,
    headers_text: Option<String>,
    mut response: Response,
) -> Result<Response, Box<Response>> {
    if let Some(text) = status_text {
        let code = parse_status(&text).ok_or_else(|| {
            Box::new(pgrst(
                StatusCode::INTERNAL_SERVER_ERROR,
                "PGRST112",
                "response.status guc must be a valid status code",
                None,
                None,
            ))
        })?;
        *response.status_mut() = code;
    }
    if let Some(text) = headers_text {
        let extras = parse_guc_headers(&text).map_err(|()| {
            Box::new(pgrst(
                StatusCode::INTERNAL_SERVER_ERROR,
                "PGRST111",
                "response.headers guc must be a JSON array composed of objects with a single key and a string value",
                None,
                None,
            ))
        })?;
        for (name, value) in extras {
            if response.headers().get(&name).is_none() {
                response.headers_mut().insert(name, value);
            }
        }
    }
    Ok(response)
}

fn parse_status(text: &str) -> Option<StatusCode> {
    let code: u16 = text.trim().parse().ok()?;
    StatusCode::from_u16(code).ok()
}

fn parse_guc_headers(text: &str) -> Result<Vec<(HeaderName, HeaderValue)>, ()> {
    let value: Value = serde_json::from_str(text).map_err(|_| ())?;
    let items = value.as_array().ok_or(())?;
    let mut headers = Vec::with_capacity(items.len());
    for item in items {
        let Value::Object(map) = item else {
            return Err(());
        };
        let (name, raw) = single_string(map)?;
        let name = HeaderName::try_from(name).map_err(|_| ())?;
        let value = HeaderValue::from_str(raw).map_err(|_| ())?;
        headers.push((name, value));
    }
    Ok(headers)
}

fn single_string(map: &Map<String, Value>) -> Result<(&str, &str), ()> {
    let mut pairs = map.iter();
    let (name, value) = pairs.next().ok_or(())?;
    if pairs.next().is_some() {
        return Err(());
    }
    Ok((name.as_str(), value.as_str().ok_or(())?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use megabase_core::Config;
    use tower::ServiceExt;

    fn param(name: &str, pg_type: &str, required: bool) -> Param {
        Param {
            name: name.to_string(),
            pg_type: pg_type.to_string(),
            cast_type: pg_type.to_string(),
            required,
            variadic: false,
        }
    }

    fn variadic_param(name: &str, pg_type: &str) -> Param {
        Param {
            name: name.to_string(),
            pg_type: pg_type.to_string(),
            cast_type: pg_type.to_string(),
            required: true,
            variadic: true,
        }
    }

    fn scalar(name: &str, params: Vec<Param>) -> Proc {
        Proc {
            schema: "public".to_string(),
            name: name.to_string(),
            volatility: Volatility::Immutable,
            returns_set: false,
            returns_composite: false,
            ret_schema: "pg_catalog".to_string(),
            ret_name: "int4".to_string(),
            params,
        }
    }

    fn keys(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    fn get_invocation(pairs: &[(&str, &str)]) -> Invocation {
        let mut get_values: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (key, value) in pairs {
            get_values
                .entry((*key).to_string())
                .or_default()
                .push((*value).to_string());
        }
        Invocation {
            keys: get_values.keys().cloned().collect(),
            get_values,
            json_body: None,
        }
    }

    #[test]
    fn lookup_binds_schema_and_name() {
        assert!(LOOKUP_SQL.contains("pn.nspname = $1"));
        assert!(LOOKUP_SQL.contains("p.proname = $2"));
        assert!(LOOKUP_SQL.contains("bit varying"));
        assert!(!LOOKUP_SQL.contains("{function}"));
    }

    #[test]
    fn get_call_binds_catalog_order() {
        let proc = scalar(
            "add_numbers",
            vec![param("a", "integer", true), param("b", "integer", true)],
        );
        let built = statement(&proc, &get_invocation(&[("b", "5"), ("a", "4")])).unwrap();
        assert!(built.sql.contains(
            "\"public\".\"add_numbers\"(\"a\" := ($1::text)::integer, \"b\" := ($2::text)::integer)"
        ));
        assert!(!built.sql.contains("SET TRANSACTION"));
        assert!(built.sql.contains("1::bigint AS page_total"));
        assert!(built.sql.contains("json_agg(_postgrest_t.pgrst_scalar)->0"));
        match built.binds.as_slice() {
            [Bind::Text(a), Bind::Text(b)] => {
                assert_eq!(a, "4");
                assert_eq!(b, "5");
            }
            _ => panic!("expected two text binds"),
        }
    }

    #[test]
    fn repeated_scalar_keeps_the_last_value_and_variadic_keeps_all() {
        let proc = scalar("f", vec![param("a", "integer", true)]);
        let invocation = {
            let mut get_values = BTreeMap::new();
            get_values.insert("a".to_string(), vec!["1".to_string(), "2".to_string()]);
            Invocation {
                keys: keys(&["a"]),
                get_values,
                json_body: None,
            }
        };
        let built = statement(&proc, &invocation).unwrap();
        match built.binds.as_slice() {
            [Bind::Text(value)] => assert_eq!(value, "2"),
            _ => panic!("expected one text bind"),
        }

        let proc = scalar("sum_var", vec![variadic_param("xs", "integer[]")]);
        let invocation = {
            let mut get_values = BTreeMap::new();
            get_values.insert(
                "xs".to_string(),
                vec!["1".to_string(), "2".to_string(), "3".to_string()],
            );
            Invocation {
                keys: keys(&["xs"]),
                get_values,
                json_body: None,
            }
        };
        let built = statement(&proc, &invocation).unwrap();
        assert!(built
            .sql
            .contains("VARIADIC \"xs\" := ($1::text[])::integer[]"));
        match built.binds.as_slice() {
            [Bind::TextArray(values)] => {
                assert_eq!(values, &["1".to_string(), "2".to_string(), "3".to_string()])
            }
            _ => panic!("expected a text array"),
        }
    }

    #[test]
    fn post_object_uses_json_to_record_for_present_keys_only() {
        let proc = scalar(
            "add_numbers",
            vec![param("a", "integer", true), param("b", "integer", false)],
        );
        let invocation = Invocation {
            keys: keys(&["a"]),
            get_values: BTreeMap::new(),
            json_body: Some(r#"{"a":2}"#.to_string()),
        };
        let built = statement(&proc, &invocation).unwrap();
        assert!(built
            .sql
            .contains("json_to_record(pgrst_payload.json_data) AS _(\"a\" integer) LIMIT 1"));
        assert!(built.sql.contains("\"a\" := pgrst_body.\"a\""));
        assert!(!built.sql.contains("\"b\""));
        match built.binds.as_slice() {
            [Bind::Text(raw)] => assert_eq!(raw, r#"{"a":2}"#),
            _ => panic!("expected the json body"),
        }
    }

    #[test]
    fn unnamed_json_is_one_positional_cast() {
        let proc = scalar(
            "ingest",
            vec![Param {
                name: String::new(),
                pg_type: "jsonb".to_string(),
                cast_type: "jsonb".to_string(),
                required: true,
                variadic: false,
            }],
        );
        let invocation = Invocation {
            keys: keys(&["a"]),
            get_values: BTreeMap::new(),
            json_body: Some(r#"{"a":1}"#.to_string()),
        };
        let built = statement(&proc, &invocation).unwrap();
        assert!(built
            .sql
            .contains("\"public\".\"ingest\"(($1::text)::jsonb)"));
    }

    #[test]
    fn quoted_name_is_doubled_and_unsafe_cast_is_rejected() {
        let proc = scalar("a\"b", vec![param("a", "integer", true)]);
        let built = statement(&proc, &get_invocation(&[("a", "1")])).unwrap();
        assert!(built.sql.contains("\"public\".\"a\"\"b\""));

        let proc = scalar("f", vec![param("a", "int; drop table t", true)]);
        assert!(statement(&proc, &get_invocation(&[("a", "1")])).is_err());
    }

    #[test]
    fn void_skips_json_agg_and_setof_counts_rows() {
        let mut proc = scalar("gone", Vec::new());
        proc.ret_name = "void".to_string();
        proc.volatility = Volatility::Volatile;
        let built = statement(&proc, &get_invocation(&[])).unwrap();
        assert!(built.sql.contains("NULL::text AS body"));
        assert!(!built.sql.contains("json_agg"));
        assert!(!read_only("POST", &proc));
        assert!(read_only("GET", &proc));

        proc.ret_name = "int4".to_string();
        proc.returns_set = true;
        proc.volatility = Volatility::Stable;
        let built = statement(&proc, &get_invocation(&[])).unwrap();
        assert!(built.sql.contains("pg_catalog.count(_postgrest_t)::bigint"));
        assert!(built.sql.contains("json_agg(_postgrest_t.pgrst_scalar)"));
        assert!(read_only("POST", &proc));
    }

    #[test]
    fn find_proc_matches_required_optional_and_unnamed_json() {
        let add = scalar(
            "add_numbers",
            vec![param("a", "integer", true), param("b", "integer", true)],
        );
        let with_default = scalar(
            "add_numbers",
            vec![param("a", "integer", true), param("b", "integer", false)],
        );
        let unnamed = scalar(
            "add_numbers",
            vec![Param {
                name: String::new(),
                pg_type: "json".to_string(),
                cast_type: "json".to_string(),
                required: true,
                variadic: false,
            }],
        );
        assert!(matches!(
            find_proc(&[add], &keys(&["a", "b"]), false),
            Found::One(_)
        ));
        let add = scalar(
            "add_numbers",
            vec![param("a", "integer", true), param("b", "integer", true)],
        );
        assert!(matches!(
            find_proc(&[add], &keys(&["a"]), false),
            Found::None
        ));
        assert!(matches!(
            find_proc(&[with_default], &keys(&["a"]), true),
            Found::One(_)
        ));
        let with_default = scalar(
            "add_numbers",
            vec![param("a", "integer", true), param("b", "integer", false)],
        );
        assert!(matches!(
            find_proc(&[with_default], &keys(&["a", "c"]), true),
            Found::None
        ));
        let named = scalar("add_numbers", vec![param("a", "integer", true)]);
        assert!(matches!(
            find_proc(&[named, unnamed], &keys(&["a"]), true),
            Found::One(proc) if proc.params[0].name == "a"
        ));
        let unnamed = scalar(
            "add_numbers",
            vec![Param {
                name: String::new(),
                pg_type: "json".to_string(),
                cast_type: "json".to_string(),
                required: true,
                variadic: false,
            }],
        );
        assert!(matches!(
            find_proc(&[unnamed], &keys(&["z"]), true),
            Found::One(_)
        ));
        let unnamed = scalar(
            "add_numbers",
            vec![Param {
                name: String::new(),
                pg_type: "json".to_string(),
                cast_type: "json".to_string(),
                required: true,
                variadic: false,
            }],
        );
        assert!(matches!(
            find_proc(&[unnamed], &keys(&["z"]), false),
            Found::None
        ));
        let left = scalar("f", vec![param("a", "integer", true)]);
        let right = scalar("f", vec![param("a", "text", true)]);
        assert!(matches!(
            find_proc(&[left, right], &keys(&["a"]), false),
            Found::Ambiguous(procs) if procs.len() == 2
        ));
    }

    #[test]
    fn no_rpc_orders_keys_and_post_mentions_unnamed_json() {
        let response = no_rpc("public", "add_numbers", &keys(&["b", "a"]), true);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX);
        let body = futures_executor(body);
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            value["message"],
            "Could not find the function public.add_numbers(a, b) in the schema cache"
        );
        assert_eq!(
            value["details"],
            "Searched for the function public.add_numbers with parameters a, b or with a single unnamed json/jsonb parameter, but no matches were found in the schema cache."
        );
        assert!(value["hint"].is_null());

        let response = no_rpc("public", "ping", &BTreeSet::new(), false);
        let body = futures_executor(axum::body::to_bytes(response.into_body(), usize::MAX));
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            value["message"],
            "Could not find the function public.ping without parameters in the schema cache"
        );
        assert_eq!(
            value["details"],
            "Searched for the function public.ping without parameters, but no matches were found in the schema cache."
        );
    }

    fn futures_executor(
        future: impl std::future::Future<Output = Result<axum::body::Bytes, axum::Error>>,
    ) -> axum::body::Bytes {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(future).unwrap()
    }

    #[test]
    fn get_arguments_ignore_star_and_reject_shaping_and_filters() {
        let values = get_arguments("select=*&a=4&b=5").unwrap();
        assert_eq!(values.get("a").unwrap(), &vec!["4".to_string()]);
        assert!(get_arguments("select=id").is_err());
        assert!(get_arguments("order=id").is_err());
        assert!(get_arguments("a=eq.1").is_err());
        assert!(get_arguments("items.order=id").is_err());
        let decoded = get_arguments("q=a%2Bb+c").unwrap();
        assert_eq!(decoded.get("q").unwrap(), &vec!["a+b c".to_string()]);
    }

    #[test]
    fn allow_follows_volatility() {
        assert_eq!(allow_list(Volatility::Volatile), "OPTIONS,POST");
        assert_eq!(allow_list(Volatility::Immutable), "OPTIONS,GET,HEAD,POST");
        assert_eq!(allow_list(Volatility::Stable), "OPTIONS,GET,HEAD,POST");
    }

    #[test]
    fn guc_status_and_headers_reject_bad_payloads() {
        assert_eq!(parse_status("204"), Some(StatusCode::NO_CONTENT));
        assert_eq!(parse_status("99"), None);
        assert_eq!(parse_status("200 extra"), None);
        let headers = parse_guc_headers(r#"[{"X-Test":"1"}]"#).unwrap();
        assert_eq!(headers.len(), 1);
        assert!(parse_guc_headers(r#"[{"X-Test":1}]"#).is_err());
        assert!(parse_guc_headers(r#"[{"A":"1","B":"2"}]"#).is_err());
        assert!(parse_guc_headers("{}").is_err());
    }

    #[test]
    fn scalar_response_sets_range_and_profile_and_head_omits_the_body() {
        let proc = scalar("add_numbers", Vec::new());
        let response = call_response("public", false, &proc, 1, "7");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            JSON_UTF8
        );
        assert_eq!(
            response.headers().get(header::CONTENT_RANGE).unwrap(),
            "0-0/*"
        );
        assert_eq!(response.headers().get("content-profile").unwrap(), "public");
        let head = call_response("public", true, &proc, 1, "7");
        assert_eq!(head.headers().get(header::CONTENT_TYPE).unwrap(), JSON_UTF8);
        assert!(head.headers().get(header::CONTENT_LENGTH).is_none());

        let mut void_proc = scalar("gone", Vec::new());
        void_proc.ret_name = "void".to_string();
        let response = call_response("public", false, &void_proc, 1, "");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(response.headers().get(header::CONTENT_TYPE).is_none());
        assert!(response.headers().get("content-profile").is_none());
        assert_eq!(
            response.headers().get(header::CONTENT_RANGE).unwrap(),
            "0-0/*"
        );
    }

    async fn send(request: Request<Body>) -> (StatusCode, Value, HeaderMap) {
        let app = crate::read::router(RestState::from_config(&Config::default()));
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, body, headers)
    }

    #[tokio::test]
    async fn rpc_without_a_pool_is_503_and_other_methods_are_405() {
        let (status, body, _) = send(
            Request::builder()
                .uri("/rest/v1/rpc/add_numbers?a=4&b=5")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "PGRST000");
        assert_eq!(body["details"], "DATABASE_URL is unset");

        let (status, _body, _) = send(
            Request::builder()
                .method("HEAD")
                .uri("/rest/v1/rpc/add_numbers")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

        let (status, body, _) = send(
            Request::builder()
                .method("OPTIONS")
                .uri("/rest/v1/rpc/add_numbers")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(body["code"] == "PGRST000" || body.is_null());

        for method in ["PATCH", "PUT", "DELETE"] {
            let (status, body, _) = send(
                Request::builder()
                    .method(method)
                    .uri("/rest/v1/rpc/add_numbers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{method}");
            assert_eq!(body["code"], "PGRST101");
            assert_eq!(
                body["message"],
                format!("Cannot use the {method} method on RPC")
            );
            assert!(body["details"].is_null());
            assert!(body["hint"].is_null());
        }
    }

    #[tokio::test]
    async fn schema_is_checked_before_the_method_and_the_body() {
        let (status, body, _) = send(
            Request::builder()
                .method("PATCH")
                .uri("/rest/v1/rpc/add_numbers")
                .header("content-profile", "secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_ACCEPTABLE);
        assert_eq!(body["code"], "PGRST106");

        let (status, body, _) = send(
            Request::builder()
                .uri("/rest/v1/rpc/add_numbers")
                .header("accept-profile", "graphql_public")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "PGRST000");
    }

    #[tokio::test]
    async fn prefer_shaping_and_post_query_are_501_before_the_pool() {
        let (status, body, _) = send(
            Request::builder()
                .uri("/rest/v1/rpc/add_numbers?a=1")
                .header("prefer", "count=exact")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "rest:prefer:count=exact");

        let (status, body, _) = send(
            Request::builder()
                .uri("/rest/v1/rpc/add_numbers?select=id")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "GET /rest/v1/rpc/add_numbers");

        let (status, body, _) = send(
            Request::builder()
                .method("POST")
                .uri("/rest/v1/rpc/add_numbers?a=1")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"a":1,"b":2}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "POST /rest/v1/rpc/add_numbers");
    }

    #[tokio::test]
    async fn post_json_errors_do_not_need_a_pool() {
        let (status, body, _) = send(
            Request::builder()
                .method("POST")
                .uri("/rest/v1/rpc/add_numbers")
                .header("content-type", "application/json")
                .body(Body::from("not json"))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "PGRST102");
        assert_eq!(body["message"], "Empty or invalid json");

        let (status, body, _) = send(
            Request::builder()
                .method("POST")
                .uri("/rest/v1/rpc/add_numbers")
                .header("content-type", "application/json")
                .body(Body::from(r#"[{"a":1},{"b":2}]"#))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["message"], "All object keys must match");

        let (status, body, _) = send(
            Request::builder()
                .method("POST")
                .uri("/rest/v1/rpc/add_numbers")
                .header("content-type", "application/json")
                .body(Body::from("[1,2]"))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["message"], "All object keys must match");

        let (status, body, _) = send(
            Request::builder()
                .method("POST")
                .uri("/rest/v1/rpc/add_numbers")
                .header("content-type", "text/csv")
                .body(Body::from("a,b"))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "rest:media-type:text/csv");

        let (status, body, _) = send(
            Request::builder()
                .method("POST")
                .uri("/rest/v1/rpc/add_numbers")
                .header("content-type", "application/x-custom")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "PGRST102");
        assert_eq!(
            body["message"],
            "Content-Type not acceptable: application/x-custom"
        );
    }

    #[tokio::test]
    async fn uniform_json_array_and_scalar_json_are_501() {
        let (status, body, _) = send(
            Request::builder()
                .method("POST")
                .uri("/rest/v1/rpc/add_numbers")
                .header("content-type", "application/json")
                .body(Body::from(r#"[{"a":1},{"a":2}]"#))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "POST /rest/v1/rpc/add_numbers");

        let (status, body, _) = send(
            Request::builder()
                .method("POST")
                .uri("/rest/v1/rpc/add_numbers")
                .header("content-type", "application/json")
                .body(Body::from("1"))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "POST /rest/v1/rpc/add_numbers");
    }

    #[test]
    fn ambiguous_hint_names_both_candidates() {
        let left = scalar("f", vec![param("a", "integer", true)]);
        let right = scalar("f", vec![param("b", "text", true)]);
        let response = ambiguous(&[&left, &right]);
        assert_eq!(response.status(), StatusCode::MULTIPLE_CHOICES);
    }
}
