//! `POST /bucket`, `PUT /bucket/:bucketId` and the bucket lifecycle routes.

// Ported from supabase/storage src/http/routes/bucket/createBucket.ts,
// src/http/routes/bucket/updateBucket.ts, src/http/routes/bucket/lifecycle.ts,
// src/storage/storage.ts, src/storage/database/pg.ts,
// src/storage/database/errors.ts, src/storage/lifecycle/guards.ts,
// src/storage/schemas/lifecycle.ts and src/internal/errors/codes.ts
// (Apache-2.0), pin v1.80.2.

use axum::{
    body::Bytes,
    extract::{OriginalUri, Path, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use megabase_core::MegabaseNotImplemented;
use serde_json::{json, Map, Value};

use crate::{
    database_error, fastify_json,
    limits::{
        is_creatable_bucket_name, is_uuid, is_valid_bucket_name, parse_file_size_to_bytes,
        UPLOAD_FILE_SIZE_LIMIT,
    },
    mime::normalize_allowed_mime_types,
    not_implemented, service_role, StorageError, StorageState, COMPONENT,
};

/// `LIFECYCLE_MAX_RULES`.
const LIFECYCLE_MAX_RULES: usize = 1000;

/// A body Fastify's schema validation would reject. Its error envelope is
/// not ported, so the request is answered with a 501.
fn validation_not_implemented() -> Response {
    MegabaseNotImplemented::new(COMPONENT, "storage request body validation").into_response()
}

/// The JSON object body Fastify's `application/json` parser accepts.
/// `None` for another content type, an empty or malformed body, or a
/// non-object (content-type parser and schema errors are not ported).
fn json_object(headers: &HeaderMap, body: &[u8]) -> Option<Map<String, Value>> {
    let content_type = headers.get(header::CONTENT_TYPE)?.to_str().ok()?;
    let essence = content_type.split(';').next().unwrap_or_default().trim();
    if !essence.eq_ignore_ascii_case("application/json") {
        return None;
    }
    match serde_json::from_slice(body).ok()? {
        Value::Object(object) => Some(object),
        _ => None,
    }
}

/// An Ajv `integer`: a finite number without a fractional part.
fn ajv_integer(value: &Value) -> Option<f64> {
    value.as_f64().filter(|n| n.is_finite() && n.fract() == 0.0)
}

/// `fileSizeLimitSchema` after validation.
#[derive(Clone, Copy)]
enum FileSizeLimit {
    Null,
    Bytes(f64),
}

/// A request field: absent, or present with a validated value.
type Field<T> = Option<T>;

/// `fileSizeLimitSchema`: a non-negative integer, a size string, or `null`.
/// `Err` when Ajv would reject the value (including coercible strings).
fn file_size_limit(value: Option<&Value>) -> Result<Field<FileSizeLimit>, ()> {
    match value {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(FileSizeLimit::Null)),
        Some(Value::String(size)) => parse_file_size_to_bytes(size)
            .map(|bytes| Some(FileSizeLimit::Bytes(bytes)))
            .ok_or(()),
        Some(number) => match ajv_integer(number) {
            Some(bytes) if bytes >= 0.0 => Ok(Some(FileSizeLimit::Bytes(bytes))),
            _ => Err(()),
        },
    }
}

/// `allowed_mime_types`: an array of strings or `null`. The route drops
/// empty strings before the service normalizes the rest.
fn allowed_mime_types(value: Option<&Value>) -> Result<Field<Option<Vec<String>>>, ()> {
    match value {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().ok_or(()))
            .filter(|item| !matches!(item, Ok("")))
            .map(|item| item.map(str::to_string))
            .collect::<Result<Vec<_>, ()>>()
            .map(|items| Some(Some(items))),
        Some(_) => Err(()),
    }
}

