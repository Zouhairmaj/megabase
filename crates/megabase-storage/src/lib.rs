//! Supabase Storage-compatible API (`/storage/v1`) for Megabase.
//!
//! Target behavior: Supabase Storage, vendor/storage (Apache-2.0), pinned in `vendor/`.
//! The bucket routes (list, get, create, update, and the lifecycle get and
//! put) are served for the service role (`specs/storage/buckets.md`,
//! `specs/storage/bucket.md`), and `GET /health` for any valid JWT
//! (`specs/storage/health.md`). A missing or invalid JWT, other roles on the
//! bucket routes, and every other route return the structured
//! `MEGABASE_NOT_IMPLEMENTED` 501. The `DELETE /cdn` purge routes answer as
//! storage-api does without a CDN purge endpoint (`specs/storage/cdn.md`),
//! and the `/iceberg` routes as it does without the Iceberg catalog
//! (`specs/storage/iceberg.md`). Selected `GET`, `HEAD`, `DELETE` and `POST`
//! `/s3` routes answer requests without an AWS Signature V4; signed requests
//! and the other `/s3` routes (such as service-level `HEAD /s3`) are a 501
//! (`specs/storage/s3.md`).
//! The `GET /render/image` routes answer their signature, key, bucket and
//! object errors; the transformation itself is a 501
//! (`specs/storage/render.md`). The `/object` upload, download, info, sign,
//! remove and list routes run under the request's RLS scope and keep bytes
//! under `FILE_STORAGE_BACKEND_PATH` (`specs/storage/object.md`,
//! `specs/storage/object-list.md`).

// Ported from supabase/storage src/http/routes/bucket/getAllBuckets.ts,
// src/http/routes/bucket/getBucket.ts, src/http/routes/health/healthcheck.ts,
// src/storage/database/pg.ts, src/storage/schemas/bucket.ts,
// src/http/error-handler.ts and src/internal/errors/codes.ts (Apache-2.0),
// pin v1.80.2.

use std::{borrow::Cow, sync::Arc};

use axum::{
    extract::{OriginalUri, Path, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, delete, get, post},
    Router,
};
use chrono::{DateTime, Utc};
use megabase_core::{bearer_token, Hs256, JwtClaims, MegabaseNotImplemented};
use serde_json::{json, Map, Value};

mod bucket;
mod cdn;
mod iceberg;
mod limits;
mod mime;
mod multipart;
mod object;
mod render;
mod s3;
mod schema;

pub use schema::{install_schema, SchemaError};

pub const COMPONENT: &str = "storage";

/// Columns `getAllBuckets.ts` selects (client supports `type`).
const LIST_COLUMNS: &str = "id, name, public, owner::text AS owner, created_at, updated_at, \
     file_size_limit, allowed_mime_types, type::text AS type";

/// Columns `getBucket.ts` selects.
const GET_COLUMNS: &str = "id, name, owner::text AS owner, public, created_at, updated_at, \
     file_size_limit, allowed_mime_types";

/// Database pool and JWT verifier for `/storage/v1`.
#[derive(Clone, Default)]
pub struct StorageState {
    pool: Option<sqlx::PgPool>,
    jwt: Option<Arc<Hs256>>,
    files: Option<Arc<std::path::Path>>,
}

impl StorageState {
    /// State with an optional pool and the `JWT_SECRET` verifier.
    #[must_use]
    pub fn new(pool: Option<sqlx::PgPool>, jwt: Option<Hs256>) -> Self {
        Self {
            pool,
            jwt: jwt.map(Arc::new),
            files: None,
        }
    }

    /// Keep object bytes under `root` (`FILE_STORAGE_BACKEND_PATH`). Without
    /// it the object routes answer 501.
    #[must_use]
    pub fn with_file_root(mut self, root: Option<&std::path::Path>) -> Self {
        self.files = root.map(Arc::from);
        self
    }
}

/// `/storage/v1` routes without a database or JWT secret.
pub fn router() -> Router {
    router_with_state(StorageState::default())
}

