//! `POST /object/move` and `POST /object/copy`
//! (`specs/storage/object-transfer.md`).

// Ported from supabase/storage src/http/routes/object/moveObject.ts,
// src/http/routes/object/copyObject.ts, src/storage/object.ts,
// src/storage/uploader.ts, src/storage/database/pg.ts,
// src/storage/backend/file.ts and
// src/storage/events/objects/object-admin-delete.ts (Apache-2.0),
// pin v1.80.2.

use std::path::{Path as FsPath, PathBuf};

use axum::{
    body::Bytes,
    extract::{OriginalUri, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use sqlx::Row;

use super::{
    backend, etag, header_str, object_file, object_row_json, pg_error, remove_file, request_jwt,
    Actor, Scope, FIND_OBJECT_FOR_UPDATE_SQL, FIND_OBJECT_SQL,
};
use crate::{
    bucket::{json_object, validation_not_implemented},
    fastify_json, iso,
    limits::{is_uuid, is_valid_key},
    not_implemented, StorageError, StorageState,
};

/// `ROUTE_OPERATIONS.MOVE_OBJECT` and `COPY_OBJECT`.
const OP_MOVE: &str = "storage.object.move";
const OP_COPY: &str = "storage.object.copy";

/// `updateObject` in `moveObject`'s permission check: rename the current
/// version. `owner` and `owner_id` are left alone when the request has no
/// UUID `sub` / no `sub` (`buildUpdate` skips `undefined`).
const MOVE_PROBE_SQL: &str = "UPDATE storage.objects \
     SET name = $1, bucket_id = $2, owner = COALESCE($3::uuid, owner), \
     owner_id = COALESCE($4, owner_id), version = $5 \
     WHERE bucket_id = $6 AND name COLLATE \"C\" = $7 AND archived_at IS NULL \
     RETURNING id";

/// `updateObject` once the file is copied: the probe's columns plus the new
/// file's metadata and the source's `user_metadata`.
const MOVE_SQL: &str = "UPDATE storage.objects \
     SET name = $1, bucket_id = $2, owner = COALESCE($3::uuid, owner), \
     owner_id = COALESCE($4, owner_id), version = $5, metadata = $6, user_metadata = $7 \
     WHERE bucket_id = $8 AND name COLLATE \"C\" = $9 AND archived_at IS NULL \
     RETURNING id";

/// `findObject(bucket, name, 'id, version, user_metadata')` as the superuser.
const FIND_MOVE_SOURCE_SQL: &str = "SELECT version, user_metadata FROM storage.objects \
     WHERE name COLLATE \"C\" = $1 AND bucket_id = $2 AND archived_at IS NULL LIMIT 1";

/// `findObject(bucket, name, 'bucket_id,metadata,user_metadata,version')`.
const FIND_COPY_SOURCE_SQL: &str = "SELECT version, metadata, user_metadata FROM storage.objects \
     WHERE name COLLATE \"C\" = $1 AND bucket_id = $2 AND archived_at IS NULL LIMIT 1";

/// `createObject` in `canUpload`.
const COPY_PROBE_CREATE_SQL: &str = "INSERT INTO storage.objects \
     (name, owner, owner_id, bucket_id, metadata, user_metadata, version) \
     VALUES ($1, $2::uuid, $3, $4, $5, $6, $7) RETURNING id";

/// The columns `objectSchema` serializes.
const OBJECT_COLUMNS: &str = "name, bucket_id, owner::text AS owner, owner_id, version, \
     id::text AS id, updated_at, created_at, last_accessed_at, metadata, user_metadata, \
     archived_at, is_delete_marker, is_versioned";

/// `upsertObject`: on conflict, update `metadata`, `version`, and
/// `user_metadata`, `owner` and `owner_id` only when they are defined.
fn upsert_sql(set_user_metadata: bool) -> String {
    let user_metadata = if set_user_metadata {
        "user_metadata = EXCLUDED.user_metadata, "
    } else {
        ""
    };
    format!(
        "INSERT INTO storage.objects \
         (name, owner, owner_id, bucket_id, metadata, user_metadata, version) \
         VALUES ($1, $2::uuid, $3, $4, $5, $6, $7) \
         ON CONFLICT (bucket_id, name COLLATE \"C\") WHERE archived_at IS NULL \
         DO UPDATE SET metadata = EXCLUDED.metadata, {user_metadata}\
         version = EXCLUDED.version, \
         owner = COALESCE(EXCLUDED.owner, storage.objects.owner), \
         owner_id = COALESCE(EXCLUDED.owner_id, storage.objects.owner_id) \
         RETURNING {OBJECT_COLUMNS}"
    )
}

/// The body fields `moveObjectsBodySchema` and `copyRequestBodySchema` share.
#[derive(Debug, PartialEq)]
struct Transfer {
    bucket_id: String,
    source_key: String,
    /// `destinationBucket || bucketId`.
    destination_bucket: String,
    destination_key: String,
}

/// An optional string property; `Err` when present but not a string
/// (Ajv runs with `coerceTypes: false`).
fn optional_str<'a>(body: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, ()> {
    match body.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(()),
    }
}

