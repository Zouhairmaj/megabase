//! `/object` routes: upload, download, remove and signed URLs
//! (`specs/storage/object.md`); batch and upload signing live in [`sign`] and
//! object info in [`info`].
//!
//! Object bytes live under `FILE_STORAGE_BACKEND_PATH` as
//! `<bucket>/<name>/<version>`, like storage-api's file backend; rows live in
//! `storage.objects`. Queries run in the request scope storage-api sets
//! (`role`, `request.jwt.claims`, `storage.operation`, ...), so RLS policies
//! on `storage.objects` apply. Anything outside the spec is a 501.

// Ported from supabase/storage src/http/routes/object/createObject.ts,
// updateObject.ts, getObject.ts, getPublicObject.ts, getSignedObject.ts,
// getSignedURL.ts, deleteObject.ts, deleteObjects.ts, src/storage/object.ts,
// src/storage/uploader.ts, src/storage/database/pg.ts,
// src/storage/database/errors.ts, src/storage/backend/file.ts,
// src/storage/renderer/renderer.ts,
// src/internal/database/postgres/scope.ts and src/internal/auth/jwt.ts
// (Apache-2.0), pin v1.80.2.

use std::{
    borrow::Cow,
    path::{Path as FsPath, PathBuf},
};

use axum::{
    body::{Body, Bytes},
    extract::{rejection::PathRejection, OriginalUri, Path, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use md5::{Digest, Md5};
use megabase_core::{JwtClaims, JwtError};
use serde_json::{json, Map, Value};
use sqlx::{Postgres, Row, Transaction};

mod info;
mod sign;

pub(super) use info::{
    get_object_info_any, get_object_info_authenticated, get_public_object_info,
    head_authenticated_object,
};
pub(super) use sign::{sign_objects, sign_upload_object};

use super::{
    database_error, fastify_json, iso,
    limits::{is_uuid, is_valid_bucket_name, is_valid_key, UPLOAD_FILE_SIZE_LIMIT},
    mime::is_allowed_mime_type,
    multipart, not_implemented,
    render::{is_download_scoped, query_pairs, single, verified_signed_payload, SCOPE_DOWNLOAD},
    verified_claims, StorageError, StorageState,
};

/// `ROUTE_OPERATIONS` (`src/http/routes/operations.ts`).
const OP_UPLOAD: &str = "storage.object.upload";
const OP_UPLOAD_UPDATE: &str = "storage.object.upload_update";
const OP_GET_AUTHENTICATED: &str = "storage.object.get_authenticated";
const OP_GET_PUBLIC: &str = "storage.object.get_public";
const OP_GET_SIGNED: &str = "storage.object.get_signed";
const OP_SIGN: &str = "storage.object.sign";
const OP_DELETE: &str = "storage.object.delete";
const OP_DELETE_MANY: &str = "storage.object.delete_many";
const OP_LIST: &str = "storage.object.list";

/// `MAX_OBJECTS_PER_DELETE_BATCH`: storage-api deletes larger requests in
/// several transactions, which is not ported.
const MAX_OBJECTS_PER_DELETE_BATCH: usize = 500;

/// Largest `expiresIn` Megabase signs: ten years. storage-api's bound
/// depends on the clock (`getMaxNumericJWTExpiration`) and is not ported.
const MAX_SIGNED_URL_EXPIRES_IN: i64 = 315_360_000;

/// `scope.ts` `SCOPE_CONFIG_SQL`.
const SCOPE_SQL: &str = "SELECT set_config('role', $1, true), \
     set_config('request.jwt.claim.role', $2, true), \
     set_config('request.jwt', $3, true), \
     set_config('request.jwt.claim.sub', $4, true), \
     set_config('request.jwt.claims', $5, true), \
     set_config('request.headers', $6, true), \
     set_config('request.method', $7, true), \
     set_config('request.path', $8, true), \
     set_config('storage.operation', $9, true), \
     set_config('storage.allow_delete_query', 'true', true)";

/// `findBucketById(id, 'id, file_size_limit, allowed_mime_types')`.
const UPLOAD_BUCKET_SQL: &str =
    "SELECT id, file_size_limit, allowed_mime_types FROM storage.buckets WHERE id = $1 LIMIT 1";

/// `findBucket(id, 'id,public')`.
const BUCKET_PUBLIC_SQL: &str = "SELECT public FROM storage.buckets WHERE id = $1 LIMIT 1";

/// `findObject(bucket, name, 'id, version, metadata')` on the current version.
const FIND_OBJECT_SQL: &str = "SELECT id::text AS id, version, metadata FROM storage.objects \
     WHERE name COLLATE \"C\" = $1 AND bucket_id = $2 AND archived_at IS NULL LIMIT 1";

/// The same lookup `FOR UPDATE`, inside `completeUpload`.
const FIND_OBJECT_FOR_UPDATE_SQL: &str = "SELECT id::text AS id, version, metadata \
     FROM storage.objects \
     WHERE name COLLATE \"C\" = $1 AND bucket_id = $2 AND archived_at IS NULL \
     LIMIT 1 FOR UPDATE";

/// `createObject`: a plain insert.
const CREATE_OBJECT_SQL: &str = "INSERT INTO storage.objects \
     (name, owner, owner_id, bucket_id, metadata, version) \
     VALUES ($1, $2::uuid, $3, $4, $5, $6)";

/// `upsertObject` after `objects-current-version-index`.
const UPSERT_OBJECT_SQL: &str = "INSERT INTO storage.objects \
     (name, owner, owner_id, bucket_id, metadata, version) \
     VALUES ($1, $2::uuid, $3, $4, $5, $6) \
     ON CONFLICT (bucket_id, name COLLATE \"C\") WHERE archived_at IS NULL \
     DO UPDATE SET metadata = EXCLUDED.metadata, version = EXCLUDED.version, \
     owner = EXCLUDED.owner, owner_id = EXCLUDED.owner_id \
     RETURNING id::text AS id";

/// `deleteObject(bucket, name)` on the current version.
const DELETE_OBJECT_SQL: &str = "DELETE FROM storage.objects \
     WHERE name COLLATE \"C\" = $1 AND bucket_id = $2 AND archived_at IS NULL \
     RETURNING id";

/// `deleteObjects(bucket, names, 'name')`.
const DELETE_OBJECTS_SQL: &str = "DELETE FROM storage.objects \
     WHERE bucket_id = $1 AND name COLLATE \"C\" = ANY($2) AND archived_at IS NULL \
     RETURNING name, bucket_id, owner::text AS owner, owner_id, version, id::text AS id, \
     updated_at, created_at, last_accessed_at, metadata, user_metadata, archived_at, \
     is_delete_marker, is_versioned";

/// `searchObjects` once the `list-objects-with-versions` migration ran.
const SEARCH_OBJECTS_SQL: &str = "select name, id::text AS id, updated_at, created_at, \
     last_accessed_at, metadata, version, archived_at, is_delete_marker, is_versioned \
     from storage.search($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)";

/// Who a query runs as: the request JWT, or storage-api's superuser
/// connection (the service key user).
enum Actor<'a> {
    Request { claims: &'a JwtClaims, jwt: &'a str },
    SuperUser,
}

/// The per-request values `buildScopeStatement` binds.
struct Scope<'a> {
    actor: Actor<'a>,
    method: &'a Method,
    uri: &'a Uri,
    headers: &'a HeaderMap,
    operation: &'static str,
}