/// `/storage/v1` routes sharing `state`.
pub fn router_with_state(state: StorageState) -> Router {
    Router::new()
        .route(
            "/storage/v1/bucket",
            get(list_buckets).post(bucket::create_bucket),
        )
        .route(
            "/storage/v1/bucket/",
            get(list_buckets).post(bucket::create_bucket),
        )
        .route(
            "/storage/v1/bucket/:bucket_id",
            get(get_bucket).put(bucket::update_bucket),
        )
        .route(
            "/storage/v1/bucket/:bucket_id/lifecycle",
            get(bucket::get_bucket_lifecycle).put(bucket::put_bucket_lifecycle),
        )
        .route("/storage/v1/health", get(healthcheck))
        .route("/storage/v1/health/", get(healthcheck))
        .route("/storage/v1/iceberg", any(iceberg::catalog_disabled))
        .route("/storage/v1/iceberg/", any(iceberg::catalog_disabled))
        .route("/storage/v1/iceberg/*rest", any(iceberg::catalog_disabled))
        .route("/storage/v1/s3", any(s3::unsigned))
        .route("/storage/v1/s3/", any(s3::unsigned))
        .route("/storage/v1/s3/*rest", any(s3::unsigned))
        .route("/storage/v1/cdn", delete(cdn::purge_cache))
        .route("/storage/v1/cdn/", delete(cdn::purge_cache))
        .route("/storage/v1/cdn/:bucket_name", delete(cdn::purge_cache))
        .route(
            "/storage/v1/cdn/:bucket_name/*object_name",
            delete(cdn::purge_cache),
        )
        .route(
            "/storage/v1/render/image/authenticated/:bucket_name/*object_name",
            get(render::render_authenticated_image),
        )
        .route(
            "/storage/v1/render/image/public/:bucket_name/*object_name",
            get(render::render_public_image),
        )
        .route(
            "/storage/v1/render/image/sign/:bucket_name/*object_name",
            get(render::render_signed_image),
        )
        // `HEAD` is `getObjectInfo`; where it is not ported, axum would
        // answer it with the `GET` handler otherwise.
        .route(
            "/storage/v1/object/authenticated/:bucket_name/*object_name",
            get(object::get_authenticated_object).head(object::head_authenticated_object),
        )
        .route(
            "/storage/v1/object/public/:bucket_name/*object_name",
            get(object::get_public_object).head(not_implemented_head),
        )
        .route(
            "/storage/v1/object/sign/:bucket_name/*object_name",
            get(object::get_signed_object).post(object::sign_object),
        )
        .route(
            "/storage/v1/object/list/:bucket_name",
            post(object::list_objects),
        )
        .route(
            "/storage/v1/object/sign/:bucket_name",
            post(object::sign_objects),
        )
        .route(
            "/storage/v1/object/upload/sign/:bucket_name/*object_name",
            post(object::sign_upload_object),
        )
        // Unported upstream routes whose first segment the generic
        // `:bucket_name/*object_name` route would take for a bucket name.
        .route(
            "/storage/v1/object/list-v2/:bucket_name",
            post(not_implemented_any),
        )
        // Fastify's automatic `HEAD` for the info routes is not ported.
        .route(
            "/storage/v1/object/info/authenticated/:bucket_name/*object_name",
            get(object::get_object_info_authenticated).head(not_implemented_head),
        )
        .route(
            "/storage/v1/object/info/public/:bucket_name/*object_name",
            get(object::get_public_object_info).head(not_implemented_head),
        )
        .route(
            "/storage/v1/object/info/:bucket_name/*object_name",
            get(object::get_object_info_any).head(not_implemented_head),
        )
        .route(
            "/storage/v1/object/:bucket_name",
            delete(object::delete_objects),
        )
        .route(
            "/storage/v1/object/:bucket_name/*object_name",
            get(object::get_object_any)
                .head(not_implemented_head)
                .post(object::create_object)
                .put(object::update_object)
                .delete(object::delete_object),
        )
        .with_state(state)
        .merge(megabase_core::not_implemented_router(COMPONENT))
        .method_not_allowed_fallback(megabase_core::not_implemented_handler(COMPONENT))
}

