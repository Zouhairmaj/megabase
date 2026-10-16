//! `POST /realtime/v1/api/broadcast` and
//! `POST /realtime/v1/api/broadcast/{topic}/events/{event}`.

// Ported from supabase/realtime lib/realtime_web/router.ex,
// lib/realtime_web/controllers/broadcast_controller.ex,
// lib/realtime_web/controllers/broadcast_single_controller.ex,
// lib/realtime_web/controllers/fallback_controller.ex,
// lib/realtime_web/plugs/auth_tenant.ex,
// lib/realtime_web/plugs/validate_broadcast_content_type.ex,
// lib/realtime_web/plugs/parsers/octet_stream.ex,
// lib/realtime/tenants/batch_broadcast.ex and
// lib/realtime/tenants/single_broadcast.ex (Apache-2.0), pin v2.143.3.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    body::{to_bytes, Body, Bytes},
    extract::{rejection::PathRejection, Path, RawQuery, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use megabase_core::MegabaseNotImplemented;
use serde_json::{json, Map, Value};

use crate::token::{authorize_conn, TokenError};
use crate::{RealtimeState, COMPONENT};

pub(crate) const BATCH_UNIT: &str = "realtime:route:POST /realtime/v1/api/broadcast";
pub(crate) const SINGLE_UNIT: &str =
    "realtime:route:POST /realtime/v1/api/broadcast/{topic}/events/{event}";

/// `Plug.Conn.read_body/2` default `:length`, used by every `Plug.Parsers` parser.
const MAX_BODY: usize = 8_000_000;

/// Self-hosted tenant defaults: `Tenant.max_payload_size_in_kb` (3000) and
/// `TENANT_MAX_EVENTS_PER_SECOND` (100, `config/runtime.exs`).
const MAX_PAYLOAD_SIZE_IN_KB: usize = 3000;
const MAX_EVENTS_PER_SECOND: usize = 100;

/// `Tenants.validate_payload_size/2`: `max_payload_size_in_kb * 1000 + 500`.
const MAX_PAYLOAD_SIZE: usize = MAX_PAYLOAD_SIZE_IN_KB * 1000 + 500;

const JSON: &str = "application/json; charset=utf-8";

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn json_response(status: StatusCode, body: &Value) -> Response {
    (status, [(header::CONTENT_TYPE, JSON)], body.to_string()).into_response()
}

/// `FallbackController.call/2` with `{:error, status, message}`.
fn message(status: StatusCode, text: &str) -> Response {
    json_response(status, &json!({ "message": text }))
}

/// `ChangesetView`: `{"errors": …}` with status 422.
fn changeset_errors(errors: Map<String, Value>) -> Response {
    json_response(
        StatusCode::UNPROCESSABLE_ENTITY,
        &json!({ "errors": Value::Object(errors) }),
    )
}

/// The rendering of a `Plug.Parsers` exception through `render_errors`
/// (`ErrorView.template_not_found/2`): the reason phrase, as HTML unless the
/// `Accept` header prefers JSON.
fn parser_error(status: StatusCode, headers: &HeaderMap) -> Response {
    let reason = status.canonical_reason().unwrap_or_default();
    if prefers_json(headers) {
        (
            status,
            [(header::CONTENT_TYPE, JSON)],
            Value::from(reason).to_string(),
        )
            .into_response()
    } else {
        (
            status,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            reason,
        )
            .into_response()
    }
}

/// `Phoenix.Controller.accepts(conn, ["html", "json"])`: the first media
/// range, by descending `q`, that names one of the two formats.
fn prefers_json(headers: &HeaderMap) -> bool {
    let Some(accept) = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let mut ranges: Vec<(f64, String)> = accept
        .split(',')
        .map(|entry| {
            let mut parts = entry.split(';');
            let media = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
            let q = parts
                .filter_map(|p| p.trim().strip_prefix("q="))
                .find_map(|q| q.parse::<f64>().ok())
                .unwrap_or(1.0);
            (q, media)
        })
        .filter(|(q, _)| *q > 0.0)
        .collect();
    ranges.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, media) in ranges {
        match media.as_str() {
            "*/*" | "text/*" | "text/html" => return false,
            "application/*" | "application/json" => return true,
            _ => {}
        }
    }
    false
}