/// `parseMaxSizeLimit` against the single-tenant `FILE_SIZE_LIMIT`.
fn checked_file_size_limit(
    limit: Field<FileSizeLimit>,
) -> Result<Field<Option<i64>>, StorageError> {
    match limit {
        None => Ok(None),
        Some(FileSizeLimit::Null) => Ok(Some(None)),
        Some(FileSizeLimit::Bytes(bytes)) if bytes > UPLOAD_FILE_SIZE_LIMIT => {
            Err(StorageError::entity_too_large())
        }
        // At most `UPLOAD_FILE_SIZE_LIMIT`, so the integer cast is exact.
        #[allow(clippy::cast_possible_truncation)]
        Some(FileSizeLimit::Bytes(bytes)) => Ok(Some(Some(bytes as i64))),
    }
}

/// `normalizeAllowedMimeTypes` when the list is present and not `null`.
fn checked_mime_types(
    mime_types: Field<Option<Vec<String>>>,
) -> Result<Field<Option<Vec<String>>>, StorageError> {
    match mime_types {
        Some(Some(list)) => normalize_allowed_mime_types(&list)
            .map(|normalized| Some(Some(normalized)))
            .map_err(StorageError::invalid_mime_type),
        other => Ok(other),
    }
}

/// `createBucketBodySchema` after validation.
struct CreateBucket {
    name: String,
    id: Option<String>,
    public: bool,
    analytics: bool,
    file_size_limit: Field<FileSizeLimit>,
    allowed_mime_types: Field<Option<Vec<String>>>,
}

impl CreateBucket {
    fn from_body(body: &Map<String, Value>) -> Result<Self, ()> {
        if body.contains_key("lifecycle_configuration") {
            return Err(());
        }
        let name = body.get("name").and_then(Value::as_str).ok_or(())?;
        let id = match body.get("id") {
            None => None,
            Some(Value::String(id)) => Some(id.as_str()),
            Some(_) => return Err(()),
        };
        let public = match body.get("public") {
            None => false,
            Some(Value::Bool(public)) => *public,
            Some(_) => return Err(()),
        };
        let analytics = match body.get("type") {
            None => false,
            Some(Value::String(kind)) if kind == "STANDARD" => false,
            Some(Value::String(kind)) if kind == "ANALYTICS" => true,
            Some(_) => return Err(()),
        };
        Ok(Self {
            name: name.to_string(),
            id: id.filter(|id| !id.is_empty()).map(str::to_string),
            public,
            analytics,
            file_size_limit: file_size_limit(body.get("file_size_limit"))?,
            allowed_mime_types: allowed_mime_types(body.get("allowed_mime_types"))?,
        })
    }
}

/// `POST /bucket`: validate, insert the row as the request role, answer
/// `{ name }`.
// megabase:unit storage:route:POST /storage/v1/bucket
pub(crate) async fn create_bucket(
    State(state): State<StorageState>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some((pool, claims)) = service_role(&state, &headers) else {
        return not_implemented(&method, &uri);
    };
    let Some(input) = json_object(&headers, &body)
        .as_ref()
        .and_then(|body| CreateBucket::from_body(body).ok())
    else {
        return validation_not_implemented();
    };
    if !is_creatable_bucket_name(&input.name) {
        return StorageError::invalid_bucket_name().into_response();
    }
    if input.analytics {
        // Iceberg catalog buckets (`createIcebergBucket`) are not ported.
        return MegabaseNotImplemented::new(COMPONENT, "storage analytics buckets").into_response();
    }
    let file_size_limit = match checked_file_size_limit(input.file_size_limit) {
        Ok(limit) => limit.flatten(),
        Err(error) => return error.into_response(),
    };
    let allowed_mime_types = match checked_mime_types(input.allowed_mime_types) {
        Ok(mime_types) => mime_types.flatten(),
        Err(error) => return error.into_response(),
    };
    let owner_id = claims.sub.as_ref().map(|sub| sub.as_str());
    let owner = owner_id.filter(|sub| is_uuid(sub));
    let id = input.id.as_deref().unwrap_or(&input.name);
    let result = async {
        let mut tx = pool.begin().await?;
        sqlx::query("SET LOCAL ROLE service_role")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO storage.buckets \
             (id, name, owner, owner_id, public, allowed_mime_types, file_size_limit, type) \
             VALUES ($1, $2, $3::uuid, $4, $5, $6, $7, 'STANDARD')",
        )
        .bind(id)
        .bind(&input.name)
        .bind(owner)
        .bind(owner_id)
        .bind(input.public)
        .bind(allowed_mime_types)
        .bind(file_size_limit)
        .execute(&mut *tx)
        .await?;
        tx.commit().await
    }
    .await;
    match result {
        Ok(()) => fastify_json(StatusCode::OK, &json!({ "name": input.name })),
        Err(error) if is_unique_violation(&error) => {
            StorageError::bucket_already_exists().into_response()
        }
        Err(error) => database_error(&error),
    }
}