#[derive(sqlx::FromRow)]
struct BucketRow {
    id: String,
    name: String,
    owner: Option<String>,
    public: Option<bool>,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
    file_size_limit: Option<i64>,
    allowed_mime_types: Option<Vec<String>>,
    #[sqlx(default)]
    r#type: Option<String>,
}

impl BucketRow {
    /// `bucketSchema` through fast-json-stringify: a `null` string is `""`,
    /// a `null` boolean is `false`, dates are `toISOString()`.
    fn to_json(&self, with_type: bool) -> Value {
        let mut out = Map::new();
        out.insert("id".into(), json!(self.id));
        out.insert("name".into(), json!(self.name));
        out.insert("owner".into(), json!(self.owner.as_deref().unwrap_or("")));
        out.insert("public".into(), json!(self.public.unwrap_or(false)));
        if with_type {
            out.insert("type".into(), json!(self.r#type.as_deref().unwrap_or("")));
        }
        out.insert("file_size_limit".into(), json!(self.file_size_limit));
        out.insert("allowed_mime_types".into(), json!(self.allowed_mime_types));
        out.insert("created_at".into(), json!(iso(self.created_at)));
        out.insert("updated_at".into(), json!(iso(self.updated_at)));
        Value::Object(out)
    }
}

fn iso(at: Option<DateTime<Utc>>) -> String {
    at.map(|at| at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
        .unwrap_or_default()
}

/// The claims of a valid bearer JWT; `None` when the header is missing or
/// the token does not verify.
fn verified_claims(state: &StorageState, headers: &HeaderMap) -> Option<JwtClaims> {
    let jwt = state.jwt.as_deref()?;
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_token)?;
    jwt.verify(token).ok()
}

/// Requests Megabase serves: a valid service-role JWT and a pool.
fn service_role<'a>(
    state: &'a StorageState,
    headers: &HeaderMap,
) -> Option<(&'a sqlx::PgPool, JwtClaims)> {
    let claims = verified_claims(state, headers)?;
    if claims.role.as_deref() != Some("service_role") {
        return None;
    }
    Some((state.pool.as_ref()?, claims))
}

/// Query `pg.ts` `healthcheck()` runs as the superuser.
const HEALTHCHECK_SQL: &str = "SELECT id from storage.buckets limit 1";

/// `GET /health`: any valid JWT; `{"healthy":true}` when the probe query
/// succeeds and `{"healthy":false}` (still 200) when it fails.
// megabase:unit storage:route:GET /storage/v1/health
async fn healthcheck(
    State(state): State<StorageState>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    // storage-api's `AccessDenied` body for a missing or invalid JWT is not ported.
    if verified_claims(&state, &headers).is_none() {
        return not_implemented(&method, &uri);
    }
    let Some(pool) = state.pool.as_ref() else {
        return not_implemented(&method, &uri);
    };
    let healthy = match sqlx::query(HEALTHCHECK_SQL).fetch_optional(pool).await {
        Ok(_) => true,
        Err(error) => {
            tracing::error!(%error, "storage healthcheck failed");
            false
        }
    };
    fastify_json(StatusCode::OK, &json!({ "healthy": healthy }))
}

async fn not_implemented_any(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    not_implemented(&method, &uri)
}

async fn not_implemented_head(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    not_implemented(&method, &uri)
}

fn not_implemented(method: &Method, uri: &axum::http::Uri) -> Response {
    MegabaseNotImplemented::new(COMPONENT, format!("{method} {}", uri.path())).into_response()
}

/// Run `sql` as the request role, as storage-api does for every query.
async fn fetch_as_service_role(
    pool: &sqlx::PgPool,
    sql: &str,
    bucket_id: Option<&str>,
) -> Result<Vec<BucketRow>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL ROLE service_role")
        .execute(&mut *tx)
        .await?;
    let mut query = sqlx::query_as::<_, BucketRow>(sql);
    if let Some(bucket_id) = bucket_id {
        query = query.bind(bucket_id);
    }
    let rows = query.fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(rows)
}