/// `Plug.Conn.Utils.content_type/1`: lowercase `type/subtype`, parameters
/// dropped. `None` when the header does not parse.
fn media_type(headers: &HeaderMap) -> Option<Option<(String, String)>> {
    let Some(value) = headers.get(header::CONTENT_TYPE) else {
        return Some(None);
    };
    let value = value.to_str().ok()?;
    let essence = value.split(';').next().unwrap_or_default().trim();
    let (kind, sub) = essence.split_once('/')?;
    let valid = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$&-^_.+".contains(&b))
    };
    if !valid(kind) || !valid(sub) {
        return None;
    }
    Some(Some((kind.to_ascii_lowercase(), sub.to_ascii_lowercase())))
}

/// What `Plug.Parsers` leaves in `body_params`.
enum BodyParams {
    /// A JSON object, a non-object wrapped as `{"_json": …}`, an
    /// `octet-stream` body under `"_binary"`, or `%{}` for unparsed types.
    Map(Map<String, Value>),
    /// The raw `application/octet-stream` body (`OctetStream` parser).
    Binary(Bytes),
}

/// Why a body did not reach the controller.
enum ParseFailure {
    /// A `Plug.Parsers` exception with this status, see [`parser_error`].
    Status(StatusCode),
    /// What Megabase does not model: urlencoded and multipart bodies, and a
    /// body over [`MAX_BODY`].
    Unmodeled,
}

/// The `Plug.Parsers` step of the endpoint: `:urlencoded`, `:multipart`,
/// `:json` and `OctetStream`, each reading at most [`MAX_BODY`] bytes.
async fn parse_body(headers: &HeaderMap, body: Body) -> Result<BodyParams, ParseFailure> {
    // Above the limit Plug raises `RequestTooLargeError` (413); its rendered
    // reason phrase depends on the Plug status table, which is not vendored.
    let read = |body: Body| async move {
        to_bytes(body, MAX_BODY)
            .await
            .map_err(|_| ParseFailure::Unmodeled)
    };
    match media_type(headers).flatten() {
        Some((kind, sub)) if kind == "application" && (sub == "json" || sub.ends_with("+json")) => {
            let bytes = read(body).await?;
            // `Plug.Parsers.JSON`: an empty body decodes to `%{}`.
            if bytes.is_empty() {
                return Ok(BodyParams::Map(Map::new()));
            }
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(Value::Object(map)) => Ok(BodyParams::Map(map)),
                Ok(other) => Ok(BodyParams::Map(Map::from_iter([(
                    "_json".to_owned(),
                    other,
                )]))),
                Err(_) => Err(ParseFailure::Status(StatusCode::BAD_REQUEST)),
            }
        }
        Some((kind, sub)) if kind == "application" && sub == "octet-stream" => {
            Ok(BodyParams::Binary(read(body).await?))
        }
        Some((kind, sub))
            if (kind == "application" && sub == "x-www-form-urlencoded") || kind == "multipart" =>
        {
            Err(ParseFailure::Unmodeled)
        }
        // `pass: ["*/*"]`: any other body is left unread.
        _ => Ok(BodyParams::Map(Map::new())),
    }
}

/// `Plug.Conn.Query.decode/1` for flat `key=value` pairs. `None` for nested
/// (`a[b]=…`) keys and bad escapes, which Megabase does not model.
fn decode_query(query: Option<&str>) -> Option<Map<String, Value>> {
    let mut params = Map::new();
    for pair in query
        .unwrap_or_default()
        .split('&')
        .filter(|p| !p.is_empty())
    {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = decode_www_form(key)?;
        if key.contains('[') || key.contains(']') {
            return None;
        }
        params.insert(key, Value::String(decode_www_form(value)?));
    }
    Some(params)
}