impl Scope<'_> {
    /// `request.url` as Fastify sees it behind Kong: the path after
    /// `/storage/v1`, with the query string.
    fn request_path(&self) -> String {
        let path = self.uri.path();
        let path = path.strip_prefix("/storage/v1").unwrap_or(path);
        match self.uri.query() {
            Some(query) => format!("{path}?{query}"),
            None => path.to_string(),
        }
    }

    fn headers_json(&self) -> String {
        let mut out = Map::new();
        for (name, value) in self.headers {
            if let Ok(value) = value.to_str() {
                out.insert(name.as_str().to_string(), json!(value));
            }
        }
        Value::Object(out).to_string()
    }

    /// A transaction with the scope applied.
    async fn begin(
        &self,
        pool: &sqlx::PgPool,
    ) -> Result<Transaction<'static, Postgres>, sqlx::Error> {
        let mut tx = pool.begin().await?;
        self.apply(&mut tx).await?;
        Ok(tx)
    }

    /// Switch an open transaction to this scope.
    async fn apply(&self, tx: &mut Transaction<'static, Postgres>) -> Result<(), sqlx::Error> {
        let (role, jwt, sub, claims) = match &self.actor {
            Actor::Request { claims, jwt } => (
                claims.role.as_deref().unwrap_or("anon"),
                *jwt,
                claims.sub.as_ref().map_or("", |sub| sub.as_str()),
                serde_json::to_string(&claims.raw).unwrap_or_default(),
            ),
            Actor::SuperUser => (
                "service_role",
                "",
                "",
                json!({ "role": "service_role" }).to_string(),
            ),
        };
        sqlx::query(SCOPE_SQL)
            .bind(role)
            .bind(role)
            .bind(jwt)
            .bind(sub)
            .bind(claims)
            .bind(self.headers_json())
            .bind(self.method.as_str())
            .bind(self.request_path())
            .bind(self.operation)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    fn as_super_user(&self) -> Scope<'_> {
        Scope {
            actor: Actor::SuperUser,
            method: self.method,
            uri: self.uri,
            headers: self.headers,
            operation: self.operation,
        }
    }
}

/// `DBError.fromDBError` for the codes the object routes reach; any other
/// database error is a 501.
fn pg_error(error: &sqlx::Error, duplicate: fn() -> StorageError) -> Response {
    let Some(db) = error.as_database_error() else {
        return database_error(error);
    };
    match db.code().as_deref() {
        Some("42501") => {
            let message = if db.message().contains("row-level security") {
                Cow::Borrowed("new row violates row-level security policy")
            } else {
                Cow::Owned(db.message().to_string())
            };
            StorageError::access_denied(message).into_response()
        }
        Some("23505") => duplicate().into_response(),
        _ => database_error(error),
    }
}

/// Whether every segment of `path` is safe to join under the storage root
/// (`resolveSecurePath` rejects escapes; Megabase answers 501 instead).
fn is_plain_path(path: &str) -> bool {
    path.split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// `<root>/<bucket>/<name>/<version>`, or `None` for a path Megabase does
/// not store.
fn object_file(root: &FsPath, bucket: &str, name: &str, version: &str) -> Option<PathBuf> {
    if !is_plain_path(bucket) || !is_plain_path(name) || !is_plain_path(version) {
        return None;
    }
    let mut path = root.join(bucket);
    for segment in name.split('/') {
        path.push(segment);
    }
    path.push(version);
    Some(path)
}

/// The file backend's default `md5` ETag: the quoted hex digest.
fn etag(bytes: &[u8]) -> String {
    let digest = Md5::digest(bytes);
    let mut out = String::with_capacity(34);
    out.push('"');
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out.push('"');
    out
}

/// `Date.prototype.toUTCString()`.
fn http_date(at: DateTime<Utc>) -> String {
    at.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// The object routes need a database and a storage root; routes that need a
/// verified JWT check for it separately (public reads do not).
fn backend(state: &StorageState) -> Option<(&sqlx::PgPool, &FsPath)> {
    Some((state.pool.as_ref()?, state.files.as_deref()?))
}

/// The bearer token as storage-api strips it, with its verified claims.
fn request_jwt<'a>(state: &StorageState, headers: &'a HeaderMap) -> Option<(JwtClaims, &'a str)> {
    let claims = verified_claims(state, headers)?;
    let token = header_str(headers, header::AUTHORIZATION.as_str())
        .and_then(megabase_core::bearer_token)?;
    Some((claims, token))
}

// ---------------------------------------------------------------------------
// Upload
// ---------------------------------------------------------------------------

/// `POST /object/:bucketName/*`: create, or replace with `x-upsert: true`.
// megabase:unit storage:route:POST /storage/v1/object/{bucketName}/{*}
pub(super) async fn create_object(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let upsert = header_str(&headers, "x-upsert") == Some("true");
    upload(
        &state, path, &method, &uri, &headers, &body, upsert, OP_UPLOAD,
    )
    .await
}

/// `PUT /object/:bucketName/*`: always an upsert.
// megabase:unit storage:route:PUT /storage/v1/object/{bucketName}/{*}
pub(super) async fn update_object(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    upload(
        &state,
        path,
        &method,
        &uri,
        &headers,
        &body,
        true,
        OP_UPLOAD_UPDATE,
    )
    .await
}

#[derive(sqlx::FromRow)]
struct UploadBucket {
    file_size_limit: Option<i64>,
    allowed_mime_types: Option<Vec<String>>,
}

/// `isEmptyFolder`.
fn is_empty_folder(name: &str) -> bool {
    name.ends_with(".emptyFolderPlaceholder")
}

/// `getStandardMaxFileSizeLimit` without `UPLOAD_FILE_SIZE_LIMIT_STANDARD`
/// (unset in the self-hosted compose file).
fn max_file_size(name: &str, bucket_limit: Option<i64>) -> u64 {
    if is_empty_folder(name) {
        return 0;
    }
    // `UPLOAD_FILE_SIZE_LIMIT` is an integral byte count.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let global = UPLOAD_FILE_SIZE_LIMIT as u64;
    match bucket_limit {
        Some(limit) => u64::try_from(limit).unwrap_or(0).min(global),
        None => global,
    }
}

#[allow(clippy::too_many_arguments)]
async fn upload(
    state: &StorageState,
    path: Result<Path<(String, String)>, PathRejection>,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &Bytes,
    upsert: bool,
    operation: &'static str,
) -> Response {
    let Ok(Path((bucket_id, object_name))) = path else {
        return not_implemented(method, uri);
    };
    let Some((pool, root)) = backend(state) else {
        return not_implemented(method, uri);
    };
    // storage-api's `AccessDenied` messages for a missing or invalid JWT are
    // not ported.
    let Some((claims, jwt)) = request_jwt(state, headers) else {
        return not_implemented(method, uri);
    };
    // Content encodings, custom metadata and `x-robots-tag` are not ported;
    // an upload without a declared length or with an empty file is outside
    // the spec.
    let declared_length = header_str(headers, header::CONTENT_LENGTH.as_str())
        .and_then(|value| value.parse::<u64>().ok());
    if headers.contains_key(header::CONTENT_ENCODING)
        || headers.contains_key("x-robots-tag")
        || declared_length.is_none_or(|length| length == 0)
    {
        return not_implemented(method, uri);
    }
    let Some(file) = file_upload(headers, body) else {
        return not_implemented(method, uri);
    };
    if file.body.is_empty() {
        return not_implemented(method, uri);
    }
    let scope = Scope {
        actor: Actor::Request {
            claims: &claims,
            jwt,
        },
        method,
        uri,
        headers,
        operation,
    };

    let bucket = match fetch_upload_bucket(pool, &scope, &bucket_id).await {
        Ok(Some(bucket)) => bucket,
        Ok(None) => return StorageError::no_such_bucket().into_response(),
        Err(error) => return database_error(&error),
    };
    let mime_type = file.mime_type.as_ref();
    let allowed = bucket.allowed_mime_types.unwrap_or_default();
    if !allowed.is_empty()
        && !is_empty_folder(&object_name)
        && !is_allowed_mime_type(mime_type, &allowed)
    {
        return StorageError::invalid_mime_type(mime_type).into_response();
    }
    // A binary upload is checked against its declared length up front; a
    // multipart file only once streamed (`isTruncated`), after `canUpload`.
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
        return not_implemented(method, uri);
    };
    let owner_id = claims.sub.as_ref().map(|sub| sub.as_str());
    let owner = owner_id.filter(|sub| is_uuid(sub));

    // `canUpload`: the insert under the request's RLS, rolled back.
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

    let body = &file.body;
    let last_modified = match write_file(&path, body).await {
        Ok(at) => at,
        Err(error) => {
            tracing::error!(%error, "storage object write failed");
            return not_implemented(method, uri);
        }
    };
    let metadata = json!({
        "eTag": etag(body),
        "size": body.len(),
        "mimetype": mime_type,
        "cacheControl": file.cache_control,
        "lastModified": iso(Some(last_modified)),
        "contentLength": body.len(),
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
        Ok((id, previous)) => {
            if let Some(previous) =
                previous.and_then(|previous| object_file(root, &bucket_id, &object_name, &previous))
            {
                remove_file(&previous).await;
            }
            fastify_json(
                StatusCode::OK,
                &json!({ "Id": id, "Key": format!("{bucket_id}/{object_name}") }),
            )
        }
        Err(response) => {
            remove_file(&path).await;
            *response
        }
    }
}

/// The uploaded file as `fileUploadFromRequest` reads it.
struct FileUpload<'a> {
    body: Cow<'a, Bytes>,
    mime_type: Cow<'a, str>,
    cache_control: Cow<'a, str>,
    multipart: bool,
}