/// `23505`, which `DBError.fromDBError` maps to `ResourceAlreadyExists`
/// and `createBucket` rethrows as `BucketAlreadyExists`.
fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .is_some_and(|code| code == "23505")
}

/// `updateBucketBodySchema` after validation.
struct UpdateBucket {
    public: Field<bool>,
    file_size_limit: Field<FileSizeLimit>,
    allowed_mime_types: Field<Option<Vec<String>>>,
}

impl UpdateBucket {
    fn from_body(body: &Map<String, Value>) -> Result<Self, ()> {
        if body.contains_key("lifecycle_configuration") {
            return Err(());
        }
        // `anyOf` requires at least one updatable field.
        if !["public", "file_size_limit", "allowed_mime_types"]
            .iter()
            .any(|field| body.contains_key(*field))
        {
            return Err(());
        }
        let public = match body.get("public") {
            None => None,
            Some(Value::Bool(public)) => Some(*public),
            Some(_) => return Err(()),
        };
        Ok(Self {
            public,
            file_size_limit: file_size_limit(body.get("file_size_limit"))?,
            allowed_mime_types: allowed_mime_types(body.get("allowed_mime_types"))?,
        })
    }
}

/// `PUT /bucket/:bucketId`: update the given columns, answer
/// `{"message":"Successfully updated"}`.
// megabase:unit storage:route:PUT /storage/v1/bucket/{bucketId}
pub(crate) async fn update_bucket(
    State(state): State<StorageState>,
    Path(bucket_id): Path<String>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some((pool, _claims)) = service_role(&state, &headers) else {
        return not_implemented(&method, &uri);
    };
    let Some(input) = json_object(&headers, &body)
        .as_ref()
        .and_then(|body| UpdateBucket::from_body(body).ok())
    else {
        return validation_not_implemented();
    };
    if !is_valid_bucket_name(&bucket_id) {
        return StorageError::invalid_bucket_name().into_response();
    }
    let file_size_limit = match checked_file_size_limit(input.file_size_limit) {
        Ok(limit) => limit,
        Err(error) => return error.into_response(),
    };
    let allowed_mime_types = match checked_mime_types(input.allowed_mime_types) {
        Ok(mime_types) => mime_types,
        Err(error) => return error.into_response(),
    };
    let mut query = sqlx::QueryBuilder::<sqlx::Postgres>::new("UPDATE storage.buckets SET ");
    let mut set = query.separated(", ");
    if let Some(public) = input.public {
        set.push("public = ").push_bind_unseparated(public);
    }
    if let Some(limit) = file_size_limit {
        set.push("file_size_limit = ").push_bind_unseparated(limit);
    }
    if let Some(mime_types) = allowed_mime_types {
        set.push("allowed_mime_types = ")
            .push_bind_unseparated(mime_types);
    }
    query.push(" WHERE id = ").push_bind(&bucket_id);
    let result = async {
        let mut tx = pool.begin().await?;
        sqlx::query("SET LOCAL ROLE service_role")
            .execute(&mut *tx)
            .await?;
        let updated = query.build().execute(&mut *tx).await?.rows_affected();
        tx.commit().await?;
        Ok::<_, sqlx::Error>(updated)
    }
    .await;
    match result {
        Ok(0) => StorageError::no_such_bucket().into_response(),
        Ok(_) => fastify_json(
            StatusCode::OK,
            &json!({ "message": "Successfully updated" }),
        ),
        Err(error) => database_error(&error),
    }
}