/// `URI.decode_www_form/1`: `+` is a space, `%XX` an escape.
fn decode_www_form(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Kong's `request-transformer` (`LUA_AUTH_EXPR`) then `AuthTenant.access_token/1`:
/// the `Bearer` token of `Authorization`, else `apikey`. Kong replaces an
/// `Authorization: Bearer sb_…` header with the raw `apikey`, which has no
/// scheme, so `AuthTenant` falls back to `apikey` there as well.
fn access_token(headers: &HeaderMap) -> Option<String> {
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.starts_with("Bearer sb_"))
        .and_then(|value| {
            let (scheme, token) = value.split_once(' ')?;
            scheme.eq_ignore_ascii_case("bearer").then_some(token)
        });
    authorization
        .or_else(|| headers.get("apikey").and_then(|v| v.to_str().ok()))
        .map(str::to_owned)
}

/// `RealtimeWeb.AuthTenant`: 401 `{"message":"Unauthorized"}` unless the token
/// verifies with the tenant secret and carries `role` and `exp`.
fn authenticate(
    state: &RealtimeState,
    headers: &HeaderMap,
    unit: &'static str,
) -> Option<Response> {
    let Some(jwt) = state.jwt.as_ref() else {
        return Some(MegabaseNotImplemented::new(COMPONENT, unit).into_response());
    };
    let Some(token) = access_token(headers) else {
        return Some(message(StatusCode::UNAUTHORIZED, "Unauthorized"));
    };
    match authorize_conn(jwt, &token, unix_now()) {
        Ok(_) => None,
        Err(TokenError::UnsupportedAlgorithm) => {
            Some(MegabaseNotImplemented::new(COMPONENT, unit).into_response())
        }
        Err(_) => Some(message(StatusCode::UNAUTHORIZED, "Unauthorized")),
    }
}

/// `:erlang.external_size/1` of the term `Jason` decodes `value` into, with
/// the leading version byte.
#[cfg(test)]
fn external_size(value: &Value) -> usize {
    1 + term_size(value)
}

fn term_size(value: &Value) -> usize {
    match value {
        // Atoms as `SMALL_ATOM_UTF8_EXT`: `nil`, `true`, `false`.
        Value::Null => 2 + 3,
        Value::Bool(true) => 2 + 4,
        Value::Bool(false) => 2 + 5,
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                integer_size(i128::from(i))
            } else if let Some(u) = n.as_u64() {
                integer_size(i128::from(u))
            } else {
                // `NEW_FLOAT_EXT`.
                9
            }
        }
        // `BINARY_EXT`.
        Value::String(s) => 5 + s.len(),
        // `LIST_EXT` with a `NIL_EXT` tail, or `NIL_EXT` alone.
        Value::Array(items) if items.is_empty() => 1,
        Value::Array(items) => 5 + items.iter().map(term_size).sum::<usize>() + 1,
        // `MAP_EXT`; keys are binaries.
        Value::Object(map) => map_size(map),
    }
}

/// `MAP_EXT` with binary keys, without the version byte.
fn map_size(map: &Map<String, Value>) -> usize {
    5 + map
        .iter()
        .map(|(k, v)| 5 + k.len() + term_size(v))
        .sum::<usize>()
}

fn integer_size(i: i128) -> usize {
    if (0..=255).contains(&i) {
        // `SMALL_INTEGER_EXT`.
        2
    } else if i32::try_from(i).is_ok() {
        // `INTEGER_EXT`.
        5
    } else {
        // `SMALL_BIG_EXT`: tag, length, sign, then the magnitude bytes.
        let magnitude = i.unsigned_abs();
        let bytes = (128 - magnitude.leading_zeros()).div_ceil(8) as usize;
        3 + bytes
    }
}

/// `Ecto.Type.empty_trimmed_string?/1`, the default `empty_values`.
fn is_blank(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        _ => false,
    }
}

fn push_error(errors: &mut Map<String, Value>, field: &str, text: &str) {
    if let Value::Array(list) = errors
        .entry(field.to_owned())
        .or_insert_with(|| Value::Array(Vec::new()))
    {
        list.push(Value::from(text));
    }
}

/// One `BatchBroadcast.Message` after `message_changeset/3`.
struct Message {
    private: bool,
}