/// A `multipart/form-data` body's file part (`multipart::parse`), or the
/// raw body with `Content-Type` (default `application/octet-stream`) and
/// `Cache-Control` (default `no-cache`). `None` for what is not ported:
/// a malformed or unsupported form, a form type in other letter case
/// (upstream's check is a case-sensitive `startsWith`, and how Fastify
/// parses such a body is not verified), and `x-metadata` on a binary upload.
fn file_upload<'a>(headers: &'a HeaderMap, body: &'a Bytes) -> Option<FileUpload<'a>> {
    let content_type = header_str(headers, header::CONTENT_TYPE.as_str());
    if content_type.is_some_and(|value| {
        value
            .get(.."multipart/form-data".len())
            .is_some_and(|prefix| {
                prefix.eq_ignore_ascii_case("multipart/form-data")
                    && prefix != "multipart/form-data"
            })
    }) {
        return None;
    }
    if let Some(content_type) =
        content_type.filter(|value| value.starts_with("multipart/form-data"))
    {
        let form = multipart::parse(content_type, body)?;
        return Some(FileUpload {
            body: Cow::Owned(form.body),
            mime_type: Cow::Owned(form.mime_type),
            cache_control: Cow::Owned(form.cache_control),
            multipart: true,
        });
    }
    if headers.contains_key("x-metadata") {
        return None;
    }
    Some(FileUpload {
        body: Cow::Borrowed(body),
        mime_type: Cow::Borrowed(content_type.unwrap_or("application/octet-stream")),
        cache_control: Cow::Borrowed(
            header_str(headers, header::CACHE_CONTROL.as_str()).unwrap_or("no-cache"),
        ),
        multipart: false,
    })
}

async fn fetch_upload_bucket(
    pool: &sqlx::PgPool,
    scope: &Scope<'_>,
    bucket_id: &str,
) -> Result<Option<UploadBucket>, sqlx::Error> {
    let mut tx = scope.as_super_user().begin(pool).await?;
    let bucket = sqlx::query_as::<_, UploadBucket>(UPLOAD_BUCKET_SQL)
        .bind(bucket_id)
        .fetch_optional(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(bucket)
}

#[allow(clippy::too_many_arguments)]
async fn can_upload(
    pool: &sqlx::PgPool,
    scope: &Scope<'_>,
    bucket_id: &str,
    object_name: &str,
    owner: Option<&str>,
    owner_id: Option<&str>,
    metadata: &Value,
    upsert: bool,
) -> Result<(), Box<Response>> {
    let sql = if upsert {
        UPSERT_OBJECT_SQL
    } else {
        CREATE_OBJECT_SQL
    };
    let attempt = async {
        let mut tx = scope.begin(pool).await?;
        sqlx::query(sql)
            .bind(object_name)
            .bind(owner)
            .bind(owner_id)
            .bind(bucket_id)
            .bind(metadata)
            .bind("1")
            .execute(&mut *tx)
            .await?;
        tx.rollback().await
    };
    attempt
        .await
        .map_err(|error| Box::new(pg_error(&error, StorageError::key_already_exists)))
}

/// `completeUpload`: as the superuser, refuse to replace an existing
/// object without upsert, then write the row. Returns the row id and the
/// version whose file is now unreferenced.
#[allow(clippy::too_many_arguments)]
async fn complete_upload(
    pool: &sqlx::PgPool,
    scope: &Scope<'_>,
    bucket_id: &str,
    object_name: &str,
    owner: Option<&str>,
    owner_id: Option<&str>,
    metadata: &Value,
    version: &str,
    upsert: bool,
) -> Result<(String, Option<String>), Box<Response>> {
    let attempt = async {
        let mut tx = scope.as_super_user().begin(pool).await?;
        let current = sqlx::query(FIND_OBJECT_FOR_UPDATE_SQL)
            .bind(object_name)
            .bind(bucket_id)
            .fetch_optional(&mut *tx)
            .await?;
        let previous: Option<Option<String>> = current
            .as_ref()
            .map(|row| row.try_get::<Option<String>, _>("version"))
            .transpose()?;
        if !upsert && previous.is_some() {
            return Ok(Err(StorageError::key_already_exists()));
        }
        let row = sqlx::query(UPSERT_OBJECT_SQL)
            .bind(object_name)
            .bind(owner)
            .bind(owner_id)
            .bind(bucket_id)
            .bind(metadata)
            .bind(version)
            .fetch_one(&mut *tx)
            .await?;
        let id: String = row.try_get("id")?;
        tx.commit().await?;
        Ok::<_, sqlx::Error>(Ok((id, previous.flatten())))
    };
    match attempt.await {
        Ok(Ok(done)) => Ok(done),
        Ok(Err(error)) => Err(Box::new(error.into_response())),
        Err(error) => Err(Box::new(pg_error(
            &error,
            StorageError::resource_already_exists,
        ))),
    }
}

async fn write_file(file: &FsPath, body: &[u8]) -> std::io::Result<DateTime<Utc>> {
    if let Some(parent) = file.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(file, body).await?;
    let modified = tokio::fs::metadata(file).await?.modified()?;
    Ok(DateTime::<Utc>::from(modified))
}

/// Removes `file`, treating a file that is already gone as removed.
async fn try_remove_file(file: &FsPath) -> std::io::Result<()> {
    match tokio::fs::remove_file(file).await {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

async fn remove_file(file: &FsPath) {
    if let Err(error) = tokio::fs::remove_file(file).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(%error, "storage object file removal failed");
        }
    }
}

// ---------------------------------------------------------------------------
// Download
// ---------------------------------------------------------------------------

/// A found object row.
struct FoundObject {
    version: Option<String>,
    metadata: Option<Value>,
}

async fn find_object(
    pool: &sqlx::PgPool,
    scope: &Scope<'_>,
    bucket_id: &str,
    object_name: &str,
) -> Result<Option<FoundObject>, sqlx::Error> {
    let mut tx = scope.begin(pool).await?;
    let row = sqlx::query(FIND_OBJECT_SQL)
        .bind(object_name)
        .bind(bucket_id)
        .fetch_optional(&mut *tx)
        .await?;
    tx.commit().await?;
    row.map(|row| {
        Ok(FoundObject {
            version: row.try_get("version")?,
            metadata: row.try_get("metadata")?,
        })
    })
    .transpose()
}

/// `findBucket(id, 'id,public')` as the superuser: `None` when missing.
async fn bucket_is_public(
    pool: &sqlx::PgPool,
    scope: &Scope<'_>,
    bucket_id: &str,
) -> Result<Option<bool>, sqlx::Error> {
    let mut tx = scope.as_super_user().begin(pool).await?;
    let row = sqlx::query(BUCKET_PUBLIC_SQL)
        .bind(bucket_id)
        .fetch_optional(&mut *tx)
        .await?;
    tx.commit().await?;
    row.map(|row| {
        row.try_get::<Option<bool>, _>("public")
            .map(|public| public.unwrap_or(false))
    })
    .transpose()
}

/// The `download` query value, when Megabase renders its
/// `Content-Disposition` exactly: absent, empty, or a filename whose
/// fallback and RFC 5987 forms are the name itself.
enum Download {
    No,
    Yes(String),
}

/// The query options `getObjectQuerySchema` allows that Megabase serves.
/// `versionId`, a repeated key, an undecodable query and a filename that
/// `toFilenameFallback` or `encodeExtValue` would change are 501.
fn download_option(uri: &Uri) -> Option<Download> {
    let pairs = query_pairs(uri)?;
    if single(&pairs, "versionId")?.is_some() {
        return None;
    }
    match single(&pairs, "download")? {
        None => Some(Download::No),
        Some(value) => value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            .then(|| Download::Yes(value.to_string())),
    }
}

/// Conditional and range requests are not ported.
fn has_conditional_headers(headers: &HeaderMap) -> bool {
    [
        header::RANGE,
        header::IF_MATCH,
        header::IF_NONE_MATCH,
        header::IF_MODIFIED_SINCE,
        header::IF_UNMODIFIED_SINCE,
    ]
    .iter()
    .any(|name| headers.contains_key(name))
}

/// `renderer('asset').render` for a found row: the file and its headers.
/// `expires` is the signed URL's `exp` (`Expires` replaces `Cache-Control`).
#[allow(clippy::too_many_arguments)]
async fn render_asset(
    root: &FsPath,
    bucket_id: &str,
    object_name: &str,
    object: &FoundObject,
    download: &Download,
    expires: Option<i64>,
    method: &Method,
    uri: &Uri,
) -> Response {
    let metadata = object.metadata.as_ref().and_then(Value::as_object);
    let text = |key: &str| metadata.and_then(|m| m.get(key)).and_then(Value::as_str);
    if metadata.is_some_and(|m| m.contains_key("xRobotsTag") || m.contains_key("contentEncoding")) {
        return not_implemented(method, uri);
    }
    let Some(file) = object
        .version
        .as_deref()
        .and_then(|version| object_file(root, bucket_id, object_name, version))
    else {
        return not_implemented(method, uri);
    };
    let (bytes, modified) = match read_file(&file).await {
        Ok(Some(found)) => found,
        Ok(None) => return not_found_asset(),
        Err(error) => {
            tracing::error!(%error, "storage object read failed");
            return not_implemented(method, uri);
        }
    };
    let mimetype = text("mimetype").unwrap_or("application/octet-stream");
    let content_type = if mimetype.to_lowercase().contains("text/html") {
        "text/plain"
    } else {
        mimetype
    };
    let cache_control = text("cacheControl").unwrap_or("no-cache");
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ETAG, etag(&bytes))
        .header("x-robots-tag", "none")
        .header(header::LAST_MODIFIED, http_date(modified))
        .header(header::CONTENT_LENGTH, bytes.len());
    match HeaderValue::from_str(content_type) {
        Ok(value) => response = response.header(header::CONTENT_TYPE, value),
        Err(_) => return not_implemented(method, uri),
    }
    if let Some(exp) = expires {
        let Some(at) = DateTime::from_timestamp(exp, 0) else {
            return not_implemented(method, uri);
        };
        response = response.header(header::EXPIRES, http_date(at));
        if cache_control
            .split(',')
            .any(|directive| directive.trim().eq_ignore_ascii_case("no-transform"))
        {
            response = response.header(header::CACHE_CONTROL, "no-transform");
        }
    } else if !cache_control.is_empty() {
        match HeaderValue::from_str(cache_control) {
            Ok(value) => response = response.header(header::CACHE_CONTROL, value),
            Err(_) => return not_implemented(method, uri),
        }
    }
    match download {
        Download::No => {}
        Download::Yes(name) if name.is_empty() => {
            response = response.header(header::CONTENT_DISPOSITION, "attachment;");
        }
        Download::Yes(name) => {
            response = response.header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename={name}; filename*=UTF-8''{name}"),
            );
        }
    }
    response
        .body(Body::from(bytes))
        .unwrap_or_else(|_| not_implemented(method, uri))
}