impl Transfer {
    fn from_body(body: &Map<String, Value>) -> Result<Self, ()> {
        let required = |key| optional_str(body, key)?.ok_or(());
        let bucket_id = required("bucketId")?;
        let source_key = required("sourceKey")?;
        let destination_key = required("destinationKey")?;
        optional_str(body, "sourceVersionId")?;
        let destination_bucket = optional_str(body, "destinationBucket")?
            .filter(|bucket| !bucket.is_empty())
            .unwrap_or(bucket_id);
        Ok(Self {
            bucket_id: bucket_id.to_string(),
            source_key: source_key.to_string(),
            destination_bucket: destination_bucket.to_string(),
            destination_key: destination_key.to_string(),
        })
    }
}

/// The `metadata` overrides of `copyRequestBodySchema`.
#[derive(Debug, Default, PartialEq)]
struct FileMetadata {
    cache_control: Option<String>,
    content_encoding: Option<String>,
    mimetype: Option<String>,
}

/// `copyRequestBodySchema`'s extra fields: `metadata` and `copyMetadata`
/// (default `true`).
fn copy_options(body: &Map<String, Value>) -> Result<(FileMetadata, bool), ()> {
    let metadata = match body.get("metadata") {
        None => FileMetadata::default(),
        Some(Value::Object(metadata)) => {
            let field = |key| optional_str(metadata, key).map(|value| value.map(str::to_string));
            FileMetadata {
                cache_control: field("cacheControl")?,
                content_encoding: field("contentEncoding")?,
                mimetype: field("mimetype")?,
            }
        }
        Some(_) => return Err(()),
    };
    let copy_metadata = match body.get("copyMetadata") {
        None => true,
        Some(Value::Bool(copy)) => *copy,
        Some(_) => return Err(()),
    };
    Ok((metadata, copy_metadata))
}

/// The JSON body and its shared fields; `Err` for a body Fastify's parser or
/// schema would reject.
fn transfer_body(headers: &HeaderMap, body: &[u8]) -> Result<(Map<String, Value>, Transfer), ()> {
    let body = json_object(headers, body).ok_or(())?;
    let transfer = Transfer::from_body(&body)?;
    Ok((body, transfer))
}

/// What a file copy reports: `copyObject` plus `headObject` on the file
/// backend (default `md5` ETags).
struct CopiedFile {
    e_tag: String,
    last_modified: DateTime<Utc>,
    size: usize,
}

/// `FileBackend.copyObject`: `ensureFile` the destination, then copy.
/// Megabase keeps no extended attributes; the file metadata lives in the row.
async fn copy_file(source: &FsPath, destination: &FsPath) -> std::io::Result<CopiedFile> {
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::copy(source, destination).await?;
    let bytes = tokio::fs::read(destination).await?;
    let modified = tokio::fs::metadata(destination).await?.modified()?;
    Ok(CopiedFile {
        e_tag: etag(&bytes),
        last_modified: DateTime::<Utc>::from(modified),
        size: bytes.len(),
    })
}

/// A metadata string the source row carries.
fn metadata_str<'a>(metadata: &'a Value, key: &str) -> Option<&'a str> {
    metadata.get(key).and_then(Value::as_str)
}