/// `BatchBroadcast.message_changeset/3`: `cast` of `id`, `topic`, `payload`,
/// `event` and `private`, `validate_required([:topic, :payload, :event])`
/// and `validate_payload_size/2`.
fn message_changeset(attrs: &Map<String, Value>) -> Result<Message, Map<String, Value>> {
    let mut errors = Map::new();
    // `cast/3`: a blank value is no change; a value of the wrong type is
    // "is invalid", and `validate_required/2` skips that field.
    let string_field = |errors: &mut Map<String, Value>, field: &str| -> bool {
        match attrs.get(field) {
            None => false,
            Some(v) if is_blank(v) => false,
            Some(Value::String(_)) => true,
            Some(_) => {
                push_error(errors, field, "is invalid");
                true
            }
        }
    };
    // `id` is the embedded `:binary_id` primary key: any string casts.
    string_field(&mut errors, "id");
    let topic = string_field(&mut errors, "topic");
    let payload = match attrs.get("payload") {
        None | Some(Value::Null) => false,
        Some(Value::Object(map)) => {
            if 1 + map_size(map) > MAX_PAYLOAD_SIZE {
                push_error(&mut errors, "payload", "Payload size exceeds tenant limit");
            }
            true
        }
        Some(_) => {
            push_error(&mut errors, "payload", "is invalid");
            true
        }
    };
    let event = string_field(&mut errors, "event");
    let private = match attrs.get("private") {
        Some(v) if is_blank(v) => false,
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) if s == "true" || s == "1" => true,
        Some(Value::String(s)) if s == "false" || s == "0" => false,
        Some(_) => {
            push_error(&mut errors, "private", "is invalid");
            false
        }
    };
    for (field, present) in [("topic", topic), ("payload", payload), ("event", event)] {
        if !present {
            push_error(&mut errors, field, "can't be blank");
        }
    }
    if errors.is_empty() {
        Ok(Message { private })
    } else {
        Err(errors)
    }
}

/// Outcome of `BatchBroadcast.changeset/3`.
enum Batch {
    Valid(Vec<Message>),
    Invalid(Map<String, Value>),
    Unmodeled,
}

/// `cast_embed(:messages, required: true, with: &message_changeset/3)`.
fn batch_changeset(params: &Map<String, Value>) -> Batch {
    let mut errors = Map::new();
    let items = match params.get("messages") {
        None => {
            push_error(&mut errors, "messages", "can't be blank");
            return Batch::Invalid(errors);
        }
        Some(Value::Array(items)) => items,
        // Ecto's index-map form, a `nil` relation and keyword-list entries
        // take paths Megabase does not model.
        Some(Value::Null | Value::Object(_)) => return Batch::Unmodeled,
        Some(_) => {
            push_error(&mut errors, "messages", "is invalid");
            return Batch::Invalid(errors);
        }
    };
    if items.is_empty() {
        push_error(&mut errors, "messages", "can't be blank");
        return Batch::Invalid(errors);
    }
    let mut messages = Vec::with_capacity(items.len());
    let mut children = Vec::with_capacity(items.len());
    let mut all_valid = true;
    for item in items {
        match item {
            Value::Object(attrs) => match message_changeset(attrs) {
                Ok(message) => {
                    messages.push(message);
                    children.push(Value::Object(Map::new()));
                }
                Err(child) => {
                    all_valid = false;
                    children.push(Value::Object(child));
                }
            },
            Value::Array(_) => return Batch::Unmodeled,
            _ => {
                push_error(&mut errors, "messages", "is invalid");
                return Batch::Invalid(errors);
            }
        }
    }
    if all_valid {
        Batch::Valid(messages)
    } else {
        errors.insert("messages".to_owned(), Value::Array(children));
        Batch::Invalid(errors)
    }
}