/// The renderer's answer when the backend has no file for a found row.
fn not_found_asset() -> Response {
    let mut response = fastify_json(
        StatusCode::BAD_REQUEST,
        &json!({
            "statusCode": "404",
            "code": "NoSuchKey",
            "error": "Not found",
            "message": "The resource was not found",
        }),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn read_file(file: &FsPath) -> std::io::Result<Option<(Vec<u8>, DateTime<Utc>)>> {
    match tokio::fs::read(file).await {
        Ok(bytes) => {
            let modified = tokio::fs::metadata(file).await?.modified()?;
            Ok(Some((bytes, DateTime::<Utc>::from(modified))))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// `GET /object/authenticated/:bucketName/*`: a valid JWT is required.
// megabase:unit storage:route:GET /storage/v1/object/authenticated/{bucketName}/{*}
pub(super) async fn get_authenticated_object(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let Some(jwt) = request_jwt(&state, &headers) else {
        return not_implemented(&method, &uri);
    };
    get_object(&state, path, &method, &uri, &headers, Some(jwt)).await
}

/// `GET /object/:bucketName/*`: `allowInvalidJwt`, so a missing or invalid
/// JWT reads public buckets only.
// megabase:unit storage:route:GET /storage/v1/object/{bucketName}/{*}
pub(super) async fn get_object_any(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let jwt = request_jwt(&state, &headers);
    get_object(&state, path, &method, &uri, &headers, jwt).await
}

/// `allowInvalidJwt`'s fallback payload `{role: anon}`.
fn anon_claims() -> JwtClaims {
    JwtClaims {
        role: Some("anon".into()),
        sub: None,
        exp: None,
        raw: Map::new(),
    }
}

/// `getObject.ts` `requestHandler`.
async fn get_object(
    state: &StorageState,
    path: Result<Path<(String, String)>, PathRejection>,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    jwt: Option<(JwtClaims, &str)>,
) -> Response {
    let Ok(Path((bucket_id, object_name))) = path else {
        return not_implemented(method, uri);
    };
    let (Some((pool, root)), Some(download)) = (backend(state), download_option(uri)) else {
        return not_implemented(method, uri);
    };
    if has_conditional_headers(headers) {
        return not_implemented(method, uri);
    }
    let anon = anon_claims();
    let authenticated = jwt.is_some();
    let (claims, token) = match &jwt {
        Some((claims, token)) => (claims, *token),
        None => (&anon, ""),
    };
    let scope = Scope {
        actor: Actor::Request { claims, jwt: token },
        method,
        uri,
        headers,
        operation: OP_GET_AUTHENTICATED,
    };
    let public = match bucket_is_public(pool, &scope, &bucket_id).await {
        Ok(public) => public,
        Err(error) => return database_error(&error),
    };
    let public = match public {
        Some(public) if public || authenticated => public,
        _ => return StorageError::no_such_bucket().into_response(),
    };
    if !is_valid_key(&object_name) {
        return StorageError::invalid_key(&object_name).into_response();
    }
    let lookup = if public {
        find_object(pool, &scope.as_super_user(), &bucket_id, &object_name).await
    } else {
        find_object(pool, &scope, &bucket_id, &object_name).await
    };
    match lookup {
        Ok(Some(object)) => {
            render_asset(
                root,
                &bucket_id,
                &object_name,
                &object,
                &download,
                None,
                method,
                uri,
            )
            .await
        }
        Ok(None) => StorageError::no_such_key().into_response(),
        Err(error) => database_error(&error),
    }
}

/// `GET /object/public/:bucketName/*`: the bucket must be public; no JWT.
/// storage-api runs the bucket and object lookups with `Promise.all`, so
/// when both fail the reported error is a race and Megabase answers 501.
// megabase:unit storage:route:GET /storage/v1/object/public/{bucketName}/{*}
pub(super) async fn get_public_object(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let Ok(Path((bucket_id, object_name))) = path else {
        return not_implemented(&method, &uri);
    };
    let (Some((pool, root)), Some(download)) = (backend(&state), download_option(&uri)) else {
        return not_implemented(&method, &uri);
    };
    if has_conditional_headers(&headers) {
        return not_implemented(&method, &uri);
    }
    // `findObject` throws `InvalidKey` synchronously, before either query.
    if !is_valid_key(&object_name) {
        return StorageError::invalid_key(&object_name).into_response();
    }
    let scope = Scope {
        actor: Actor::SuperUser,
        method: &method,
        uri: &uri,
        headers: &headers,
        operation: OP_GET_PUBLIC,
    };
    let bucket = bucket_is_public(pool, &scope, &bucket_id).await;
    let object = find_object(pool, &scope, &bucket_id, &object_name).await;
    match (bucket, object) {
        (Ok(Some(true)), Ok(Some(object))) => {
            render_asset(
                root,
                &bucket_id,
                &object_name,
                &object,
                &download,
                None,
                &method,
                &uri,
            )
            .await
        }
        (Ok(Some(true)), Ok(None)) => StorageError::no_such_key().into_response(),
        (Ok(_), Ok(Some(_))) => StorageError::no_such_bucket().into_response(),
        (Ok(_), Ok(None)) => not_implemented(&method, &uri),
        (Err(error), _) | (_, Err(error)) => database_error(&error),
    }
}

// ---------------------------------------------------------------------------
// Signed URLs
// ---------------------------------------------------------------------------

/// `POST /object/sign/:bucketName/*` with `{"expiresIn": n}`: a download
/// token for an object the request role can read.
// megabase:unit storage:route:POST /storage/v1/object/sign/{bucketName}/{*}
pub(super) async fn sign_object(
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
    // Fastify body validation messages, `transform` and `versionId` are not
    // ported: only `{"expiresIn": <integer 1..=10 years>}` is served.
    let expires_in = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|body| {
            let body = body.as_object()?;
            if body.keys().any(|key| key != "expiresIn") {
                return None;
            }
            body.get("expiresIn")?
                .as_i64()
                .filter(|n| (1..=MAX_SIGNED_URL_EXPIRES_IN).contains(n))
        });
    let Some(expires_in) = expires_in else {
        return not_implemented(&method, &uri);
    };
    let (Some((pool, _root)), Some(signer)) = (backend(&state), state.jwt.as_deref()) else {
        return not_implemented(&method, &uri);
    };
    let Some((claims, jwt)) = request_jwt(&state, &headers) else {
        return not_implemented(&method, &uri);
    };
    // `urlToSign` is `decodeURI` of the raw path; Megabase serves paths
    // without percent escapes, where it is the raw path itself.
    let raw = uri
        .path()
        .strip_prefix("/storage/v1/object/sign/")
        .unwrap_or_default();
    if raw.contains('%') || raw != format!("{bucket_id}/{object_name}") {
        return not_implemented(&method, &uri);
    }
    if !is_valid_key(&object_name) {
        return StorageError::invalid_key(&object_name).into_response();
    }
    let scope = Scope {
        actor: Actor::Request {
            claims: &claims,
            jwt,
        },
        method: &method,
        uri: &uri,
        headers: &headers,
        operation: OP_SIGN,
    };
    match find_object(pool, &scope, &bucket_id, &object_name).await {
        Ok(Some(_)) => {}
        Ok(None) => return StorageError::no_such_key().into_response(),
        Err(error) => return database_error(&error),
    }
    let now = Utc::now().timestamp();
    let payload = json!({
        "url": raw,
        "scope": SCOPE_DOWNLOAD,
        "iat": now,
        "exp": now + expires_in,
    });
    match signer.sign(&payload) {
        Ok(token) => fastify_json(
            StatusCode::OK,
            &json!({ "signedURL": format!("/object/sign/{raw}?token={}", token.as_str()) }),
        ),
        Err(_) => not_implemented(&method, &uri),
    }
}

/// jose's message for the two `jwtVerify` failures Megabase reproduces.
fn invalid_jwt(state: &StorageState, token: &str) -> Option<StorageError> {
    let verifier = state.jwt.as_deref()?;
    match verifier.verify_at(token, i64::MIN / 2) {
        Ok(claims) => claims
            .exp
            .filter(|exp| *exp <= Utc::now().timestamp())
            .map(|_| StorageError::invalid_jwt("\"exp\" claim timestamp check failed")),
        Err(JwtError::KeyError) => Some(StorageError::invalid_jwt("signature verification failed")),
        Err(_) => None,
    }
}

/// `GET /object/sign/:bucketName/*?token=`: `verifyObjectSignature` with the
/// download scope, then the object the token's `url` names, as the
/// superuser.
// megabase:unit storage:route:GET /storage/v1/object/sign/{bucketName}/{*}
pub(super) async fn get_signed_object(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let (Ok(Path((bucket_name, object_name))), Some(pairs)) = (path, query_pairs(&uri)) else {
        return not_implemented(&method, &uri);
    };
    let (Some(Some(token)), Some(None)) = (single(&pairs, "token"), single(&pairs, "download"))
    else {
        // `download` on a signed URL is served by `download_option` below
        // only when absent; a required `token` that is missing is a Fastify
        // validation message.
        return not_implemented(&method, &uri);
    };
    let Some((pool, root)) = backend(&state) else {
        return not_implemented(&method, &uri);
    };
    if has_conditional_headers(&headers) {
        return not_implemented(&method, &uri);
    }
    let Some(payload) = verified_signed_payload(&state, token) else {
        return match invalid_jwt(&state, token) {
            Some(error) => error.into_response(),
            None => not_implemented(&method, &uri),
        };
    };
    if !is_download_scoped(&payload) {
        return StorageError::invalid_signature(Cow::Owned(format!(
            "Token is not scoped for {SCOPE_DOWNLOAD}"
        )))
        .into_response();
    }
    let expected = format!("{bucket_name}/{object_name}");
    let Some(url) = payload
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| *url == expected)
    else {
        return StorageError::invalid_signature(Cow::Borrowed("Invalid signature")).into_response();
    };
    if payload.contains_key("versionId") {
        return not_implemented(&method, &uri);
    }
    let Some(exp) = payload.get("exp").and_then(Value::as_i64) else {
        return not_implemented(&method, &uri);
    };
    let (token_bucket, token_object) = url.split_once('/').unwrap_or((url, ""));
    if !is_valid_key(token_object) {
        return StorageError::invalid_key(token_object).into_response();
    }
    let scope = Scope {
        actor: Actor::SuperUser,
        method: &method,
        uri: &uri,
        headers: &headers,
        operation: OP_GET_SIGNED,
    };
    match find_object(pool, &scope, token_bucket, token_object).await {
        Ok(Some(object)) => {
            render_asset(
                root,
                token_bucket,
                token_object,
                &object,
                &Download::No,
                Some(exp),
                &method,
                &uri,
            )
            .await
        }
        Ok(None) => StorageError::no_such_key().into_response(),
        Err(error) => database_error(&error),
    }
}

// ---------------------------------------------------------------------------
// Remove
// ---------------------------------------------------------------------------

/// The `prefixes` of a `deleteObjectsBodySchema` body Megabase serves: a
/// non-empty array of plain names, at most one delete batch.
fn delete_prefixes(body: &[u8]) -> Option<Vec<String>> {
    let body: Value = serde_json::from_slice(body).ok()?;
    let prefixes = body.as_object()?.get("prefixes")?.as_array()?;
    if prefixes.is_empty() || prefixes.len() > MAX_OBJECTS_PER_DELETE_BATCH {
        return None;
    }
    prefixes
        .iter()
        .map(|prefix| prefix.as_str().map(str::to_string))
        .collect()
}

/// A timestamp column through `objectSchema`.
fn timestamp(row: &sqlx::postgres::PgRow, column: &str) -> Result<Value, sqlx::Error> {
    Ok(row
        .try_get::<Option<DateTime<Utc>>, _>(column)?
        .map_or(Value::Null, |at| json!(iso(Some(at)))))
}

/// A deleted row through fast-json-stringify and `objectSchema`: schema
/// property order, a `null` plain string is `""`.
fn deleted_object_json(row: &sqlx::postgres::PgRow) -> Result<Value, sqlx::Error> {
    let text = |column: &str| -> Result<Value, sqlx::Error> {
        Ok(json!(row
            .try_get::<Option<String>, _>(column)?
            .unwrap_or_default()))
    };
    let mut out = Map::new();
    out.insert("name".into(), text("name")?);
    out.insert("bucket_id".into(), text("bucket_id")?);
    out.insert("owner".into(), text("owner")?);
    out.insert("owner_id".into(), text("owner_id")?);
    out.insert("version".into(), text("version")?);
    out.insert("id".into(), json!(row.try_get::<Option<String>, _>("id")?));
    out.insert("updated_at".into(), timestamp(row, "updated_at")?);
    out.insert("created_at".into(), timestamp(row, "created_at")?);
    out.insert(
        "last_accessed_at".into(),
        timestamp(row, "last_accessed_at")?,
    );
    out.insert(
        "metadata".into(),
        json!(row.try_get::<Option<Value>, _>("metadata")?),
    );
    out.insert(
        "user_metadata".into(),
        json!(row.try_get::<Option<Value>, _>("user_metadata")?),
    );
    out.insert("archived_at".into(), timestamp(row, "archived_at")?);
    out.insert(
        "is_delete_marker".into(),
        json!(row.try_get::<bool, _>("is_delete_marker")?),
    );
    out.insert(
        "is_versioned".into(),
        json!(row.try_get::<bool, _>("is_versioned")?),
    );
    Ok(Value::Object(out))
}

/// `DELETE /object/:bucketName` with `{"prefixes": [...]}`: delete the
/// current rows the request role may delete, then their files.
// megabase:unit storage:route:DELETE /storage/v1/object/{bucketName}
pub(super) async fn delete_objects(
    State(state): State<StorageState>,
    Path(bucket_id): Path<String>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // `{path, versionId}` entries and Fastify validation messages are not
    // ported.
    let Some(prefixes) = delete_prefixes(&body) else {
        return not_implemented(&method, &uri);
    };
    let Some((pool, root)) = backend(&state) else {
        return not_implemented(&method, &uri);
    };
    let Some((claims, jwt)) = request_jwt(&state, &headers) else {
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
        operation: OP_DELETE_MANY,
    };
    let attempt = async {
        let mut tx = scope.begin(pool).await?;
        let rows = sqlx::query(DELETE_OBJECTS_SQL)
            .bind(&bucket_id)
            .bind(&prefixes)
            .fetch_all(&mut *tx)
            .await?;
        let mut deleted = Vec::with_capacity(rows.len());
        let mut files = Vec::with_capacity(rows.len());
        for row in &rows {
            deleted.push(deleted_object_json(row)?);
            let name: Option<String> = row.try_get("name")?;
            let version: Option<String> = row.try_get("version")?;
            if let (Some(name), Some(version)) = (name, version) {
                files.extend(object_file(root, &bucket_id, &name, &version));
            }
        }
        tx.commit().await?;
        Ok::<_, sqlx::Error>((deleted, files))
    };
    match attempt.await {
        Ok((deleted, files)) => {
            for file in &files {
                remove_file(file).await;
            }
            fastify_json(StatusCode::OK, &Value::Array(deleted))
        }
        Err(error) => pg_error(&error, StorageError::resource_already_exists),
    }
}

/// `DELETE /object/:bucketName/*`: in one transaction, lock the current row
/// as the superuser (`NoSuchKey`), delete it as the request role (nothing
/// deleted is `AccessDenied`), then remove its file. There is no
/// `mustBeValidKey` here.
// megabase:unit storage:route:DELETE /storage/v1/object/{bucketName}/{*}
pub(super) async fn delete_object(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let Ok(Path((bucket_id, object_name))) = path else {
        return not_implemented(&method, &uri);
    };
    // `versionId` is not ported.
    if query_pairs(&uri).and_then(|pairs| single(&pairs, "versionId").map(|v| v.is_none()))
        != Some(true)
    {
        return not_implemented(&method, &uri);
    }
    let Some((pool, root)) = backend(&state) else {
        return not_implemented(&method, &uri);
    };
    let Some((claims, jwt)) = request_jwt(&state, &headers) else {
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
        operation: OP_DELETE,
    };
    let attempt = async {
        let mut tx = scope.as_super_user().begin(pool).await?;
        let Some(current) = sqlx::query(FIND_OBJECT_FOR_UPDATE_SQL)
            .bind(&object_name)
            .bind(&bucket_id)
            .fetch_optional(&mut *tx)
            .await?
        else {
            return Ok(Err(StorageError::no_such_key().into_response()));
        };
        let version: Option<String> = current.try_get("version")?;
        // A row Megabase could not have stored a file for.
        let file = match version {
            Some(version) => match object_file(root, &bucket_id, &object_name, &version) {
                Some(file) => Some(file),
                None => return Ok(Err(not_implemented(&method, &uri))),
            },
            None => None,
        };
        scope.apply(&mut tx).await?;
        let deleted = sqlx::query(DELETE_OBJECT_SQL)
            .bind(&object_name)
            .bind(&bucket_id)
            .fetch_optional(&mut *tx)
            .await?;
        if deleted.is_none() {
            return Ok(Err(StorageError::access_denied(Cow::Borrowed(
                "Access denied",
            ))
            .into_response()));
        }
        // Upstream removes the file inside the transaction, so a backend
        // failure drops `tx` (rollback) and the row survives for a retry.
        if let Some(file) = file {
            if let Err(error) = try_remove_file(&file).await {
                tracing::error!(%error, "storage object file removal failed");
                return Ok(Err(not_implemented(&method, &uri)));
            }
        }
        tx.commit().await?;
        Ok::<_, sqlx::Error>(Ok(()))
    };
    match attempt.await {
        Ok(Ok(())) => fastify_json(
            StatusCode::OK,
            &json!({ "message": "Successfully deleted" }),
        ),
        Ok(Err(response)) => response,
        Err(error) => pg_error(&error, StorageError::resource_already_exists),
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// A `searchRequestBodySchema` body Megabase serves.
#[derive(Debug, PartialEq)]
struct ListOptions {
    prefix: String,
    limit: Option<i32>,
    offset: Option<i32>,
    sort_column: Option<String>,
    sort_order: Option<String>,
    search: Option<String>,
    noncurrent_versions: Option<String>,
    delete_markers: Option<String>,
}

/// An optional string property: `None` when absent, `Err` when present
/// but not a string or outside `allowed` (when given).
fn optional_str(
    body: &Map<String, Value>,
    key: &str,
    allowed: Option<&[&str]>,
) -> Result<Option<String>, ()> {
    match body.get(key) {
        None => Ok(None),
        Some(Value::String(value))
            if allowed.is_none_or(|allowed| allowed.contains(&value.as_str())) =>
        {
            Ok(Some(value.clone()))
        }
        Some(_) => Err(()),
    }
}

/// An optional integer property of at least `minimum` that PostgreSQL
/// binds as `integer`.
fn optional_int(body: &Map<String, Value>, key: &str, minimum: i32) -> Result<Option<i32>, ()> {
    match body.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .filter(|n| *n >= minimum)
            .map(Some)
            .ok_or(()),
    }
}

/// Parse a list body. Fastify validation messages, type coercion
/// (`"10"`, `null`) and `exactMatch: true` are not ported: `None`.
fn list_options(body: &[u8]) -> Option<ListOptions> {
    const VERSION_FILTERS: &[&str] = &["exclude", "include", "only"];
    let body: Value = serde_json::from_slice(body).ok()?;
    let body = body.as_object()?;
    let prefix = optional_str(body, "prefix", None).ok()??;
    match body.get("exactMatch") {
        None | Some(Value::Bool(false)) => {}
        Some(_) => return None,
    }
    let (sort_column, sort_order) = match body.get("sortBy") {
        None => (None, None),
        Some(Value::Object(sort_by)) => (
            Some(
                optional_str(
                    sort_by,
                    "column",
                    Some(&["name", "updated_at", "created_at", "last_accessed_at"]),
                )
                .ok()??,
            ),
            optional_str(sort_by, "order", Some(&["asc", "desc"])).ok()?,
        ),
        Some(_) => return None,
    };
    Some(ListOptions {
        prefix,
        limit: optional_int(body, "limit", 1).ok()?,
        offset: optional_int(body, "offset", 0).ok()?,
        sort_column,
        sort_order,
        search: optional_str(body, "search", None).ok()?,
        noncurrent_versions: optional_str(body, "noncurrentVersions", Some(VERSION_FILTERS))
            .ok()?,
        delete_markers: optional_str(body, "deleteMarkers", Some(VERSION_FILTERS)).ok()?,
    })
}

/// `escapeLike`.
fn escape_like(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The `storage.search` arguments `searchObjects` binds, after
/// `ObjectStorage.searchObjects` makes a non-empty prefix a folder.
#[derive(Debug, PartialEq)]
struct SearchArgs {
    prefix: String,
    limit: i32,
    levels: i32,
    offset: i32,
    search: String,
    sort_column: String,
    sort_order: String,
    noncurrent_versions: String,
    delete_markers: String,
}

impl ListOptions {
    fn search_args(&self) -> SearchArgs {
        let mut prefix = self.prefix.clone();
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        let sort_column = self.sort_column.as_deref().unwrap_or("name");
        let search = self.search.as_deref().unwrap_or_default();
        let (prefix, search) = if sort_column == "name" {
            (prefix, search.to_string())
        } else {
            (escape_like(&prefix), escape_like(search))
        };
        // `(prefix + search).split('/').length`; unused by the function.
        let levels = prefix.matches('/').count() + search.matches('/').count() + 1;
        SearchArgs {
            limit: self.limit.unwrap_or(100),
            levels: i32::try_from(levels).unwrap_or(i32::MAX),
            offset: self.offset.unwrap_or(0),
            prefix,
            search,
            sort_column: sort_column.to_string(),
            sort_order: self.sort_order.as_deref().unwrap_or("asc").to_string(),
            noncurrent_versions: self
                .noncurrent_versions
                .as_deref()
                .unwrap_or("exclude")
                .to_string(),
            delete_markers: self
                .delete_markers
                .as_deref()
                .unwrap_or("exclude")
                .to_string(),
        }
    }
}

/// A `storage.search` row through fast-json-stringify and
/// `objectListEntrySchema`: schema property order, folders keep their
/// `null`s.
fn list_entry_json(row: &sqlx::postgres::PgRow) -> Result<Value, sqlx::Error> {
    let mut out = Map::new();
    out.insert(
        "name".into(),
        json!(row
            .try_get::<Option<String>, _>("name")?
            .unwrap_or_default()),
    );
    out.insert(
        "version".into(),
        json!(row.try_get::<Option<String>, _>("version")?),
    );
    out.insert("id".into(), json!(row.try_get::<Option<String>, _>("id")?));
    out.insert("updated_at".into(), timestamp(row, "updated_at")?);
    out.insert("created_at".into(), timestamp(row, "created_at")?);
    out.insert(
        "last_accessed_at".into(),
        timestamp(row, "last_accessed_at")?,
    );
    out.insert(
        "metadata".into(),
        json!(row.try_get::<Option<Value>, _>("metadata")?),
    );
    out.insert("archived_at".into(), timestamp(row, "archived_at")?);
    out.insert(
        "is_delete_marker".into(),
        json!(row.try_get::<Option<bool>, _>("is_delete_marker")?),
    );
    out.insert(
        "is_versioned".into(),
        json!(row.try_get::<Option<bool>, _>("is_versioned")?),
    );
    Ok(Value::Object(out))
}

/// `POST /object/list/:bucketName`: `storage.search` under the request's
/// RLS scope. A bucket that does not exist lists nothing, as upstream.
// megabase:unit storage:route:POST /storage/v1/object/list/{bucketName}
pub(super) async fn list_objects(
    State(state): State<StorageState>,
    Path(bucket_id): Path<String>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(options) = list_options(&body) else {
        return not_implemented(&method, &uri);
    };
    let Some(pool) = state.pool.as_ref() else {
        return not_implemented(&method, &uri);
    };
    let Some((claims, jwt)) = request_jwt(&state, &headers) else {
        return not_implemented(&method, &uri);
    };
    if !is_valid_bucket_name(&bucket_id) {
        return StorageError::invalid_bucket_name().into_response();
    }
    let args = options.search_args();
    let scope = Scope {
        actor: Actor::Request {
            claims: &claims,
            jwt,
        },
        method: &method,
        uri: &uri,
        headers: &headers,
        operation: OP_LIST,
    };
    let attempt = async {
        let mut tx = scope.begin(pool).await?;
        let rows = sqlx::query(SEARCH_OBJECTS_SQL)
            .bind(&args.prefix)
            .bind(&bucket_id)
            .bind(args.limit)
            .bind(args.levels)
            .bind(args.offset)
            .bind(&args.search)
            .bind(&args.sort_column)
            .bind(&args.sort_order)
            .bind(&args.noncurrent_versions)
            .bind(&args.delete_markers)
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        rows.iter()
            .map(list_entry_json)
            .collect::<Result<Vec<_>, _>>()
    };
    match attempt.await {
        Ok(entries) => fastify_json(StatusCode::OK, &Value::Array(entries)),
        Err(error) => pg_error(&error, StorageError::resource_already_exists),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn object_routes_without_a_backend_or_jwt_stay_501() {
        use tower::ServiceExt;

        const SECRET: &str = "super-secret-jwt-token-with-at-least-32-characters-long";
        let service = megabase_core::Hs256::new(SECRET)
            .unwrap()
            .sign(&json!({ "role": "service_role" }))
            .unwrap();
        let app = crate::router_with_state(StorageState::new(
            None,
            Some(megabase_core::Hs256::new(SECRET).unwrap()),
        ));
        for (method, path, bearer) in [
            (Method::POST, "/storage/v1/object/avatars/u1/a.png", true),
            (Method::PUT, "/storage/v1/object/avatars/u1/a.png", true),
            (Method::GET, "/storage/v1/object/avatars/u1/a.png", false),
            (Method::HEAD, "/storage/v1/object/avatars/u1/a.png", true),
            (
                Method::GET,
                "/storage/v1/object/authenticated/avatars/a.png",
                false,
            ),
            (
                Method::HEAD,
                "/storage/v1/object/public/avatars/a.png",
                false,
            ),
            (
                Method::GET,
                "/storage/v1/object/public/avatars/a.png",
                false,
            ),
            (
                Method::GET,
                "/storage/v1/object/sign/avatars/a.png?token=x",
                false,
            ),
            (Method::POST, "/storage/v1/object/sign/avatars/a.png", true),
            (Method::DELETE, "/storage/v1/object/avatars", true),
            (Method::DELETE, "/storage/v1/object/avatars/u1/a.png", true),
            (Method::POST, "/storage/v1/object/list/avatars", true),
            (Method::POST, "/storage/v1/object/sign/avatars", true),
            (
                Method::POST,
                "/storage/v1/object/upload/sign/avatars/a.png",
                true,
            ),
            (
                Method::PUT,
                "/storage/v1/object/upload/sign/avatars/a.png",
                true,
            ),
            (
                Method::HEAD,
                "/storage/v1/object/authenticated/avatars/a.png",
                true,
            ),
            (
                Method::HEAD,
                "/storage/v1/object/authenticated/avatars/a.png",
                false,
            ),
            (
                Method::GET,
                "/storage/v1/object/info/authenticated/avatars/a.png",
                true,
            ),
            (
                Method::GET,
                "/storage/v1/object/info/public/avatars/a.png",
                false,
            ),
            (Method::GET, "/storage/v1/object/info/avatars/a.png", false),
            (Method::HEAD, "/storage/v1/object/info/avatars/a.png", true),
        ] {
            let mut request = axum::http::Request::builder()
                .method(&method)
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json");
            if bearer {
                request = request.header(
                    header::AUTHORIZATION,
                    format!("Bearer {}", service.as_str()),
                );
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::from(r#"{"expiresIn":60}"#)).unwrap())
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::NOT_IMPLEMENTED,
                "{method} {path}"
            );
        }
    }

    #[test]
    fn object_files_stay_under_the_root() {
        let root = FsPath::new("/data");
        assert_eq!(
            object_file(root, "avatars", "u1/a.png", "v1"),
            Some(PathBuf::from("/data/avatars/u1/a.png/v1"))
        );
        for name in ["../x", "u1/../../x", "./a", "a//b", "a/", ""] {
            assert_eq!(object_file(root, "avatars", name, "v1"), None, "{name}");
        }
        assert_eq!(object_file(root, "..", "a", "v1"), None);
    }

    #[test]
    fn etag_is_the_quoted_md5() {
        assert_eq!(etag(b""), "\"d41d8cd98f00b204e9800998ecf8427e\"");
        assert_eq!(etag(b"hello"), "\"5d41402abc4b2a76b9719d911017c592\"");
    }

    #[test]
    fn http_dates_follow_to_utc_string() {
        let at = DateTime::from_timestamp(1_613_537_012, 0).unwrap();
        assert_eq!(http_date(at), "Wed, 17 Feb 2021 04:43:32 GMT");
    }

    #[test]
    fn empty_folder_placeholders_allow_no_bytes() {
        assert_eq!(max_file_size("a/.emptyFolderPlaceholder", None), 0);
        assert_eq!(max_file_size("a.png", None), 52_428_800);
        assert_eq!(max_file_size("a.png", Some(10)), 10);
        assert_eq!(max_file_size("a.png", Some(1 << 40)), 52_428_800);
    }

    #[test]
    fn uploads_read_a_raw_body_or_a_form_file() {
        let mut headers = HeaderMap::new();
        let raw = Bytes::from_static(b"bytes");
        let file = file_upload(&headers, &raw).unwrap();
        assert_eq!(
            (&file.body[..], &*file.mime_type, &*file.cache_control),
            (&b"bytes"[..], "application/octet-stream", "no-cache")
        );
        assert!(!file.multipart);

        headers.insert("x-metadata", HeaderValue::from_static("e30="));
        assert!(file_upload(&headers, &raw).is_none());

        // A form ignores `x-metadata` and `Cache-Control` headers.
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("max-age=1"));
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("multipart/form-data; boundary=b"),
        );
        let form = Bytes::from_static(
            b"--b\r\nContent-Disposition: form-data; name=\"cacheControl\"\r\n\r\n60\r\n\
              --b\r\nContent-Disposition: form-data; name=\"\"; filename=\"blob\"\r\n\
              Content-Type: image/png\r\n\r\nPNG\r\n--b--\r\n",
        );
        let file = file_upload(&headers, &form).unwrap();
        assert_eq!(
            (&file.body[..], &*file.mime_type, &*file.cache_control),
            (&b"PNG"[..], "image/png", "max-age=60")
        );
        assert!(file.multipart);
        assert!(file_upload(&headers, &raw).is_none());

        // A form type in other letter case is not stored as a raw body.
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("Multipart/Form-Data; boundary=b"),
        );
        assert!(file_upload(&headers, &form).is_none());
    }

    #[test]
    fn delete_bodies_need_plain_prefixes() {
        assert_eq!(
            delete_prefixes(br#"{"prefixes":["a.png","u1/b.png"]}"#),
            Some(vec!["a.png".to_string(), "u1/b.png".to_string()])
        );
        for body in [
            &br#"{"prefixes":[]}"#[..],
            br#"{"prefixes":[{"path":"a","versionId":"v"}]}"#,
            br#"{"prefixes":"a"}"#,
            br#"{}"#,
            b"not json",
        ] {
            assert_eq!(delete_prefixes(body), None);
        }
    }

    #[test]
    fn list_bodies_follow_the_search_schema() {
        let options = list_options(
            br#"{"prefix":"u1","limit":10,"offset":2,"search":"a",
                "sortBy":{"column":"name","order":"desc"},"extra":1}"#,
        )
        .unwrap();
        assert_eq!(
            options.search_args(),
            SearchArgs {
                prefix: "u1/".into(),
                limit: 10,
                levels: 2,
                offset: 2,
                search: "a".into(),
                sort_column: "name".into(),
                sort_order: "desc".into(),
                noncurrent_versions: "exclude".into(),
                delete_markers: "exclude".into(),
            }
        );
        let defaults = list_options(br#"{"prefix":""}"#).unwrap().search_args();
        assert_eq!(
            (
                defaults.prefix.as_str(),
                defaults.limit,
                defaults.levels,
                defaults.offset,
                defaults.sort_column.as_str(),
                defaults.sort_order.as_str()
            ),
            ("", 100, 1, 0, "name", "asc")
        );
        assert!(list_options(br#"{"prefix":"a","exactMatch":false}"#).is_some());
        for body in [
            &br#"{}"#[..],
            br#"{"prefix":null}"#,
            br#"{"prefix":"a","limit":0}"#,
            br#"{"prefix":"a","limit":"10"}"#,
            br#"{"prefix":"a","limit":4294967296}"#,
            br#"{"prefix":"a","offset":-1}"#,
            br#"{"prefix":"a","sortBy":{"order":"asc"}}"#,
            br#"{"prefix":"a","sortBy":{"column":"size"}}"#,
            br#"{"prefix":"a","sortBy":{"column":"name","order":"up"}}"#,
            br#"{"prefix":"a","search":null}"#,
            br#"{"prefix":"a","exactMatch":true}"#,
            br#"{"prefix":"a","deleteMarkers":"all"}"#,
            b"not json",
        ] {
            assert_eq!(
                list_options(body),
                None,
                "{}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn non_name_sorts_escape_like_patterns() {
        let args =
            list_options(br#"{"prefix":"a_b/","search":"50%\\","sortBy":{"column":"created_at"}}"#)
                .unwrap()
                .search_args();
        assert_eq!(args.prefix, r"a\_b/");
        assert_eq!(args.search, r"50\%\\");
        assert_eq!(args.levels, 2);
        let args = list_options(br#"{"prefix":"a_b","search":"%"}"#)
            .unwrap()
            .search_args();
        assert_eq!((args.prefix.as_str(), args.search.as_str()), ("a_b/", "%"));
    }

    #[test]
    fn download_options_are_absent_empty_or_simple_names() {
        let option = |query: &str| {
            let uri: Uri = format!("/x?{query}").parse().unwrap();
            match download_option(&uri) {
                None => "501".to_string(),
                Some(Download::No) => "no".to_string(),
                Some(Download::Yes(name)) => format!("yes:{name}"),
            }
        };
        assert_eq!(option(""), "no");
        assert_eq!(option("download="), "yes:");
        assert_eq!(option("download=cat.png"), "yes:cat.png");
        assert_eq!(option("download=a%20b"), "501");
        assert_eq!(option("versionId=v1"), "501");
        assert_eq!(option("download=a&download=b"), "501");
    }
}