/// `GET /bucket` without query options: every row, in table order.
// megabase:unit storage:route:GET /storage/v1/bucket
async fn list_buckets(
    State(state): State<StorageState>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let Some((pool, _claims)) = service_role(&state, &headers) else {
        return not_implemented(&method, &uri);
    };
    // `limit`, `offset`, `sortColumn`, `sortOrder` and `search` are not ported.
    if uri.query().is_some_and(|query| !query.is_empty()) {
        return not_implemented(&method, &uri);
    }
    let sql = format!("SELECT {LIST_COLUMNS} FROM storage.buckets");
    match fetch_as_service_role(pool, &sql, None).await {
        Ok(rows) => fastify_json(
            StatusCode::OK,
            &Value::Array(rows.iter().map(|row| row.to_json(true)).collect()),
        ),
        Err(error) => database_error(&error),
    }
}

/// `GET /bucket/:bucketId`: the row, or `NoSuchBucket` rendered with
/// status 400 by the global error handler.
// megabase:unit storage:route:GET /storage/v1/bucket/{bucketId}
async fn get_bucket(
    State(state): State<StorageState>,
    Path(bucket_id): Path<String>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let Some((pool, _claims)) = service_role(&state, &headers) else {
        return not_implemented(&method, &uri);
    };
    let sql = format!("SELECT {GET_COLUMNS} FROM storage.buckets WHERE id = $1 LIMIT 1");
    match fetch_as_service_role(pool, &sql, Some(&bucket_id)).await {
        Ok(rows) => match rows.first() {
            Some(row) => fastify_json(StatusCode::OK, &row.to_json(false)),
            None => StorageError::no_such_bucket().into_response(),
        },
        Err(error) => database_error(&error),
    }
}

/// A failed query is outside the ported surface: log it and answer 501
/// rather than invent storage-api's database error mapping.
/// A `StorageBackendError` from `src/internal/errors/codes.ts`.
pub(crate) struct StorageError {
    status_code: &'static str,
    code: &'static str,
    error: &'static str,
    message: Cow<'static, str>,
}

impl StorageError {
    /// `ERRORS.NoSuchBucket`.
    fn no_such_bucket() -> Self {
        Self {
            status_code: "404",
            code: "NoSuchBucket",
            error: "Bucket not found",
            message: Cow::Borrowed("Bucket not found"),
        }
    }

    /// `ERRORS.NoSuchKey`.
    fn no_such_key() -> Self {
        Self {
            status_code: "404",
            code: "NoSuchKey",
            error: "not_found",
            message: Cow::Borrowed("Object not found"),
        }
    }

    /// `ERRORS.InvalidKey`. It sets no `error`, so the global error handler
    /// repeats the code there.
    fn invalid_key(key: &str) -> Self {
        Self {
            status_code: "400",
            code: "InvalidKey",
            error: "InvalidKey",
            message: Cow::Owned(format!("Invalid key: {key}")),
        }
    }

    /// `ERRORS.InvalidSignature`, with `Invalid signature` as the default
    /// message and no `error`.
    fn invalid_signature(message: Cow<'static, str>) -> Self {
        Self {
            status_code: "400",
            code: "InvalidSignature",
            error: "InvalidSignature",
            message,
        }
    }

    /// `ERRORS.AccessDenied(message)`.
    fn access_denied(message: Cow<'static, str>) -> Self {
        Self {
            status_code: "403",
            code: "AccessDenied",
            error: "Unauthorized",
            message,
        }
    }

    /// `ERRORS.KeyAlreadyExists`.
    fn key_already_exists() -> Self {
        Self {
            status_code: "409",
            code: "KeyAlreadyExists",
            error: "Duplicate",
            message: Cow::Borrowed("The resource already exists"),
        }
    }

    /// `ERRORS.ResourceAlreadyExists`.
    fn resource_already_exists() -> Self {
        Self {
            status_code: "409",
            code: "ResourceAlreadyExists",
            error: "Duplicate",
            message: Cow::Borrowed("The resource already exists"),
        }
    }

    /// `ERRORS.InvalidJWT` with jose's message. It sets no `error`, so the
    /// global error handler repeats the code there.
    fn invalid_jwt(message: &'static str) -> Self {
        Self {
            status_code: "400",
            code: "InvalidJWT",
            error: "InvalidJWT",
            message: Cow::Borrowed(message),
        }
    }