/// `BroadcastController.broadcast/2` behind the `:tenant_api` and
/// `:secure_tenant_api` pipelines.
// megabase:unit realtime:route:POST /realtime/v1/api/broadcast
pub(crate) async fn broadcast(
    State(state): State<Arc<RealtimeState>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if state.jwt.is_none() {
        return MegabaseNotImplemented::new(COMPONENT, BATCH_UNIT).into_response();
    }
    let body = match parse_body(&headers, body).await {
        Ok(body) => body,
        Err(ParseFailure::Status(status)) => return parser_error(status, &headers),
        Err(ParseFailure::Unmodeled) => {
            return MegabaseNotImplemented::new(COMPONENT, BATCH_UNIT).into_response()
        }
    };
    let Some(mut params) = decode_query(query.as_deref()) else {
        return MegabaseNotImplemented::new(COMPONENT, BATCH_UNIT).into_response();
    };
    if let Some(rejection) = authenticate(&state, &headers, BATCH_UNIT) {
        return rejection;
    }
    // Phoenix merges body params over query params; an `octet-stream` body
    // only adds `"_binary"`, which the changeset ignores.
    if let BodyParams::Map(map) = body {
        params.extend(map);
    }
    let messages = match batch_changeset(&params) {
        Batch::Valid(messages) => messages,
        Batch::Invalid(errors) => return changeset_errors(errors),
        Batch::Unmodeled => {
            return MegabaseNotImplemented::new(COMPONENT, BATCH_UNIT).into_response()
        }
    };
    // `check_rate_limit/3` with the rolling rate, which Megabase does not
    // track yet: only the batch-size bound applies.
    if messages.len() > MAX_EVENTS_PER_SECOND {
        return message(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many messages to broadcast, please reduce the batch size",
        );
    }
    // Private messages need the tenant's RLS policies on `realtime.messages`.
    if messages.iter().any(|m| m.private) {
        return MegabaseNotImplemented::new(COMPONENT, BATCH_UNIT).into_response();
    }
    StatusCode::ACCEPTED.into_response()
}

/// `parse_boolean/1`: only `"true"` and `true` are true.
fn parse_boolean(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::Bool(true)))
        || matches!(value, Some(Value::String(s)) if s == "true")
}

/// `BroadcastSingleController.broadcast/2` behind the `:broadcast_single`
/// and `:secure_tenant_api` pipelines.
// megabase:unit realtime:route:POST /realtime/v1/api/broadcast/{topic}/events/{event}
pub(crate) async fn broadcast_single(
    State(state): State<Arc<RealtimeState>>,
    path: Result<Path<(String, String)>, PathRejection>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let not_implemented = || MegabaseNotImplemented::new(COMPONENT, SINGLE_UNIT).into_response();
    if state.jwt.is_none() {
        return not_implemented();
    }
    let Ok(Path((topic, event))) = path else {
        return not_implemented();
    };
    let body = match parse_body(&headers, body).await {
        Ok(body) => body,
        Err(ParseFailure::Status(status)) => return parser_error(status, &headers),
        Err(ParseFailure::Unmodeled) => return not_implemented(),
    };
    let Some(mut params) = decode_query(query.as_deref()) else {
        return not_implemented();
    };
    // `ValidateBroadcastContentType`: no header, `application/json` or
    // `application/octet-stream`.
    match media_type(&headers) {
        Some(None) => {}
        Some(Some((kind, sub)))
            if kind == "application" && (sub == "json" || sub == "octet-stream") => {}
        _ => {
            return json_response(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                &json!({
                    "error": "Unsupported Media Type. Use application/json or application/octet-stream"
                }),
            )
        }
    }
    if let Some(rejection) = authenticate(&state, &headers, SINGLE_UNIT) {
        return rejection;
    }
    // The first controller clause matches `body_params` with a `"_binary"` key.
    let payload = match body {
        // `external_size` of a binary is its length plus 6.
        BodyParams::Binary(bytes) => Payload::Size(bytes.len() + 6),
        BodyParams::Map(map) => {
            let payload = match map.get("_binary") {
                // `validate_payload_present/3`: a `nil` binary is blank.
                Some(Value::Null) => Payload::Blank,
                Some(Value::String(s)) => Payload::Size(s.len() + 6),
                // The binary clause skips the size check for a non-binary.
                Some(_) => Payload::Size(0),
                None => Payload::Size(1 + map_size(&map)),
            };
            params.extend(map);
            payload
        }
    };
    let private = parse_boolean(params.get("private"));
    let persist = parse_boolean(params.get("persist"));

    // `SingleBroadcast.validate_message/7`.
    let mut errors = Map::new();
    if topic.trim().is_empty() {
        push_error(&mut errors, "topic", "can't be blank");
    }
    if event.trim().is_empty() {
        push_error(&mut errors, "event", "can't be blank");
    }
    // `validate_payload_present/3`, then `validate_payload_size/3`.
    match payload {
        Payload::Blank => push_error(&mut errors, "payload", "can't be blank"),
        Payload::Size(size) if size > MAX_PAYLOAD_SIZE => {
            push_error(&mut errors, "payload", "Payload size exceeds tenant limit");
        }
        Payload::Size(_) => {}
    }
    if persist && !private {
        push_error(
            &mut errors,
            "persist",
            "can only be used on private channels",
        );
    }
    if !errors.is_empty() {
        return changeset_errors(errors);
    }
    // Private broadcasts are authorized by the tenant's RLS policies.
    if private {
        return not_implemented();
    }
    StatusCode::ACCEPTED.into_response()
}