/// `assertLifecycleApiEnabled`: `STORAGE_LIFECYCLE_ENABLED` is unset in the
/// pinned self-hosted compose file, so the lifecycle API is disabled.
fn lifecycle_disabled() -> Response {
    StorageError::feature_not_enabled("object lifecycles").into_response()
}

/// `GET /bucket/:bucketId/lifecycle` with the lifecycle API disabled.
// megabase:unit storage:route:GET /storage/v1/bucket/{bucketId}/lifecycle
pub(crate) async fn get_bucket_lifecycle(
    State(state): State<StorageState>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    if service_role(&state, &headers).is_none() {
        return not_implemented(&method, &uri);
    }
    lifecycle_disabled()
}

/// `PUT /bucket/:bucketId/lifecycle` with the lifecycle API disabled: the
/// body schema is checked first, then `assertLifecycleWriteReady` fails.
// megabase:unit storage:route:PUT /storage/v1/bucket/{bucketId}/lifecycle
pub(crate) async fn put_bucket_lifecycle(
    State(state): State<StorageState>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if service_role(&state, &headers).is_none() {
        return not_implemented(&method, &uri);
    }
    match json_object(&headers, &body) {
        Some(body) if is_lifecycle_configuration(&body) => lifecycle_disabled(),
        _ => validation_not_implemented(),
    }
}

/// `bucketLifecycleConfigurationSchema` with Ajv `coerceTypes: false`.
fn is_lifecycle_configuration(body: &Map<String, Value>) -> bool {
    let Some(Value::Array(rules)) = body.get("rules") else {
        return false;
    };
    (1..=LIFECYCLE_MAX_RULES).contains(&rules.len()) && rules.iter().all(is_lifecycle_rule)
}