    /// `ERRORS.InvalidBucketName`.
    fn invalid_bucket_name() -> Self {
        Self {
            status_code: "400",
            code: "InvalidBucketName",
            error: "Invalid Input",
            message: Cow::Borrowed("Bucket name invalid"),
        }
    }

    /// `ERRORS.BucketAlreadyExists`.
    fn bucket_already_exists() -> Self {
        Self {
            status_code: "409",
            code: "BucketAlreadyExists",
            error: "Duplicate",
            message: Cow::Borrowed("The resource already exists"),
        }
    }

    /// `ERRORS.EntityTooLarge()` with its default entity and limit.
    fn entity_too_large() -> Self {
        Self {
            status_code: "413",
            code: "EntityTooLarge",
            error: "Payload too large",
            message: Cow::Borrowed("The object exceeded the maximum allowed size"),
        }
    }

    /// `ERRORS.InvalidMimeType`.
    fn invalid_mime_type(mime_type: &str) -> Self {
        Self {
            status_code: "415",
            code: "InvalidMimeType",
            error: "invalid_mime_type",
            message: Cow::Owned(format!("mime type {mime_type} is not supported")),
        }
    }

    /// `ERRORS.FeatureNotEnabled`. It sets no `error`, so the global error
    /// handler repeats the code there.
    fn feature_not_enabled(feature: &str) -> Self {
        Self {
            status_code: "409",
            code: "FeatureNotEnabled",
            error: "FeatureNotEnabled",
            message: Cow::Owned(format!(
                "The feature {feature} is not enabled for this resource"
            )),
        }
    }

    fn body(&self) -> Value {
        json!({
            "statusCode": self.status_code,
            "code": self.code,
            "error": self.error,
            "message": self.message,
        })
    }
}

impl IntoResponse for StorageError {
    /// The global error handler sends `userStatusCode`: 500 for a 500
    /// error, 400 for every other one.
    fn into_response(self) -> Response {
        let status = if self.status_code == "500" {
            StatusCode::INTERNAL_SERVER_ERROR
        } else {
            StatusCode::BAD_REQUEST
        };
        fastify_json(status, &self.body())
    }
}

fn database_error(error: &sqlx::Error) -> Response {
    tracing::error!(%error, "storage bucket query failed");
    MegabaseNotImplemented::new(COMPONENT, "storage database error mapping").into_response()
}