/// `FileBackend.headObject` on the moved file. The cache control, encoding
/// and MIME type the file backend keeps in extended attributes come from
/// the source row, where Megabase records them on upload.
fn head_metadata(source: &Value, file: &CopiedFile) -> Value {
    let mut out = Map::new();
    out.insert("httpStatusCode".into(), json!(200));
    out.insert("size".into(), json!(file.size));
    out.insert(
        "cacheControl".into(),
        json!(metadata_str(source, "cacheControl").unwrap_or("no-cache")),
    );
    if let Some(encoding) = metadata_str(source, "contentEncoding") {
        out.insert("contentEncoding".into(), json!(encoding));
    }
    out.insert(
        "mimetype".into(),
        json!(metadata_str(source, "mimetype").unwrap_or("application/octet-stream")),
    );
    out.insert("eTag".into(), json!(file.e_tag));
    out.insert("lastModified".into(), json!(iso(Some(file.last_modified))));
    out.insert("contentLength".into(), json!(file.size));
    Value::Object(out)
}

/// `copyObject`'s destination metadata: the source's, with the requested
/// overrides when `copyMetadata` is `false` (`preserveUnspecifiedFileMetadata`
/// keeps the rest), then the copy's `lastModified` and `eTag`.
fn copy_metadata_json(
    source: Option<&Value>,
    copy_metadata: bool,
    overrides: &FileMetadata,
    file: &CopiedFile,
) -> Value {
    let mut out = source
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if !copy_metadata {
        if let Some(cache_control) = &overrides.cache_control {
            out.insert("cacheControl".into(), json!(cache_control));
        }
        if let Some(mimetype) = &overrides.mimetype {
            out.insert("mimetype".into(), json!(mimetype));
        }
    }
    out.insert("lastModified".into(), json!(iso(Some(file.last_modified))));
    out.insert("eTag".into(), json!(file.e_tag));
    Value::Object(out)
}

/// The file of a row's version, or `None` when Megabase could not have
/// stored it (no version, or a path outside the root).
fn version_file(root: &FsPath, bucket: &str, name: &str, version: Option<&str>) -> Option<PathBuf> {
    object_file(root, bucket, name, version?)
}