/// The payload the single-broadcast controller passes on.
enum Payload {
    /// No payload: a `"_binary"` key set to `null`.
    Blank,
    /// The `external_size` the size check compares, `0` where it is skipped.
    Size(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_size_follows_the_external_term_format() {
        // Version byte, `MAP_EXT` header (5), key `BINARY_EXT` (5 + 1), and a
        // `SMALL_INTEGER_EXT` (2).
        assert_eq!(external_size(&json!({"a": 1})), 14);
        assert_eq!(external_size(&json!({})), 6);
        assert_eq!(external_size(&json!("abc")), 9);
        assert_eq!(external_size(&json!([])), 2);
        // `[1, 2]` is `LIST_EXT` + two small ints + `NIL_EXT`: 11.
        assert_eq!(external_size(&json!([1, 2])), 11);
        assert_eq!(external_size(&json!(1000)), 6);
        assert_eq!(external_size(&json!(1.5)), 10);
        assert_eq!(external_size(&json!(5_000_000_000_i64)), 1 + 3 + 5);
        assert_eq!(external_size(&json!(null)), 6);
    }

    #[test]
    fn query_decoding() {
        let params = decode_query(Some("private=true&a+b=c%20d&x")).unwrap();
        assert_eq!(params["private"], "true");
        assert_eq!(params["a b"], "c d");
        assert_eq!(params["x"], "");
        assert!(decode_query(Some("a[b]=1")).is_none());
        assert!(decode_query(Some("a=%zz")).is_none());
    }

    #[test]
    fn accept_negotiation() {
        let mut headers = HeaderMap::new();
        assert!(!prefers_json(&headers));
        headers.insert(header::ACCEPT, "application/json".parse().unwrap());
        assert!(prefers_json(&headers));
        headers.insert(
            header::ACCEPT,
            "text/html;q=0.5, application/json".parse().unwrap(),
        );
        assert!(prefers_json(&headers));
        headers.insert(header::ACCEPT, "*/*".parse().unwrap());
        assert!(!prefers_json(&headers));
    }

    #[test]
    fn access_token_prefers_bearer_then_apikey() {
        let mut headers = HeaderMap::new();
        headers.insert("apikey", "key".parse().unwrap());
        assert_eq!(access_token(&headers).as_deref(), Some("key"));
        headers.insert(header::AUTHORIZATION, "bearer tok".parse().unwrap());
        assert_eq!(access_token(&headers).as_deref(), Some("tok"));
        headers.insert(header::AUTHORIZATION, "Basic tok".parse().unwrap());
        assert_eq!(access_token(&headers).as_deref(), Some("key"));
        headers.insert(
            header::AUTHORIZATION,
            "Bearer sb_publishable".parse().unwrap(),
        );
        assert_eq!(access_token(&headers).as_deref(), Some("key"));
    }
}
