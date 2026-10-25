//! Object info: `HEAD /object/authenticated/:bucket/*` and the
//! `GET /object/info` routes (`specs/storage/object.md`). Both answer from
//! the row's metadata without reading the file.

// Ported from supabase/storage src/http/routes/object/getObjectInfo.ts,
// src/storage/renderer/head.ts, src/storage/renderer/info.ts and
// src/storage/renderer/renderer.ts (Apache-2.0), pin v1.80.2.

use axum::{
    extract::{rejection::PathRejection, OriginalUri, Path, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use megabase_core::JwtClaims;
use serde_json::{json, Map, Value};

use super::{
    anon_claims, backend, bucket_is_public, fastify_json, http_date, iso, not_implemented,
    request_jwt, Actor, Scope, StorageError, StorageState,
};
use crate::limits::is_valid_key;

/// `ROUTE_OPERATIONS` (`src/http/routes/operations.ts`).
const OP_INFO_PUBLIC: &str = "storage.object.info_public";
const OP_GET_AUTH_INFO: &str = "object.get_authenticated_info";
const OP_HEAD_AUTH_INFO: &str = "object.head_authenticated_info";

/// `findObject` with the `object-versioning-core` columns.
const INFO_OBJECT_SQL: &str = "SELECT id::text AS id, name, version, bucket_id, metadata, \
     user_metadata, updated_at, created_at, archived_at, is_delete_marker, is_versioned \
     FROM storage.objects \
     WHERE name COLLATE \"C\" = $1 AND bucket_id = $2 AND archived_at IS NULL LIMIT 1";

/// The renderer the route uses: `head` (headers, no body) or `info`
/// (headers and a JSON body).
#[derive(Clone, Copy, PartialEq)]
enum Render {
    Head,
    Info,
}

#[derive(sqlx::FromRow)]
struct InfoRow {
    id: Option<String>,
    name: Option<String>,
    version: Option<String>,
    bucket_id: Option<String>,
    metadata: Option<Value>,
    user_metadata: Option<Value>,
    updated_at: Option<DateTime<Utc>>,
    created_at: Option<DateTime<Utc>>,
    archived_at: Option<DateTime<Utc>>,
    is_delete_marker: Option<bool>,
    is_versioned: Option<bool>,
}

/// `HEAD /object/authenticated/:bucketName/*`: a valid JWT is required.
// megabase:unit storage:route:HEAD /storage/v1/object/authenticated/{bucketName}/{*}
pub(crate) async fn head_authenticated_object(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let Some(jwt) = request_jwt(&state, &headers) else {
        return not_implemented(&method, &uri);
    };
    let request = InfoRequest {
        method: &method,
        uri: &uri,
        headers: &headers,
        render: Render::Head,
        operation: OP_HEAD_AUTH_INFO,
    };
    object_info(&state, path, &request, Some(jwt), false).await
}

/// `GET /object/info/authenticated/:bucketName/*`: a valid JWT is required.
// megabase:unit storage:route:GET /storage/v1/object/info/authenticated/{bucketName}/{*}
pub(crate) async fn get_object_info_authenticated(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let Some(jwt) = request_jwt(&state, &headers) else {
        return not_implemented(&method, &uri);
    };
    let request = InfoRequest {
        method: &method,
        uri: &uri,
        headers: &headers,
        render: Render::Info,
        operation: OP_GET_AUTH_INFO,
    };
    object_info(&state, path, &request, Some(jwt), false).await
}

/// `GET /object/info/:bucketName/*`: `allowInvalidJwt`, so a missing or
/// invalid JWT reads public buckets only.
// megabase:unit storage:route:GET /storage/v1/object/info/{bucketName}/{*}
pub(crate) async fn get_object_info_any(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let jwt = request_jwt(&state, &headers);
    let request = InfoRequest {
        method: &method,
        uri: &uri,
        headers: &headers,
        render: Render::Info,
        operation: OP_GET_AUTH_INFO,
    };
    object_info(&state, path, &request, jwt, false).await
}

/// `GET /object/info/public/:bucketName/*`: no JWT; the bucket must be
/// public and the row is read as the superuser.
// megabase:unit storage:route:GET /storage/v1/object/info/public/{bucketName}/{*}
pub(crate) async fn get_public_object_info(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let request = InfoRequest {
        method: &method,
        uri: &uri,
        headers: &headers,
        render: Render::Info,
        operation: OP_INFO_PUBLIC,
    };
    object_info(&state, path, &request, None, true).await
}

/// `HEAD /object/public/:bucketName/*`: no JWT plugin, so a bearer token
/// is ignored. The bucket must be public; the row is read as the superuser.
// megabase:unit storage:route:HEAD /storage/v1/object/public/{bucketName}/{*}
pub(crate) async fn head_public_object(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let request = InfoRequest {
        method: &method,
        uri: &uri,
        headers: &headers,
        render: Render::Head,
        operation: OP_INFO_PUBLIC,
    };
    object_info(&state, path, &request, None, true).await
}

/// `HEAD /object/:bucketName/*`: `allowInvalidJwt`, so a missing or invalid
/// JWT reads public buckets only.
// megabase:unit storage:route:HEAD /storage/v1/object/{bucketName}/{*}
pub(crate) async fn head_object(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let jwt = request_jwt(&state, &headers);
    let request = InfoRequest {
        method: &method,
        uri: &uri,
        headers: &headers,
        render: Render::Head,
        operation: OP_HEAD_AUTH_INFO,
    };
    object_info(&state, path, &request, jwt, false).await
}

struct InfoRequest<'a> {
    method: &'a Method,
    uri: &'a Uri,
    headers: &'a HeaderMap,
    render: Render,
    operation: &'static str,
}

/// `getObjectInfo.ts` `requestHandler`: bucket as the superuser
/// (`NoSuchBucket` when missing, or private without a JWT), `InvalidKey`,
/// then the row as the superuser for a public bucket or the public route,
/// under RLS otherwise (`NoSuchKey`).
async fn object_info(
    state: &StorageState,
    path: Result<Path<(String, String)>, PathRejection>,
    request: &InfoRequest<'_>,
    jwt: Option<(JwtClaims, &str)>,
    public_route: bool,
) -> Response {
    let (method, uri) = (request.method, request.uri);
    let Ok(Path((bucket_id, object_name))) = path else {
        return not_implemented(method, uri);
    };
    let Some((pool, _root)) = backend(state) else {
        return not_implemented(method, uri);
    };
    // `versionId` and the transformation options (`X-Transformations`) are
    // not ported, nor `If-None-Match` revalidation directives.
    if uri.query().is_some_and(|query| !query.is_empty())
        || request.headers.contains_key(header::IF_NONE_MATCH)
    {
        return not_implemented(method, uri);
    }
    let anon = anon_claims();
    let authenticated = jwt.is_some();
    let actor = match &jwt {
        _ if public_route => Actor::SuperUser,
        Some((claims, token)) => Actor::Request { claims, jwt: token },
        None => Actor::Request {
            claims: &anon,
            jwt: "",
        },
    };
    let scope = Scope {
        actor,
        method,
        uri,
        headers: request.headers,
        operation: request.operation,
        roles: &state.api_roles,
    };
    if let (false, Some((claims, _))) = (public_route, &jwt) {
        if let Err(response) = super::enforce_request_role(pool, &state.api_roles, claims).await {
            return *response;
        }
    }
    let public = match bucket_is_public(pool, &scope, &bucket_id).await {
        Ok(public) => public,
        Err(error) => return super::apply_response(error),
    };
    let public = match public {
        Some(public) if public || authenticated => public,
        _ => return StorageError::no_such_bucket().into_response(),
    };
    if !is_valid_key(&object_name) {
        return StorageError::invalid_key(&object_name).into_response();
    }
    let lookup = if public || public_route {
        find_info(pool, &scope.as_super_user(), &bucket_id, &object_name).await
    } else {
        find_info(pool, &scope, &bucket_id, &object_name).await
    };
    match lookup {
        Ok(Some(row)) => {
            render(&row, request.render).unwrap_or_else(|| not_implemented(method, uri))
        }
        Ok(None) => StorageError::no_such_key().into_response(),
        Err(error) => super::apply_response(error),
    }
}

async fn find_info(
    pool: &sqlx::PgPool,
    scope: &Scope<'_>,
    bucket_id: &str,
    object_name: &str,
) -> Result<Option<InfoRow>, super::ApplyError> {
    let mut tx = scope.begin(pool).await?;
    let row = sqlx::query_as::<_, InfoRow>(INFO_OBJECT_SQL)
        .bind(object_name)
        .bind(bucket_id)
        .fetch_optional(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(row)
}

/// `createAssetMetadata`: the metadata fields the renderers read, with
/// types checked as upstream does.
#[derive(Debug, Default, PartialEq)]
struct AssetMetadata<'a> {
    cache_control: Option<&'a str>,
    content_encoding: Option<&'a str>,
    content_length: Option<u64>,
    e_tag: Option<&'a str>,
    mimetype: Option<&'a str>,
    size: Option<u64>,
    last_modified: Option<DateTime<Utc>>,
}

/// `parseMetadataNumber`: a safe non-negative integer or a string of
/// digits. Negative integers are `None` here; upstream keeps them, and no
/// upload Megabase serves stores one.
fn metadata_number(value: Option<&Value>) -> Option<u64> {
    const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;
    let n = match value? {
        Value::Number(n) => n.as_u64()?,
        Value::String(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => {
            s.parse().ok()?
        }
        _ => return None,
    };
    (n <= MAX_SAFE_INTEGER).then_some(n)
}

/// The metadata Megabase renders exactly, or `None` (501) for what is not
/// ported: `contentRange`, an `httpStatusCode` other than 200, a missing
/// `eTag` (so also missing metadata), and a `lastModified` that
/// `new Date()` would parse differently.
fn asset_metadata(metadata: Option<&Value>) -> Option<AssetMetadata<'_>> {
    let Some(Value::Object(raw)) = metadata else {
        return None;
    };
    let text = |key: &str| raw.get(key).and_then(Value::as_str);
    if raw.get("contentRange").is_some_and(Value::is_string)
        || metadata_number(raw.get("httpStatusCode")).is_some_and(|code| code != 200)
    {
        return None;
    }
    let last_modified = match raw.get("lastModified") {
        Some(Value::String(at)) => Some(DateTime::parse_from_rfc3339(at).ok()?.with_timezone(&Utc)),
        // `new Date(ms)` applies TimeClip: a finite value within 8.64e15 is
        // truncated toward zero to whole milliseconds.
        Some(Value::Number(ms)) => {
            let ms = ms.as_f64().filter(|ms| ms.abs() <= 8.64e15)?;
            Some(DateTime::from_timestamp_millis(ms.trunc() as i64)?)
        }
        None => None,
        Some(_) => return None,
    };
    Some(AssetMetadata {
        cache_control: text("cacheControl"),
        content_encoding: text("contentEncoding"),
        content_length: metadata_number(raw.get("contentLength")),
        e_tag: Some(text("eTag")?),
        mimetype: text("mimetype"),
        size: metadata_number(raw.get("size")),
        last_modified,
    })
}

/// The renderer's headers and, for `info`, the body. `None` is a 501.
fn render(row: &InfoRow, render: Render) -> Option<Response> {
    let metadata = asset_metadata(row.metadata.as_ref())?;
    let mut response = match render {
        Render::Head => {
            // `HeadRenderer` sets `Content-Type` and `Content-Length` from
            // the metadata; without them the header would be `undefined`.
            let mimetype = metadata.mimetype?;
            let content_type = if mimetype.to_lowercase().contains("text/html") {
                "text/plain"
            } else {
                mimetype
            };
            let mut response = StatusCode::OK.into_response();
            let headers = response.headers_mut();
            headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(content_type).ok()?,
            );
            headers.insert("x-robots-tag", HeaderValue::from_static("none"));
            if let Some(encoding) = metadata.content_encoding {
                headers.insert(
                    header::CONTENT_ENCODING,
                    HeaderValue::from_str(encoding).ok()?,
                );
            }
            headers.insert(header::CONTENT_LENGTH, metadata.content_length?.into());
            response
        }
        Render::Info => fastify_json(StatusCode::OK, &info_body(row, &metadata)),
    };
    let headers = response.headers_mut();
    headers.insert(header::ETAG, HeaderValue::from_str(metadata.e_tag?).ok()?);
    if let Some(at) = metadata.last_modified {
        headers.insert(
            header::LAST_MODIFIED,
            HeaderValue::from_str(&http_date(at)).ok()?,
        );
    }
    if let Some(cache_control) = metadata.cache_control.filter(|value| !value.is_empty()) {
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_str(cache_control).ok()?,
        );
    }
    Some(response)
}