/// `POST /object/move`: check the rename under the request's RLS (rolled
/// back), copy the file to a new version, rename the row as the superuser,
/// then remove the old file.
// megabase:unit storage:route:POST /storage/v1/object/move
pub(crate) async fn move_object(
    State(state): State<StorageState>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some((pool, root)) = backend(&state) else {
        return not_implemented(&method, &uri);
    };
    // storage-api's `AccessDenied` messages for a missing or invalid JWT are
    // not ported.
    let Some((claims, jwt)) = request_jwt(&state, &headers) else {
        return not_implemented(&method, &uri);
    };
    let Ok((body, input)) = transfer_body(&headers, &body) else {
        return validation_not_implemented();
    };
    // Object versions are not ported.
    if body.contains_key("sourceVersionId") {
        return not_implemented(&method, &uri);
    }
    if !is_valid_key(&input.destination_key) {
        return StorageError::invalid_key(&input.destination_key).into_response();
    }
    let new_version = uuid::Uuid::new_v4().to_string();
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
        operation: OP_MOVE,
    };

    // `testPermission`: find and rename the source as the request role.
    let probe = async {
        let mut tx = scope.begin(pool).await?;
        let found = sqlx::query(FIND_OBJECT_SQL)
            .bind(&input.source_key)
            .bind(&input.bucket_id)
            .fetch_optional(&mut *tx)
            .await?;
        if found.is_none() {
            return Ok(false);
        }
        let updated = sqlx::query(MOVE_PROBE_SQL)
            .bind(&input.destination_key)
            .bind(&input.destination_bucket)
            .bind(owner)
            .bind(owner_id)
            .bind(&new_version)
            .bind(&input.bucket_id)
            .bind(&input.source_key)
            .fetch_optional(&mut *tx)
            .await?;
        tx.rollback().await?;
        Ok::<_, sqlx::Error>(updated.is_some())
    };
    match probe.await {
        Ok(true) => {}
        Ok(false) => return StorageError::no_such_key().into_response(),
        Err(error) => return pg_error(&error, StorageError::resource_already_exists),
    }

    let source = async {
        let mut tx = scope.as_super_user().begin(pool).await?;
        let row = sqlx::query(FIND_MOVE_SOURCE_SQL)
            .bind(&input.source_key)
            .bind(&input.bucket_id)
            .fetch_optional(&mut *tx)
            .await?;
        tx.commit().await?;
        row.map(|row| {
            Ok::<_, sqlx::Error>((
                row.try_get::<Option<String>, _>("version")?,
                row.try_get::<Option<Value>, _>("user_metadata")?,
            ))
        })
        .transpose()
    };
    let (source_version, user_metadata) = match source.await {
        Ok(Some(source)) => source,
        Ok(None) => return StorageError::no_such_key().into_response(),
        Err(error) => return pg_error(&error, StorageError::resource_already_exists),
    };
    let moved = || fastify_json(StatusCode::OK, &json!({ "message": "Successfully moved" }));
    if input.bucket_id == input.destination_bucket && input.source_key == input.destination_key {
        return moved();
    }

    let (Some(source_file), Some(destination_file)) = (
        version_file(
            root,
            &input.bucket_id,
            &input.source_key,
            source_version.as_deref(),
        ),
        object_file(
            root,
            &input.destination_bucket,
            &input.destination_key,
            &new_version,
        ),
    ) else {
        return not_implemented(&method, &uri);
    };
    let copied = match copy_file(&source_file, &destination_file).await {
        Ok(copied) => copied,
        Err(error) => {
            tracing::error!(%error, "storage object copy failed");
            remove_file(&destination_file).await;
            return not_implemented(&method, &uri);
        }
    };

    let renamed = async {
        let mut tx = scope.as_super_user().begin(pool).await?;
        let Some(current) = sqlx::query(FIND_OBJECT_FOR_UPDATE_SQL)
            .bind(&input.source_key)
            .bind(&input.bucket_id)
            .fetch_optional(&mut *tx)
            .await?
        else {
            return Ok(false);
        };
        // The row was replaced since the file was copied: the copy is of
        // another version, so do not rename over it.
        let current_version: Option<String> = current.try_get("version")?;
        if current_version != source_version {
            return Ok(false);
        }
        let metadata: Option<Value> = current.try_get("metadata")?;
        let head = head_metadata(metadata.as_ref().unwrap_or(&Value::Null), &copied);
        // Hardening over upstream, which renames as the superuser: run the
        // rename under the request's RLS while the row lock is held, so a
        // change after the permission probe cannot widen what the caller
        // may rename (as `delete_object` does).
        scope.apply(&mut tx).await?;
        let updated = sqlx::query(MOVE_SQL)
            .bind(&input.destination_key)
            .bind(&input.destination_bucket)
            .bind(owner)
            .bind(owner_id)
            .bind(&new_version)
            .bind(&head)
            .bind(&user_metadata)
            .bind(&input.bucket_id)
            .bind(&input.source_key)
            .fetch_optional(&mut *tx)
            .await?;
        if updated.is_none() {
            return Ok(false);
        }
        tx.commit().await?;
        Ok::<_, sqlx::Error>(true)
    };
    match renamed.await {
        Ok(true) => {
            // `ObjectAdminDelete` of the source version, run inline.
            remove_file(&source_file).await;
            moved()
        }
        Ok(false) => {
            remove_file(&destination_file).await;
            StorageError::no_such_key().into_response()
        }
        Err(error) => {
            remove_file(&destination_file).await;
            pg_error(&error, StorageError::resource_already_exists)
        }
    }
}