fn fastify_json(status: StatusCode, body: &Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        body.to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use tower::ServiceExt;

    use super::*;

    const SECRET: &str = "super-secret-jwt-token-with-at-least-32-characters-long";

    fn token(role: &str) -> String {
        Hs256::new(SECRET)
            .unwrap()
            .sign(&json!({ "role": role }))
            .unwrap()
            .as_str()
            .to_string()
    }

    async fn status(path: &str, bearer: Option<&str>) -> StatusCode {
        status_of(Method::GET, path, bearer).await
    }

    async fn status_of(method: Method, path: &str, bearer: Option<&str>) -> StatusCode {
        response_of(method, path, bearer).await.0
    }

    async fn response_of(
        method: Method,
        path: &str,
        bearer: Option<&str>,
    ) -> (StatusCode, Vec<u8>) {
        let app = router_with_state(StorageState::new(None, Some(Hs256::new(SECRET).unwrap())));
        let mut request = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(bearer) = bearer {
            request = request.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
        }
        let response = app
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, body.to_vec())
    }

    async fn assert_not_implemented(method: Method, path: &str, bearer: Option<&str>) {
        let (status, body) = response_of(method.clone(), path, bearer).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{method} {path}");
        // HEAD and OPTIONS responses may carry no body.
        if !body.is_empty() {
            let value: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(value["code"], "MEGABASE_NOT_IMPLEMENTED", "{method} {path}");
        }
    }

    #[tokio::test]
    async fn roles_other_than_service_role_stay_501() {
        for bearer in [Some(token("anon")), Some("bad".to_string()), None] {
            assert_eq!(
                status("/storage/v1/bucket", bearer.as_deref()).await,
                StatusCode::NOT_IMPLEMENTED
            );
            assert_eq!(
                status("/storage/v1/bucket/avatars", bearer.as_deref()).await,
                StatusCode::NOT_IMPLEMENTED
            );
            for (method, path) in [
                (Method::POST, "/storage/v1/bucket"),
                (Method::PUT, "/storage/v1/bucket/avatars"),
                (Method::GET, "/storage/v1/bucket/avatars/lifecycle"),
                (Method::PUT, "/storage/v1/bucket/avatars/lifecycle"),
            ] {
                assert_eq!(
                    status_of(method, path, bearer.as_deref()).await,
                    StatusCode::NOT_IMPLEMENTED,
                    "{path}"
                );
            }
        }
    }

    /// The TUS server these routes delegate to is not vendored
    /// (`specs/storage/resumable.md`), so no resumable answer is guessed.
    #[tokio::test]
    async fn resumable_upload_routes_stay_501() {
        let service = token("service_role");
        for bearer in [Some(service.as_str()), None] {
            for (method, path) in [
                (Method::POST, "/storage/v1/upload/resumable"),
                (Method::POST, "/storage/v1/upload/resumable/"),
                (Method::POST, "/storage/v1/upload/resumable/sign"),
                (Method::OPTIONS, "/storage/v1/upload/resumable"),
                (Method::OPTIONS, "/storage/v1/upload/resumable/sign"),
            ] {
                assert_not_implemented(method, path, bearer).await;
            }
            for prefix in [
                "/storage/v1/upload/resumable/",
                "/storage/v1/upload/resumable/sign/",
            ] {
                let path = format!("{prefix}YXZhdGFycy9hLnR4dC92MQ");
                for method in [
                    Method::POST,
                    Method::PUT,
                    Method::PATCH,
                    Method::HEAD,
                    Method::DELETE,
                    Method::OPTIONS,
                ] {
                    assert_not_implemented(method, &path, bearer).await;
                }
            }
        }
    }

    #[tokio::test]
    async fn without_a_database_the_routes_stay_501() {
        assert_eq!(
            status("/storage/v1/bucket", Some(&token("service_role"))).await,
            StatusCode::NOT_IMPLEMENTED
        );
    }

    #[tokio::test]
    async fn health_needs_a_valid_jwt_and_a_database() {
        for bearer in [Some("bad".to_string()), None] {
            assert_eq!(
                status("/storage/v1/health", bearer.as_deref()).await,
                StatusCode::NOT_IMPLEMENTED
            );
        }
        assert_eq!(
            status("/storage/v1/health", Some(&token("anon"))).await,
            StatusCode::NOT_IMPLEMENTED
        );
    }

    #[tokio::test]
    async fn health_reports_a_failed_probe_as_unhealthy() {
        // Nothing listens on port 1, so the probe query fails.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(500))
            .connect_lazy("postgres://postgres@127.0.0.1:1/postgres")
            .unwrap();
        let app = router_with_state(StorageState::new(
            Some(pool),
            Some(Hs256::new(SECRET).unwrap()),
        ));
        for role in ["anon", "authenticated", "service_role"] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::get("/storage/v1/health")
                        .header(header::AUTHORIZATION, format!("Bearer {}", token(role)))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "application/json; charset=utf-8"
            );
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert_eq!(&body[..], br#"{"healthy":false}"#);
        }
    }

    #[test]
    fn bucket_json_follows_the_response_schema() {
        let row = BucketRow {
            id: "avatars".into(),
            name: "avatars".into(),
            owner: None,
            public: None,
            created_at: DateTime::from_timestamp(1_613_537_012, 770_206_000),
            updated_at: None,
            file_size_limit: Some(1_000_000),
            allowed_mime_types: None,
            r#type: Some("STANDARD".into()),
        };
        assert_eq!(
            row.to_json(true),
            json!({
                "id": "avatars",
                "name": "avatars",
                "owner": "",
                "public": false,
                "type": "STANDARD",
                "file_size_limit": 1_000_000,
                "allowed_mime_types": null,
                "created_at": "2021-02-17T04:43:32.770Z",
                "updated_at": "",
            })
        );
        assert!(row.to_json(false).get("type").is_none());
    }
}