/// The `InfoRenderer` body (`JSON.stringify`, no response schema).
fn info_body(row: &InfoRow, metadata: &AssetMetadata<'_>) -> Value {
    let at = |at: Option<DateTime<Utc>>| at.map_or(Value::Null, |at| json!(iso(Some(at))));
    let mut out = Map::new();
    out.insert("id".into(), json!(row.id));
    out.insert("name".into(), json!(row.name));
    out.insert("version".into(), json!(row.version));
    out.insert("bucket_id".into(), json!(row.bucket_id));
    out.insert("size".into(), json!(metadata.size));
    out.insert("content_type".into(), json!(metadata.mimetype));
    out.insert("cache_control".into(), json!(metadata.cache_control));
    out.insert("content_encoding".into(), json!(metadata.content_encoding));
    out.insert("etag".into(), json!(metadata.e_tag));
    out.insert("metadata".into(), json!(row.user_metadata));
    out.insert("last_modified".into(), at(row.updated_at));
    out.insert("created_at".into(), at(row.created_at));
    out.insert("archived_at".into(), at(row.archived_at));
    out.insert("is_delete_marker".into(), json!(row.is_delete_marker));
    out.insert("is_versioned".into(), json!(row.is_versioned));
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(metadata: Value) -> InfoRow {
        InfoRow {
            id: Some("5d6c1c43-8c2b-4f1a-9e44-0d8f8e0a3c11".into()),
            name: Some("u1/a.png".into()),
            version: Some("v1".into()),
            bucket_id: Some("avatars".into()),
            metadata: Some(metadata),
            user_metadata: None,
            updated_at: DateTime::from_timestamp(1_613_537_012, 770_206_000),
            created_at: DateTime::from_timestamp(1_613_537_012, 0),
            archived_at: None,
            is_delete_marker: Some(false),
            is_versioned: Some(false),
        }
    }

    fn uploaded() -> Value {
        json!({
            "eTag": "\"5d41402abc4b2a76b9719d911017c592\"",
            "size": 5,
            "mimetype": "text/html",
            "cacheControl": "max-age=60",
            "lastModified": "2021-02-17T04:43:32.770Z",
            "contentLength": 5,
            "httpStatusCode": 200,
        })
    }

    #[test]
    fn metadata_numbers_are_safe_integers_or_digit_strings() {
        assert_eq!(metadata_number(Some(&json!(5))), Some(5));
        assert_eq!(metadata_number(Some(&json!("42"))), Some(42));
        assert_eq!(
            metadata_number(Some(&json!(9_007_199_254_740_992_u64))),
            None
        );
        for value in [json!(1.5), json!("4a"), json!(""), json!(null)] {
            assert_eq!(metadata_number(Some(&value)), None, "{value}");
        }
        assert_eq!(metadata_number(None), None);
    }

    #[test]
    fn fractional_last_modified_truncates_like_date() {
        let mut metadata = uploaded();
        metadata["lastModified"] = json!(1500.9);
        let parsed = asset_metadata(Some(&metadata)).unwrap();
        assert_eq!(parsed.last_modified.unwrap().timestamp_millis(), 1500);
    }

    #[test]
    fn unported_metadata_is_501() {
        assert!(asset_metadata(Some(&uploaded())).is_some());
        for patch in [
            json!({ "contentRange": "bytes 0-1/5" }),
            json!({ "httpStatusCode": 206 }),
            json!({ "eTag": null }),
            json!({ "lastModified": "yesterday" }),
            json!({ "lastModified": null }),
            json!({ "lastModified": true }),
            json!({ "lastModified": {} }),
        ] {
            let mut metadata = uploaded();
            for (key, value) in patch.as_object().unwrap() {
                metadata[key] = value.clone();
            }
            assert!(asset_metadata(Some(&metadata)).is_none(), "{patch}");
        }
    }

    #[test]
    fn head_renders_headers_from_the_metadata() {
        let response = render(&row(uploaded()), Render::Head).unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let headers = response.headers();
        assert_eq!(headers[header::CONTENT_TYPE], "text/plain");
        assert_eq!(headers[header::CONTENT_LENGTH], "5");
        assert_eq!(headers[header::ACCEPT_RANGES], "bytes");
        assert_eq!(headers["x-robots-tag"], "none");
        assert_eq!(
            headers[header::ETAG],
            "\"5d41402abc4b2a76b9719d911017c592\""
        );
        assert_eq!(headers[header::CACHE_CONTROL], "max-age=60");
        assert_eq!(
            headers[header::LAST_MODIFIED],
            "Wed, 17 Feb 2021 04:43:32 GMT"
        );

        let mut no_length = uploaded();
        no_length.as_object_mut().unwrap().remove("contentLength");
        assert!(render(&row(no_length), Render::Head).is_none());
    }

    #[test]
    fn info_renders_the_object_as_json() {
        let response = render(&row(uploaded()), Render::Info).unwrap();
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/json; charset=utf-8"
        );
        assert_eq!(response.headers()[header::CACHE_CONTROL], "max-age=60");
        assert!(response.headers().get(header::ACCEPT_RANGES).is_none());
        let stored = uploaded();
        let metadata = asset_metadata(Some(&stored)).unwrap();
        assert_eq!(
            info_body(&row(uploaded()), &metadata),
            json!({
                "id": "5d6c1c43-8c2b-4f1a-9e44-0d8f8e0a3c11",
                "name": "u1/a.png",
                "version": "v1",
                "bucket_id": "avatars",
                "size": 5,
                "content_type": "text/html",
                "cache_control": "max-age=60",
                "content_encoding": null,
                "etag": "\"5d41402abc4b2a76b9719d911017c592\"",
                "metadata": null,
                "last_modified": "2021-02-17T04:43:32.770Z",
                "created_at": "2021-02-17T04:43:32.000Z",
                "archived_at": null,
                "is_delete_marker": false,
                "is_versioned": false,
            })
        );
    }
}