/// `POST /object/copy`: read the source under the request's RLS, check the
/// destination insert (`canUpload`), copy the file to a new version, upsert
/// the destination row as the superuser, then remove a replaced file.
// megabase:unit storage:route:POST /storage/v1/object/copy
pub(crate) async fn copy_object(
    State(state): State<StorageState>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some((pool, root)) = backend(&state) else {
        return not_implemented(&method, &uri);
    };
    let Some((claims, jwt)) = request_jwt(&state, &headers) else {
        return not_implemented(&method, &uri);
    };
    let Ok((body, input)) = transfer_body(&headers, &body) else {
        return validation_not_implemented();
    };
    let Ok((overrides, copy_metadata)) = copy_options(&body) else {
        return validation_not_implemented();
    };
    // Object versions, `x-metadata` (`parseUserMetadata`) and
    // `validateContentEncoding` are not ported.
    if body.contains_key("sourceVersionId")
        || headers.contains_key("x-metadata")
        || (!copy_metadata && overrides.content_encoding.is_some())
    {
        return not_implemented(&method, &uri);
    }
    if !is_valid_key(&input.destination_key) {
        return StorageError::invalid_key(&input.destination_key).into_response();
    }
    let upsert = header_str(&headers, "x-upsert") == Some("true");
    let new_version = uuid::Uuid::new_v4().to_string();
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
        operation: OP_COPY,
    };

    let origin = async {
        let mut tx = scope.begin(pool).await?;
        let row = sqlx::query(FIND_COPY_SOURCE_SQL)
            .bind(&input.source_key)
            .bind(&input.bucket_id)
            .fetch_optional(&mut *tx)
            .await?;
        tx.commit().await?;
        row.map(|row| {
            Ok::<_, sqlx::Error>((
                row.try_get::<Option<String>, _>("version")?,
                row.try_get::<Option<Value>, _>("metadata")?,
                row.try_get::<Option<Value>, _>("user_metadata")?,
            ))
        })
        .transpose()
    };
    let (source_version, source_metadata, source_user_metadata) = match origin.await {
        Ok(Some(origin)) => origin,
        Ok(None) => return StorageError::no_such_key().into_response(),
        Err(error) => return pg_error(&error, StorageError::resource_already_exists),
    };
    // `copyMetadata ? originObject.user_metadata : userMetadata`; without
    // `x-metadata` the latter is `undefined`.
    let user_metadata = copy_metadata.then(|| source_user_metadata.unwrap_or(Value::Null));
    let probe_metadata = {
        let mut metadata = source_metadata
            .as_ref()
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if !copy_metadata {
            if let Some(cache_control) = &overrides.cache_control {
                metadata.insert("cacheControl".into(), json!(cache_control));
            }
            if let Some(mimetype) = &overrides.mimetype {
                metadata.insert("mimetype".into(), json!(mimetype));
            }
        }
        Value::Object(metadata)
    };
    let upsert_sql = upsert_sql(user_metadata.is_some());

    // `canUpload`: the destination insert under the request's RLS, rolled back.
    let probe = async {
        let mut tx = scope.begin(pool).await?;
        let sql = if upsert {
            upsert_sql.as_str()
        } else {
            COPY_PROBE_CREATE_SQL
        };
        sqlx::query(sql)
            .bind(&input.destination_key)
            .bind(owner)
            .bind(owner_id)
            .bind(&input.destination_bucket)
            .bind(&probe_metadata)
            .bind(&user_metadata)
            .bind("1")
            .execute(&mut *tx)
            .await?;
        tx.rollback().await
    };
    if let Err(error) = probe.await {
        return pg_error(&error, StorageError::key_already_exists);
    }

    let (Some(source_file), Some(destination_file)) = (
        version_file(
            root,
            &input.bucket_id,
            &input.source_key,
            source_version.as_deref(),
        ),
        object_file(
            root,
            &input.destination_bucket,
            &input.destination_key,
            &new_version,
        ),
    ) else {
        return not_implemented(&method, &uri);
    };
    let copied = match copy_file(&source_file, &destination_file).await {
        Ok(copied) => copied,
        Err(error) => {
            tracing::error!(%error, "storage object copy failed");
            remove_file(&destination_file).await;
            return not_implemented(&method, &uri);
        }
    };
    let metadata = copy_metadata_json(source_metadata.as_ref(), copy_metadata, &overrides, &copied);

    let written = async {
        let mut tx = scope.as_super_user().begin(pool).await?;
        let existing = sqlx::query(FIND_OBJECT_FOR_UPDATE_SQL)
            .bind(&input.destination_key)
            .bind(&input.destination_bucket)
            .fetch_optional(&mut *tx)
            .await?;
        let replaced: Option<String> = existing
            .map(|row| row.try_get::<Option<String>, _>("version"))
            .transpose()?
            .flatten();
        let row = sqlx::query(&upsert_sql)
            .bind(&input.destination_key)
            .bind(owner)
            .bind(owner_id)
            .bind(&input.destination_bucket)
            .bind(&metadata)
            .bind(&user_metadata)
            .bind(&new_version)
            .fetch_one(&mut *tx)
            .await?;
        let mut body = Map::new();
        body.insert("Id".into(), json!(row.try_get::<Option<String>, _>("id")?));
        body.insert(
            "Key".into(),
            json!(format!(
                "{}/{}",
                input.destination_bucket, input.destination_key
            )),
        );
        if let Value::Object(object) = object_row_json(&row)? {
            body.extend(object);
        }
        tx.commit().await?;
        Ok::<_, sqlx::Error>((Value::Object(body), replaced))
    };
    match written.await {
        Ok((body, replaced)) => {
            // `ObjectAdminDelete` of the replaced version, run inline.
            if let Some(file) = version_file(
                root,
                &input.destination_bucket,
                &input.destination_key,
                replaced.as_deref(),
            ) {
                remove_file(&file).await;
            }
            fastify_json(StatusCode::OK, &body)
        }
        Err(error) => {
            remove_file(&destination_file).await;
            pg_error(&error, StorageError::resource_already_exists)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(object) => object,
            _ => unreachable!("test bodies are objects"),
        }
    }

    #[test]
    fn transfer_bodies_follow_the_schema() {
        let input = Transfer::from_body(&object(json!({
            "bucketId": "b1",
            "sourceKey": "t/a.txt",
            "destinationKey": "t/b.txt",
            "destinationBucket": "",
        })))
        .unwrap();
        assert_eq!(
            input,
            Transfer {
                bucket_id: "b1".into(),
                source_key: "t/a.txt".into(),
                destination_bucket: "b1".into(),
                destination_key: "t/b.txt".into(),
            }
        );
        let other = Transfer::from_body(&object(json!({
            "bucketId": "b1",
            "sourceKey": "a",
            "destinationKey": "b",
            "destinationBucket": "b2",
        })))
        .unwrap();
        assert_eq!(other.destination_bucket, "b2");
        for bad in [
            json!({}),
            json!({ "bucketId": "b1", "sourceKey": "a" }),
            json!({ "bucketId": 1, "sourceKey": "a", "destinationKey": "b" }),
            json!({ "bucketId": "b1", "sourceKey": "a", "destinationKey": "b", "destinationBucket": null }),
            json!({ "bucketId": "b1", "sourceKey": "a", "destinationKey": "b", "sourceVersionId": 1 }),
        ] {
            assert!(Transfer::from_body(&object(bad.clone())).is_err(), "{bad}");
        }
    }

    #[test]
    fn copy_options_default_to_copying_metadata() {
        assert_eq!(
            copy_options(&object(json!({}))),
            Ok((FileMetadata::default(), true))
        );
        assert_eq!(
            copy_options(&object(json!({
                "copyMetadata": false,
                "metadata": { "mimetype": "text/plain", "other": 1 },
            }))),
            Ok((
                FileMetadata {
                    mimetype: Some("text/plain".into()),
                    ..FileMetadata::default()
                },
                false
            ))
        );
        for bad in [
            json!({ "copyMetadata": "false" }),
            json!({ "metadata": [] }),
            json!({ "metadata": { "cacheControl": 60 } }),
        ] {
            assert!(copy_options(&object(bad.clone())).is_err(), "{bad}");
        }
    }

    fn copied() -> CopiedFile {
        CopiedFile {
            e_tag: "\"abc\"".into(),
            last_modified: DateTime::from_timestamp(1_613_537_012, 770_000_000).unwrap(),
            size: 3,
        }
    }

    #[test]
    fn moved_files_report_head_object_metadata() {
        assert_eq!(
            head_metadata(
                &json!({ "mimetype": "text/plain", "cacheControl": "max-age=3600" }),
                &copied()
            ),
            json!({
                "httpStatusCode": 200,
                "size": 3,
                "cacheControl": "max-age=3600",
                "mimetype": "text/plain",
                "eTag": "\"abc\"",
                "lastModified": "2021-02-17T04:43:32.770Z",
                "contentLength": 3,
            })
        );
        let defaults = head_metadata(&Value::Null, &copied());
        assert_eq!(defaults["cacheControl"], "no-cache");
        assert_eq!(defaults["mimetype"], "application/octet-stream");
    }

    #[test]
    fn copies_keep_or_override_the_source_metadata() {
        let source = json!({ "mimetype": "text/plain", "cacheControl": "no-cache", "size": 3 });
        let overrides = FileMetadata {
            mimetype: Some("image/png".into()),
            ..FileMetadata::default()
        };
        let kept = copy_metadata_json(Some(&source), true, &overrides, &copied());
        assert_eq!(kept["mimetype"], "text/plain");
        assert_eq!(kept["size"], 3);
        assert_eq!(kept["eTag"], "\"abc\"");
        assert_eq!(kept["lastModified"], "2021-02-17T04:43:32.770Z");
        let replaced = copy_metadata_json(Some(&source), false, &overrides, &copied());
        assert_eq!(replaced["mimetype"], "image/png");
        assert_eq!(replaced["cacheControl"], "no-cache");
    }

    #[test]
    fn upserts_only_set_defined_user_metadata() {
        assert!(upsert_sql(true).contains("user_metadata = EXCLUDED.user_metadata"));
        assert!(!upsert_sql(false).contains("user_metadata = EXCLUDED"));
    }

    /// Requests that are answered before any query runs, against a pool
    /// nothing listens on.
    #[tokio::test]
    async fn bodies_and_destination_keys_are_checked_first() {
        use axum::body::{to_bytes, Body};
        use tower::ServiceExt;

        const SECRET: &str = "super-secret-jwt-token-with-at-least-32-characters-long";
        let jwt = megabase_core::Hs256::new(SECRET).unwrap();
        let bearer = format!(
            "Bearer {}",
            jwt.sign(&json!({ "role": "authenticated", "sub": "u1" }))
                .unwrap()
                .as_str()
        );
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(500))
            .connect_lazy("postgres://postgres@127.0.0.1:1/postgres")
            .unwrap();
        let app = crate::router_with_state(
            StorageState::new(Some(pool), Some(jwt))
                .with_file_root(Some(FsPath::new("/megabase-test-root"))),
        );
        for route in ["move", "copy"] {
            for (body, status) in [
                (
                    r#"{"bucketId":"b1","sourceKey":"a","destinationKey":"a#b"}"#,
                    StatusCode::BAD_REQUEST,
                ),
                (
                    r#"{"bucketId":"b1","sourceKey":"a"}"#,
                    StatusCode::NOT_IMPLEMENTED,
                ),
                (
                    r#"{"bucketId":"b1","sourceKey":"a","destinationKey":"b","sourceVersionId":"v"}"#,
                    StatusCode::NOT_IMPLEMENTED,
                ),
                ("", StatusCode::NOT_IMPLEMENTED),
            ] {
                let request = axum::http::Request::post(format!("/storage/v1/object/{route}"))
                    .header(axum::http::header::AUTHORIZATION, &bearer)
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap();
                let response = app.clone().oneshot(request).await.unwrap();
                assert_eq!(response.status(), status, "{route} {body}");
                let body: Value = serde_json::from_slice(
                    &to_bytes(response.into_body(), usize::MAX).await.unwrap(),
                )
                .unwrap();
                if status == StatusCode::BAD_REQUEST {
                    assert_eq!(
                        body,
                        json!({
                            "statusCode": "400",
                            "code": "InvalidKey",
                            "error": "InvalidKey",
                            "message": "Invalid key: a#b",
                        })
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn copied_files_get_an_md5_etag() {
        let dir = std::env::temp_dir().join(format!("megabase-copy-{}", uuid::Uuid::new_v4()));
        let source = dir.join("b1/a.txt/v1");
        tokio::fs::create_dir_all(source.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&source, b"abc").await.unwrap();
        let destination = dir.join("b2/t/b.txt/v2");
        let copied = copy_file(&source, &destination).await.unwrap();
        assert_eq!(copied.size, 3);
        assert_eq!(copied.e_tag, "\"900150983cd24fb0d6963f7d28e17f72\"");
        assert_eq!(tokio::fs::read(&destination).await.unwrap(), b"abc");
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
