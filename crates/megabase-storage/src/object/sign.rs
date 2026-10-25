//! `POST /object/sign/:bucketName` and `POST /object/upload/sign/:bucketName/*`:
//! batch download URLs and upload URLs (`specs/storage/object.md`).

// Ported from supabase/storage src/http/routes/object/getSignedURLs.ts,
// getSignedUploadURL.ts, src/storage/object.ts (`signObjectUrls`,
// `signUploadObjectUrl`), src/storage/uploader.ts (`canUpload`),
// src/storage/database/pg.ts (`findObjects`) and src/internal/auth/jwt.ts
// (Apache-2.0), pin v1.80.2.

use std::{borrow::Cow, collections::HashSet};

use axum::{
    body::Bytes,
    extract::{rejection::PathRejection, OriginalUri, Path, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use serde_json::{json, Value};

use super::{
    can_upload, complete_upload, etag, fetch_upload_bucket, file_upload, header_str, invalid_jwt,
    max_file_size, object_file, remove_file, request_jwt, write_file, Actor, Scope,
    MAX_SIGNED_URL_EXPIRES_IN, SCOPE_DOWNLOAD,
};
use crate::{
    database_error, fastify_json,
    limits::{is_uuid, is_valid_bucket_name, is_valid_key},
    mime::is_allowed_mime_type,
    not_implemented,
    render::{query_pairs, single, verified_signed_payload},
    StorageError, StorageState,
};

/// `ROUTE_OPERATIONS.SIGN_OBJECT_URLS`.
const OP_SIGN_MANY: &str = "storage.object.sign_many";
/// `ROUTE_OPERATIONS.SIGN_UPLOAD_URL`.
const OP_SIGN_UPLOAD_URL: &str = "storage.object.sign_upload_url";

/// `SIGNED_URL_SCOPE_UPLOAD`.
const SCOPE_UPLOAD: &str = "upload";

/// `ROUTE_OPERATIONS.UPLOAD_SIGN_OBJECT`.
const OP_UPLOAD_SIGNED: &str = "storage.object.upload_signed";

/// `MAX_OBJECTS_PER_REQUEST`, the schema's `maxItems` for `paths`.
/// It is also `MAX_OBJECTS_PER_LOOKUP_BATCH`, so one lookup covers a body.
const MAX_OBJECTS_PER_REQUEST: usize = 1000;

/// `uploadSignedUrlExpirationTime`: `UPLOAD_SIGNED_URL_EXPIRATION_TIME` is
/// unset in the self-hosted compose file, so the default 60 seconds.
const UPLOAD_SIGNED_URL_EXPIRES_IN: i64 = 60;

/// `findObjects(bucket, paths, 'name')` after `object-versioning-core`.
const FIND_OBJECTS_SQL: &str = "SELECT name FROM storage.objects \
     WHERE bucket_id = $1 AND name COLLATE \"C\" = ANY($2::text[]) AND archived_at IS NULL";

/// `signObjectUrls`' answer for a path the request role cannot read.
const NOT_FOUND_OR_DENIED: &str =
    "Either the object does not exist or you do not have access to it";

/// A `getSignedURLsBodySchema` body Megabase serves: `expiresIn` an integer
/// in `1..=10 years` and `paths` 1 to 1000 strings. Other properties,
/// Fastify coercion and validation messages are not ported: `None`.
fn sign_many_body(body: &[u8]) -> Option<(i64, Vec<String>)> {
    let body: Value = serde_json::from_slice(body).ok()?;
    let body = body.as_object()?;
    if body.keys().any(|key| key != "expiresIn" && key != "paths") {
        return None;
    }
    let expires_in = body
        .get("expiresIn")?
        .as_i64()
        .filter(|n| (1..=MAX_SIGNED_URL_EXPIRES_IN).contains(n))?;
    let paths = body.get("paths")?.as_array()?;
    if paths.is_empty() || paths.len() > MAX_OBJECTS_PER_REQUEST {
        return None;
    }
    let paths = paths
        .iter()
        .map(|path| path.as_str().map(str::to_string))
        .collect::<Option<Vec<_>>>()?;
    Some((expires_in, paths))
}

/// `POST /object/sign/:bucketName` with `{"expiresIn": n, "paths": [...]}`:
/// one entry per path, in request order, with a download URL for each path
/// the request role can read and an error for the rest.
// megabase:unit storage:route:POST /storage/v1/object/sign/{bucketName}
pub(crate) async fn sign_objects(
    State(state): State<StorageState>,
    Path(bucket_id): Path<String>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some((expires_in, paths)) = sign_many_body(&body) else {
        return not_implemented(&method, &uri);
    };
    let (Some(pool), Some(signer)) = (state.pool.as_ref(), state.jwt.as_deref()) else {
        return not_implemented(&method, &uri);
    };
    let Some((claims, jwt)) = request_jwt(&state, &headers) else {
        return not_implemented(&method, &uri);
    };
    if !is_valid_bucket_name(&bucket_id) {
        return StorageError::invalid_bucket_name().into_response();
    }
    let scope = Scope {
        actor: Actor::Request {
            claims: &claims,
            jwt,
        },
        method: &method,
        uri: &uri,
        headers: &headers,
        operation: OP_SIGN_MANY,
        roles: &state.api_roles,
    };
    let attempt = async {
        let mut tx = scope.begin(pool).await?;
        let names: Vec<String> = sqlx::query_scalar(FIND_OBJECTS_SQL)
            .bind(&bucket_id)
            .bind(&paths)
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok::<_, super::ApplyError>(names)
    };
    let found: HashSet<String> = match attempt.await {
        Ok(names) => names.into_iter().collect(),
        Err(error) => return super::apply_response(error),
    };
    let now = Utc::now().timestamp();
    let mut entries = Vec::with_capacity(paths.len());
    for path in &paths {
        let entry = if found.contains(path) {
            let url = format!("{bucket_id}/{path}");
            let payload = json!({
                "url": url,
                "scope": SCOPE_DOWNLOAD,
                "iat": now,
                "exp": now + expires_in,
            });
            let Ok(token) = signer.sign(&payload) else {
                return not_implemented(&method, &uri);
            };
            json!({
                "error": null,
                "path": path,
                "signedURL": format!("/object/sign/{url}?token={}", token.as_str()),
            })
        } else {
            json!({ "error": NOT_FOUND_OR_DENIED, "path": path, "signedURL": null })
        };
        entries.push(entry);
    }
    fastify_json(StatusCode::OK, &Value::Array(entries))
}

/// The `canUpload` metadata `getSignedUploadURL` passes: `mimetype` from
/// `Content-Type` and `contentLength` from `Content-Length`, each omitted
/// when absent. `None` for a `Content-Length` that `Number()` would not
/// read as a plain decimal (not ported).
fn upload_probe(headers: &HeaderMap) -> Option<Value> {
    let mut probe = serde_json::Map::new();
    if let Some(mime_type) = header_str(headers, header::CONTENT_TYPE.as_str()) {
        probe.insert("mimetype".into(), json!(mime_type));
    }
    if let Some(length) = header_str(headers, header::CONTENT_LENGTH.as_str()) {
        let length = length.parse::<u64>().ok()?;
        probe.insert("contentLength".into(), json!(length));
    }
    Some(Value::Object(probe))
}

/// Whether Fastify accepts `body` on a route that ignores it, in the cases
/// Megabase serves: no body and no `Content-Type`, or an `application/json`
/// object.
fn ignored_body(headers: &HeaderMap, body: &[u8]) -> bool {
    match header_str(headers, header::CONTENT_TYPE.as_str()) {
        None => body.is_empty(),
        Some(content_type) => {
            content_type
                .split(';')
                .next()
                .is_some_and(|essence| essence.trim() == "application/json")
                && serde_json::from_slice::<Value>(body).is_ok_and(|value| value.is_object())
        }
    }
}

/// `POST /object/upload/sign/:bucketName/*`: check the insert (or upsert
/// with `x-upsert: true`) under the request's RLS, then sign an upload
/// token for the path that expires in 60 seconds.
// megabase:unit storage:route:POST /storage/v1/object/upload/sign/{bucketName}/{*}
pub(crate) async fn sign_upload_object(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Ok(Path((bucket_id, object_name))) = path else {
        return not_implemented(&method, &uri);
    };
    // The route reads no body, but Fastify still parses one: Megabase serves
    // none at all, or a JSON object (supabase-js sends `{}`). User metadata
    // (`x-metadata`) is not ported.
    if !ignored_body(&headers, &body) || headers.contains_key("x-metadata") {
        return not_implemented(&method, &uri);
    }
    let Some(probe) = upload_probe(&headers) else {
        return not_implemented(&method, &uri);
    };
    let (Some(pool), Some(signer)) = (state.pool.as_ref(), state.jwt.as_deref()) else {
        return not_implemented(&method, &uri);
    };
    let Some((claims, jwt)) = request_jwt(&state, &headers) else {
        return not_implemented(&method, &uri);
    };
    if !is_valid_bucket_name(&bucket_id) {
        return StorageError::invalid_bucket_name().into_response();
    }
    // `canUpload` does not check the key; what an invalid key does to the
    // insert is not verified.
    // `urlPath` is the decoded path; Megabase serves paths without percent
    // escapes, where it is the raw path itself.
    let raw = uri
        .path()
        .strip_prefix("/storage/v1/object/upload/sign/")
        .unwrap_or_default();
    let url = format!("{bucket_id}/{object_name}");
    if !is_valid_key(&object_name) || raw.contains('%') || raw != url {
        return not_implemented(&method, &uri);
    }
    let upsert = header_str(&headers, "x-upsert") == Some("true");
    let owner_id = claims.sub.as_ref().map(|sub| sub.as_str());
    let owner = owner_id.filter(|sub| is_uuid(sub));
    let scope = Scope {
        actor: Actor::Request {
            claims: &claims,
            jwt,
        },
        method: &method,
        uri: &uri,
        headers: &headers,
        operation: OP_SIGN_UPLOAD_URL,
        roles: &state.api_roles,
    };
    if let Err(response) = can_upload(
        pool,
        &scope,
        &bucket_id,
        &object_name,
        owner,
        owner_id,
        &probe,
        upsert,
    )
    .await
    {
        return *response;
    }
    let now = Utc::now().timestamp();
    let mut payload = serde_json::Map::new();
    if let Some(owner_id) = owner_id {
        payload.insert("owner".into(), json!(owner_id));
    }
    payload.insert("url".into(), json!(url));
    payload.insert("upsert".into(), json!(upsert));
    payload.insert("scope".into(), json!(SCOPE_UPLOAD));
    payload.insert("iat".into(), json!(now));
    payload.insert("exp".into(), json!(now + UPLOAD_SIGNED_URL_EXPIRES_IN));
    match signer.sign(&Value::Object(payload)) {
        Ok(token) => fastify_json(
            StatusCode::OK,
            &json!({
                "url": format!("/object/upload/sign/{url}?token={}", token.as_str()),
                "token": token.as_str(),
            }),
        ),
        Err(_) => not_implemented(&method, &uri),
    }
}

/// JavaScript `Boolean(value)` for a JSON claim.
fn js_boolean(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// `isUploadScopedToken`: `scope` is `upload`, or absent on a token that
/// carries `upsert` (a legacy upload token).
fn is_upload_scoped(payload: &serde_json::Map<String, Value>) -> bool {
    match payload.get("scope") {
        Some(Value::String(scope)) => scope == SCOPE_UPLOAD,
        Some(_) => false,
        None => payload.contains_key("upsert"),
    }
}

/// `PUT /object/upload/sign/:bucketName/*?token=`: verify an upload token,
/// then store the object as the superuser. The permission check happened
/// when the token was signed.
// megabase:unit storage:route:PUT /storage/v1/object/upload/sign/{bucketName}/{*}
pub(crate) async fn put_signed_object(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (Ok(Path((bucket_id, object_name))), Some(pairs)) = (path, query_pairs(&uri)) else {
        return not_implemented(&method, &uri);
    };
    let Some(Some(token)) = single(&pairs, "token") else {
        return not_implemented(&method, &uri);
    };
    let Some(payload) = verified_signed_payload(&state, token) else {
        return match invalid_jwt(&state, token) {
            Some(error) => error.into_response(),
            None => not_implemented(&method, &uri),
        };
    };
    if !is_upload_scoped(&payload) {
        return StorageError::invalid_signature(Cow::Owned(format!(
            "Token is not scoped for {SCOPE_UPLOAD}"
        )))
        .into_response();
    }
    let expected = format!("{bucket_id}/{object_name}");
    if payload.get("url").and_then(Value::as_str) != Some(expected.as_str()) {
        return StorageError::invalid_signature(Cow::Borrowed("Invalid signature")).into_response();
    }
    let owner_id = match payload.get("owner") {
        None => None,
        Some(Value::String(owner)) => Some(owner.as_str()),
        Some(_) => return not_implemented(&method, &uri),
    };
    // `uploadFromRequest` passes the claim through `Boolean(file.isUpsert)`.
    // A missing claim is `undefined`, which is false. A non-boolean claim is
    // not rejected: only the JavaScript falsy JSON values stay false.
    let upsert = payload.get("upsert").is_some_and(js_boolean);
    let Some((pool, root)) = super::backend(&state) else {
        return not_implemented(&method, &uri);
    };
    let declared_length = header_str(&headers, header::CONTENT_LENGTH.as_str())
        .and_then(|value| value.parse::<u64>().ok());
    if headers.contains_key(header::CONTENT_ENCODING)
        || headers.contains_key("x-robots-tag")
        || headers.contains_key("x-metadata")
        || declared_length.is_none_or(|length| length == 0)
    {
        return not_implemented(&method, &uri);
    }
    let Some(file) = file_upload(&headers, &body) else {
        return not_implemented(&method, &uri);
    };
    if file.body.is_empty() {
        return not_implemented(&method, &uri);
    }
    let scope = Scope {
        actor: Actor::SuperUser,
        method: &method,
        uri: &uri,
        headers: &headers,
        operation: OP_UPLOAD_SIGNED,
    };
    let bucket = match fetch_upload_bucket(pool, &scope, &bucket_id).await {
        Ok(Some(bucket)) => bucket,
        Ok(None) => return StorageError::no_such_bucket().into_response(),
        Err(error) => return database_error(&error),
    };
    let mime_type = file.mime_type.as_ref();
    let allowed = bucket.allowed_mime_types.unwrap_or_default();
    if !allowed.is_empty()
        && !super::is_empty_folder(&object_name)
        && !is_allowed_mime_type(mime_type, &allowed)
    {
        return StorageError::invalid_mime_type(mime_type).into_response();
    }
    let max = max_file_size(&object_name, bucket.file_size_limit);
    let too_large = file.body.len() as u64 > max;
    if !file.multipart && (declared_length.is_some_and(|length| length > max) || too_large) {
        return StorageError::entity_too_large().into_response();
    }
    if !is_valid_key(&object_name) {
        return StorageError::invalid_key(&object_name).into_response();
    }
    let version = uuid::Uuid::new_v4().to_string();
    let Some(path) = object_file(root, &bucket_id, &object_name, &version) else {
        return not_implemented(&method, &uri);
    };
    let owner = owner_id.filter(|owner| is_uuid(owner));
    let probe = json!({ "mimetype": mime_type, "contentLength": declared_length });
    if let Err(response) = can_upload(
        pool,
        &scope,
        &bucket_id,
        &object_name,
        owner,
        owner_id,
        &probe,
        upsert,
    )
    .await
    {
        return *response;
    }
    if too_large {
        return StorageError::entity_too_large().into_response();
    }
    let bytes = &file.body;
    let last_modified = match write_file(&path, bytes).await {
        Ok(at) => at,
        Err(error) => {
            tracing::error!(%error, "storage signed upload write failed");
            return not_implemented(&method, &uri);
        }
    };
    let metadata = json!({
        "eTag": etag(bytes),
        "size": bytes.len(),
        "mimetype": mime_type,
        "cacheControl": file.cache_control,
        "lastModified": crate::iso(Some(last_modified)),
        "contentLength": bytes.len(),
        "httpStatusCode": 200,
    });
    let completed = complete_upload(
        pool,
        &scope,
        &bucket_id,
        &object_name,
        owner,
        owner_id,
        &metadata,
        &version,
        upsert,
    )
    .await;
    match completed {
        Ok((_id, previous)) => {
            if let Some(previous) =
                previous.and_then(|previous| object_file(root, &bucket_id, &object_name, &previous))
            {
                remove_file(&previous).await;
            }
            fastify_json(StatusCode::OK, &json!({ "Key": expected }))
        }
        Err(response) => {
            remove_file(&path).await;
            *response
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_many_bodies_follow_the_schema() {
        assert_eq!(
            sign_many_body(br#"{"expiresIn":60,"paths":["a.png","u1/b.png"]}"#),
            Some((60, vec!["a.png".to_string(), "u1/b.png".to_string()]))
        );
        // Upstream looks these up as they are and builds `signedURL` unescaped.
        assert_eq!(
            sign_many_body(br#"{"expiresIn":60,"paths":["a?b","a#b","a%20b"]}"#),
            Some((
                60,
                vec!["a?b".to_string(), "a#b".to_string(), "a%20b".to_string()]
            ))
        );
        let too_many = format!(
            r#"{{"expiresIn":60,"paths":[{}]}}"#,
            vec![r#""a""#; MAX_OBJECTS_PER_REQUEST + 1].join(",")
        );
        for body in [
            &br#"{"expiresIn":60,"paths":[]}"#[..],
            br#"{"expiresIn":0,"paths":["a"]}"#,
            br#"{"expiresIn":"60","paths":["a"]}"#,
            br#"{"expiresIn":1.5,"paths":["a"]}"#,
            br#"{"expiresIn":999999999999,"paths":["a"]}"#,
            br#"{"paths":["a"]}"#,
            br#"{"expiresIn":60}"#,
            br#"{"expiresIn":60,"paths":[1]}"#,
            br#"{"expiresIn":60,"paths":["a"],"extra":1}"#,
            too_many.as_bytes(),
            b"not json",
        ] {
            assert_eq!(
                sign_many_body(body),
                None,
                "{}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn upload_probes_read_type_and_length() {
        let mut headers = HeaderMap::new();
        assert_eq!(upload_probe(&headers), Some(json!({})));
        headers.insert(header::CONTENT_TYPE, "image/png".parse().unwrap());
        headers.insert(header::CONTENT_LENGTH, "12".parse().unwrap());
        assert_eq!(
            upload_probe(&headers),
            Some(json!({ "mimetype": "image/png", "contentLength": 12 }))
        );
        headers.insert(header::CONTENT_LENGTH, "1e3".parse().unwrap());
        assert_eq!(upload_probe(&headers), None);
    }

    #[test]
    fn upsert_claims_follow_javascript_boolean() {
        for value in [json!(false), json!(0), json!(0.0), json!(""), Value::Null] {
            assert!(!js_boolean(&value), "{value}");
        }
        for value in [
            json!(true),
            json!("false"),
            json!(1),
            json!(-1),
            json!([]),
            json!({}),
        ] {
            assert!(js_boolean(&value), "{value}");
        }
    }

    #[test]
    fn upload_tokens_are_scoped() {
        assert!(is_upload_scoped(
            json!({ "scope": "upload", "upsert": false })
                .as_object()
                .unwrap()
        ));
        assert!(is_upload_scoped(
            json!({ "upsert": true }).as_object().unwrap()
        ));
        assert!(!is_upload_scoped(
            json!({ "scope": "download" }).as_object().unwrap()
        ));
        assert!(!is_upload_scoped(
            json!({ "url": "a/b" }).as_object().unwrap()
        ));
    }

    #[tokio::test]
    async fn a_signed_upload_rejects_a_bad_token_and_stays_501_without_one() {
        use axum::body::Body;
        use tower::ServiceExt;

        const SECRET: &str = "super-secret-jwt-token-with-at-least-32-characters-long";
        let app = crate::router_with_state(StorageState::new(
            None,
            Some(megabase_core::Hs256::new(SECRET).unwrap()),
        ));
        let missing = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method(Method::PUT)
                    .uri("/storage/v1/object/upload/sign/avatars/a.png")
                    .header(header::CONTENT_TYPE, "text/plain")
                    .header(header::CONTENT_LENGTH, "4")
                    .body(Body::from("file"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_IMPLEMENTED);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method(Method::PUT)
                    .uri("/storage/v1/object/upload/sign/avatars/a.png?token=not-a-jwt")
                    .header(header::CONTENT_TYPE, "text/plain")
                    .header(header::CONTENT_LENGTH, "4")
                    .body(Body::from("file"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["code"], "InvalidJWT");
        assert_eq!(body["message"], "Invalid Compact JWS");
    }

    #[test]
    fn upload_sign_bodies_are_empty_or_json_objects() {
        let mut headers = HeaderMap::new();
        assert!(ignored_body(&headers, b""));
        assert!(!ignored_body(&headers, b"{}"));
        headers.insert(
            header::CONTENT_TYPE,
            "application/json; charset=utf-8".parse().unwrap(),
        );
        assert!(ignored_body(&headers, b"{}"));
        assert!(!ignored_body(&headers, b""));
        assert!(!ignored_body(&headers, b"[]"));
        headers.insert(header::CONTENT_TYPE, "text/plain".parse().unwrap());
        assert!(!ignored_body(&headers, b"{}"));
    }
}