fn is_lifecycle_rule(rule: &Value) -> bool {
    let Value::Object(rule) = rule else {
        return false;
    };
    let id = rule.get("id").is_none_or(Value::is_string);
    let status = rule
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| status == "Enabled" || status == "Disabled");
    let filter = rule.get("filter").is_some_and(Value::is_object);
    let expiration = match rule.get("noncurrentVersionExpiration") {
        Some(Value::Object(expiration)) => {
            expiration
                .get("noncurrentDays")
                .and_then(ajv_integer)
                .is_some()
                && expiration
                    .get("newerNoncurrentVersions")
                    .is_none_or(|value| ajv_integer(value).is_some())
        }
        _ => false,
    };
    id && status && filter && expiration
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
    fn json_bodies_need_the_json_content_type() {
        let mut headers = HeaderMap::new();
        assert!(json_object(&headers, b"{}").is_none());
        headers.insert(
            header::CONTENT_TYPE,
            "Application/JSON; charset=utf-8".parse().unwrap(),
        );
        assert!(json_object(&headers, b"{}").is_some());
        assert!(json_object(&headers, b"[]").is_none());
        assert!(json_object(&headers, b"").is_none());
        headers.insert(header::CONTENT_TYPE, "text/plain".parse().unwrap());
        assert!(json_object(&headers, b"{}").is_none());
    }

    #[test]
    fn create_body_follows_the_schema() {
        let input = CreateBucket::from_body(&object(json!({
            "name": "avatars",
            "id": "",
            "public": true,
            "file_size_limit": "1MB",
            "allowed_mime_types": ["image/png", ""],
            "extra": 1,
        })))
        .unwrap();
        assert_eq!(input.name, "avatars");
        assert!(input.id.is_none());
        assert!(input.public);
        assert!(!input.analytics);
        assert!(matches!(
            input.file_size_limit,
            Some(FileSizeLimit::Bytes(b)) if b == 1e6
        ));
        assert_eq!(
            input.allowed_mime_types,
            Some(Some(vec!["image/png".to_string()]))
        );
        for bad in [
            json!({}),
            json!({ "name": 1 }),
            json!({ "name": "a", "id": null }),
            json!({ "name": "a", "public": "true" }),
            json!({ "name": "a", "type": "VECTOR" }),
            json!({ "name": "a", "file_size_limit": -1 }),
            json!({ "name": "a", "file_size_limit": 1.5 }),
            json!({ "name": "a", "file_size_limit": "100" }),
            json!({ "name": "a", "allowed_mime_types": "image/png" }),
            json!({ "name": "a", "allowed_mime_types": [1] }),
            json!({ "name": "a", "lifecycle_configuration": null }),
        ] {
            assert!(
                CreateBucket::from_body(&object(bad.clone())).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn update_body_needs_an_updatable_field() {
        assert!(UpdateBucket::from_body(&object(json!({ "name": "a" }))).is_err());
        assert!(UpdateBucket::from_body(&object(json!({ "public": null }))).is_err());
        let input = UpdateBucket::from_body(&object(json!({
            "file_size_limit": null,
            "allowed_mime_types": null,
        })))
        .unwrap();
        assert!(input.public.is_none());
        assert!(matches!(input.file_size_limit, Some(FileSizeLimit::Null)));
        assert_eq!(input.allowed_mime_types, Some(None));
    }

    #[test]
    fn size_and_mime_checks_render_storage_errors() {
        assert!(matches!(
            checked_file_size_limit(Some(FileSizeLimit::Bytes(52_428_800.0))),
            Ok(Some(Some(52_428_800)))
        ));
        assert_eq!(
            checked_file_size_limit(Some(FileSizeLimit::Bytes(52_428_801.0)))
                .err()
                .map(|e| e.body()),
            Some(json!({
                "statusCode": "413",
                "code": "EntityTooLarge",
                "error": "Payload too large",
                "message": "The object exceeded the maximum allowed size",
            }))
        );
        assert_eq!(
            checked_mime_types(Some(Some(vec!["nope".to_string()])))
                .err()
                .map(|e| e.body()),
            Some(json!({
                "statusCode": "415",
                "code": "InvalidMimeType",
                "error": "invalid_mime_type",
                "message": "mime type nope is not supported",
            }))
        );
        assert_eq!(
            checked_mime_types(Some(Some(vec![]))).ok(),
            Some(Some(Some(vec![])))
        );
    }

    #[test]
    fn lifecycle_schema_matches_bucket_lifecycle_configuration_schema() {
        let rule = json!({
            "id": "r1",
            "status": "Enabled",
            "filter": {},
            "noncurrentVersionExpiration": { "noncurrentDays": 30, "newerNoncurrentVersions": 2.0 },
        });
        assert!(is_lifecycle_configuration(&object(
            json!({ "rules": [rule] })
        )));
        for bad in [
            json!({}),
            json!({ "rules": [] }),
            json!({ "rules": [1] }),
            json!({ "rules": [{ "status": "Enabled", "filter": {} }] }),
            json!({ "rules": [{ "status": "On", "filter": {}, "noncurrentVersionExpiration": { "noncurrentDays": 1 } }] }),
            json!({ "rules": [{ "status": "Enabled", "filter": [], "noncurrentVersionExpiration": { "noncurrentDays": 1 } }] }),
            json!({ "rules": [{ "status": "Enabled", "filter": {}, "noncurrentVersionExpiration": { "noncurrentDays": "1" } }] }),
            json!({ "rules": [{ "id": 1, "status": "Enabled", "filter": {}, "noncurrentVersionExpiration": { "noncurrentDays": 1 } }] }),
        ] {
            assert!(!is_lifecycle_configuration(&object(bad.clone())), "{bad}");
        }
        let too_many = vec![rule; LIFECYCLE_MAX_RULES + 1];
        assert!(!is_lifecycle_configuration(&object(
            json!({ "rules": too_many })
        )));
    }
}
