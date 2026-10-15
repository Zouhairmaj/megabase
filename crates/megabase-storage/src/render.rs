//! `GET /render/image/{authenticated,public,sign}/{bucketName}/{*}`: image
//! transformation (`specs/storage/render.md`).
//!
//! Megabase has no object backend and no imgproxy, so a request that reaches
//! the renderer is a 501. The checks storage-api runs before it (query
//! validation, the signed URL token, the object key, the bucket and object
//! lookups) answer as upstream does.

// Ported from supabase/storage src/http/routes/render/index.ts,
// src/http/routes/render/renderAuthenticatedImage.ts,
// src/http/routes/render/renderPublicImage.ts,
// src/http/routes/render/renderSignedImage.ts,
// src/http/schemas/transformations.ts, src/storage/object.ts,
// src/storage/database/pg.ts, src/internal/auth/jwt.ts and
// src/internal/errors/codes.ts (Apache-2.0), pin v1.80.2.

use std::borrow::Cow;

use axum::{
    extract::{rejection::PathRejection, OriginalUri, Path, Query, State},
    http::{HeaderMap, Method, Uri},
    response::{IntoResponse, Response},
};
use serde_json::Value;

use super::{
    database_error, limits::is_valid_key, not_implemented, service_role, StorageError, StorageState,
};

/// `findObject` with `object-versioning-core` applied: the current version.
const FIND_OBJECT_SQL: &str = "SELECT id FROM storage.objects \
     WHERE name COLLATE \"C\" = $1 AND bucket_id = $2 AND archived_at IS NULL LIMIT 1";

/// `findObject` with a `versionId`.
const FIND_OBJECT_VERSION_SQL: &str = "SELECT id FROM storage.objects \
     WHERE name COLLATE \"C\" = $1 AND bucket_id = $2 AND version = $3 LIMIT 1";

/// `findBucketById(bucketName, 'id,public', { isPublic: true })`.
const FIND_PUBLIC_BUCKET_SQL: &str =
    "SELECT id FROM storage.buckets WHERE id = $1 AND public = $2 LIMIT 1";

/// `SIGNED_URL_SCOPE_DOWNLOAD`.
pub(super) const SCOPE_DOWNLOAD: &str = "download";

/// The parsed query string; `None` when it does not decode.
pub(super) fn query_pairs(uri: &Uri) -> Option<Vec<(String, String)>> {
    Query::<Vec<(String, String)>>::try_from_uri(uri)
        .ok()
        .map(|Query(pairs)| pairs)
}

/// The value of `key`: `Some(None)` when absent, `None` when repeated
/// (Fastify parses a repeated key to an array, which a string or number
/// property rejects).
pub(super) fn single<'a>(pairs: &'a [(String, String)], key: &str) -> Option<Option<&'a str>> {
    let mut values = pairs
        .iter()
        .filter(|(name, _)| name == key)
        .map(|(_, value)| value.as_str());
    match (values.next(), values.next()) {
        (value, None) => Some(value),
        (_, Some(_)) => None,
    }
}

/// A decimal integer string Ajv coerces to an integer in `min..=max`.
fn integer_in(value: &str, min: u64, max: u64) -> bool {
    value.len() <= 15
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|n| (min..=max).contains(&n))
}

/// A plain decimal string (`-1`, `0.25`) Ajv coerces to a finite number.
fn decimal(value: &str) -> Option<f64> {
    let digits = value.strip_prefix('-').unwrap_or(value);
    let (whole, fraction) = match digits.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (digits, None),
    };
    let all_digits =
        |s: &str| !s.is_empty() && s.len() <= 15 && s.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(whole) || fraction.is_some_and(|f| !all_digits(f)) {
        return None;
    }
    value.parse().ok()
}

/// Whether the query passes `renderImageQuerySchema` (the transformation
/// options plus `download` and `versionId`). Other keys are allowed. A value
/// this check does not recognise fails Fastify validation, which is not
/// ported, or uses a coercion Megabase does not reproduce; both are 501.
fn valid_render_query(pairs: &[(String, String)]) -> bool {
    let field = |key| single(pairs, key);
    let (
        Some(height),
        Some(width),
        Some(resize),
        Some(format),
        Some(quality),
        Some(gravity),
        Some(x_offset),
        Some(y_offset),
        Some(_download),
        Some(_version_id),
    ) = (
        field("height"),
        field("width"),
        field("resize"),
        field("format"),
        field("quality"),
        field("gravity"),
        field("x_offset"),
        field("y_offset"),
        field("download"),
        field("versionId"),
    )
    else {
        return false;
    };
    let one_of = |value: Option<&str>, allowed: &[&str]| value.is_none_or(|v| allowed.contains(&v));
    let size = |value: Option<&str>| value.is_none_or(|v| integer_in(v, 0, u64::MAX));
    let x = x_offset.map(decimal);
    let y = y_offset.map(decimal);
    if x.is_some_and(|x| x.is_none()) || y.is_some_and(|y| y.is_none()) {
        return false;
    }
    let focal = |offset: Option<Option<f64>>| {
        offset
            .flatten()
            .is_some_and(|value| (0.0..=1.0).contains(&value))
    };
    size(height)
        && size(width)
        && one_of(resize, &["cover", "contain", "fill"])
        && one_of(format, &["origin", "avif", "webp"])
        && quality.is_none_or(|q| integer_in(q, 20, 100))
        && one_of(
            gravity,
            &[
                "no", "so", "ea", "we", "noea", "nowe", "soea", "sowe", "ce", "sm", "fp",
            ],
        )
        && (gravity != Some("fp") || (focal(x) && focal(y)))
}

/// Whether `findObject` finds a row. `as_service_role` runs it as
/// `service_role`, which bypasses RLS like storage-api's superuser
/// connection (`public` and `sign`) and the service-role request scope
/// (`authenticated`); otherwise it runs as the pool user.
async fn find_object(
    pool: &sqlx::PgPool,
    bucket_id: &str,
    object_name: &str,
    version: Option<&str>,
    as_service_role: bool,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    if as_service_role {
        sqlx::query("SET LOCAL ROLE service_role")
            .execute(&mut *tx)
            .await?;
    }
    let sql = if version.is_some() {
        FIND_OBJECT_VERSION_SQL
    } else {
        FIND_OBJECT_SQL
    };
    let mut query = sqlx::query(sql).bind(object_name).bind(bucket_id);
    if let Some(version) = version {
        query = query.bind(version);
    }
    let row = query.fetch_optional(&mut *tx).await?;
    tx.commit().await?;
    Ok(row.is_some())
}

/// Whether `bucket_id` exists and is public, as the superuser.
async fn find_public_bucket(pool: &sqlx::PgPool, bucket_id: &str) -> Result<bool, sqlx::Error> {
    let row = sqlx::query(FIND_PUBLIC_BUCKET_SQL)
        .bind(bucket_id)
        .bind(true)
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

/// The outcome of a `findObject` that found nothing or something: a found
/// object goes to the renderer, which is not ported.
fn object_outcome(found: Result<bool, sqlx::Error>, method: &Method, uri: &Uri) -> Response {
    match found {
        Ok(false) => StorageError::no_such_key().into_response(),
        Ok(true) => not_implemented(method, uri),
        Err(error) => database_error(&error),
    }
}

/// `GET /render/image/authenticated/:bucketName/*`: query validation, JWT
/// auth, then `findObject` as the request role. Only the service role is
/// served: other roles need storage-api's RLS request scope, and a missing
/// or invalid JWT is `AccessDenied` with the verifier's message.
// megabase:unit storage:route:GET /storage/v1/render/image/authenticated/{bucketName}/{*}
pub(super) async fn render_authenticated_image(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let (Ok(Path((bucket_name, object_name))), Some(pairs)) = (path, query_pairs(&uri)) else {
        return not_implemented(&method, &uri);
    };
    if !valid_render_query(&pairs) {
        return not_implemented(&method, &uri);
    }
    let Some((pool, _claims)) = service_role(&state, &headers) else {
        return not_implemented(&method, &uri);
    };
    if !is_valid_key(&object_name) {
        return StorageError::invalid_key(&object_name).into_response();
    }
    let version = single(&pairs, "versionId").flatten();
    let found = find_object(pool, &bucket_name, &object_name, version, true).await;
    object_outcome(found, &method, &uri)
}

/// `GET /render/image/public/:bucketName/*`: query validation, then
/// `findBucket` (public only) and `findObject` as the superuser, raced with
/// `Promise.all`. The key check throws before either query. When both
/// lookups fail, which error storage-api reports depends on which query
/// returns first, so that case is a 501.
// megabase:unit storage:route:GET /storage/v1/render/image/public/{bucketName}/{*}
pub(super) async fn render_public_image(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
) -> Response {
    let (Ok(Path((bucket_name, object_name))), Some(pairs)) = (path, query_pairs(&uri)) else {
        return not_implemented(&method, &uri);
    };
    if !valid_render_query(&pairs) {
        return not_implemented(&method, &uri);
    }
    if !is_valid_key(&object_name) {
        return StorageError::invalid_key(&object_name).into_response();
    }
    let Some(pool) = state.pool.as_ref() else {
        return not_implemented(&method, &uri);
    };
    let version = single(&pairs, "versionId").flatten();
    let bucket = find_public_bucket(pool, &bucket_name).await;
    let object = find_object(pool, &bucket_name, &object_name, version, true).await;
    match (bucket, object) {
        (Ok(true), object) => object_outcome(object, &method, &uri),
        (Ok(false), Ok(true)) => StorageError::no_such_bucket().into_response(),
        (Ok(false), Ok(false)) => not_implemented(&method, &uri),
        (Err(error), _) | (_, Err(error)) => database_error(&error),
    }
}

/// `GET /render/image/sign/:bucketName/*?token=`: `verifyObjectSignature`
/// with the download scope, then `findObject` as the superuser on the
/// token's `url`. A token that fails verification is `InvalidJWT` with the
/// verifier's message, which is not ported.
// megabase:unit storage:route:GET /storage/v1/render/image/sign/{bucketName}/{*}
pub(super) async fn render_signed_image(
    State(state): State<StorageState>,
    path: Result<Path<(String, String)>, PathRejection>,
    method: Method,
    OriginalUri(uri): OriginalUri,
) -> Response {
    let (Ok(Path((bucket_name, object_name))), Some(pairs)) = (path, query_pairs(&uri)) else {
        return not_implemented(&method, &uri);
    };
    // `token` is required; `download` is an optional string.
    let (Some(Some(token)), Some(_download)) =
        (single(&pairs, "token"), single(&pairs, "download"))
    else {
        return not_implemented(&method, &uri);
    };
    let Some(payload) = verified_signed_payload(&state, token) else {
        return not_implemented(&method, &uri);
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
    // `ExpiredSignature` is unreachable: jose already rejects `exp <= now`.
    let version = match payload.get("versionId") {
        None => None,
        Some(Value::String(version)) => Some(version.as_str()),
        // pg would bind the JSON value as text or NULL; not ported.
        Some(_) => return not_implemented(&method, &uri),
    };
    let (token_bucket, token_object) = url.split_once('/').unwrap_or((url, ""));
    if !is_valid_key(token_object) {
        return StorageError::invalid_key(token_object).into_response();
    }
    let Some(pool) = state.pool.as_ref() else {
        return not_implemented(&method, &uri);
    };
    let found = find_object(pool, token_bucket, token_object, version, true).await;
    object_outcome(found, &method, &uri)
}

/// The payload of `token` when Megabase and jose's `jwtVerify` both accept
/// it. Megabase's verifier allows a 30 second `exp` leeway and does not read
/// `nbf` or `iat`, so those cases are left to the caller's 501.
pub(super) fn verified_signed_payload(
    state: &StorageState,
    token: &str,
) -> Option<serde_json::Map<String, Value>> {
    let claims = state.jwt.as_deref()?.verify(token).ok()?;
    let now = chrono::Utc::now().timestamp();
    if claims.exp.is_some_and(|exp| exp <= now)
        || claims.raw.contains_key("nbf")
        || claims.raw.get("iat").is_some_and(|iat| !iat.is_number())
    {
        return None;
    }
    Some(claims.raw)
}

/// `isDownloadScopedToken`: `scope` is `download`, or absent on a token
/// without `upsert` (a legacy download token).
pub(super) fn is_download_scoped(payload: &serde_json::Map<String, Value>) -> bool {
    match payload.get("scope") {
        Some(Value::String(scope)) => scope == SCOPE_DOWNLOAD,
        Some(_) => false,
        None => !payload.contains_key("upsert"),
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{to_bytes, Body},
        http::{header, StatusCode},
    };
    use megabase_core::Hs256;
    use serde_json::json;
    use tower::ServiceExt;

    use super::*;
    use crate::router_with_state;

    const SECRET: &str = "super-secret-jwt-token-with-at-least-32-characters-long";

    fn sign(payload: &Value) -> String {
        Hs256::new(SECRET)
            .unwrap()
            .sign(payload)
            .unwrap()
            .as_str()
            .to_string()
    }

    fn future() -> i64 {
        chrono::Utc::now().timestamp() + 3600
    }

    /// Nothing listens on port 1, so every query fails.
    fn unreachable_pool() -> sqlx::PgPool {
        sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(500))
            .connect_lazy("postgres://postgres@127.0.0.1:1/postgres")
            .unwrap()
    }

    async fn get(
        pool: Option<sqlx::PgPool>,
        path: &str,
        bearer: Option<&str>,
    ) -> (StatusCode, Value) {
        let app = router_with_state(StorageState::new(pool, Some(Hs256::new(SECRET).unwrap())));
        let mut request = axum::http::Request::get(path);
        if let Some(bearer) = bearer {
            request = request.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
        }
        let response = app
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    fn pairs(query: &str) -> Vec<(String, String)> {
        query_pairs(&format!("/x?{query}").parse().unwrap()).unwrap()
    }

    #[test]
    fn render_query_follows_the_transformation_schema() {
        for ok in [
            "",
            "width=100&height=0",
            "resize=cover&format=webp&quality=20",
            "quality=100&gravity=ce&x_offset=-100&y_offset=0.5",
            "gravity=fp&x_offset=0&y_offset=1",
            "download=cat.png&versionId=v1&other=1&other=2",
        ] {
            assert!(valid_render_query(&pairs(ok)), "{ok}");
        }
        for bad in [
            "width=",
            "width=-1",
            "width=1.5",
            "width=abc",
            "width=1&width=2",
            "resize=crop",
            "format=png",
            "quality=19",
            "quality=101",
            "gravity=north",
            "gravity=fp",
            "gravity=fp&x_offset=0.5",
            "gravity=fp&x_offset=2&y_offset=0.5",
            "x_offset=Infinity",
            "x_offset=1e3",
            "download=a&download=b",
        ] {
            assert!(!valid_render_query(&pairs(bad)), "{bad}");
        }
    }

    #[test]
    fn download_scope_follows_is_download_scoped_token() {
        let scoped = |payload: Value| is_download_scoped(payload.as_object().unwrap());
        assert!(scoped(json!({ "scope": "download" })));
        assert!(scoped(json!({ "url": "b/o" })));
        assert!(!scoped(json!({ "upsert": false })));
        assert!(!scoped(json!({ "scope": "upload" })));
        assert!(!scoped(json!({ "scope": null })));
    }

    #[tokio::test]
    async fn signed_token_must_match_the_path() {
        let token = sign(&json!({ "url": "avatars/other.png", "exp": future() }));
        let (status, body) = get(
            None,
            &format!("/storage/v1/render/image/sign/avatars/cat.png?token={token}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            json!({
                "statusCode": "400",
                "code": "InvalidSignature",
                "error": "InvalidSignature",
                "message": "Invalid signature",
            })
        );
    }

    #[tokio::test]
    async fn signed_token_must_be_download_scoped() {
        let token = sign(&json!({ "url": "avatars/cat.png", "upsert": true, "exp": future() }));
        let (status, body) = get(
            None,
            &format!("/storage/v1/render/image/sign/avatars/cat.png?token={token}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["message"], "Token is not scoped for download");
    }

    #[tokio::test]
    async fn signed_token_with_an_invalid_key_is_invalid_key() {
        let token = sign(&json!({ "url": "avatars/a#b", "exp": future() }));
        let (status, body) = get(
            None,
            &format!("/storage/v1/render/image/sign/avatars/a%23b?token={token}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "InvalidKey");
        assert_eq!(body["message"], "Invalid key: a#b");
    }

    #[tokio::test]
    async fn unported_signed_cases_stay_501() {
        let expired = sign(&json!({ "url": "avatars/cat.png", "exp": 1 }));
        let valid = sign(&json!({ "url": "avatars/cat.png", "exp": future() }));
        let not_before = sign(&json!({ "url": "avatars/cat.png", "nbf": 1 }));
        for query in [
            String::new(),
            "?token=bad".to_string(),
            format!("?token={expired}"),
            format!("?token={not_before}"),
            format!("?token={valid}&token={valid}"),
            // Valid, but there is no database for `findObject`.
            format!("?token={valid}"),
        ] {
            let (status, _) = get(
                None,
                &format!("/storage/v1/render/image/sign/avatars/cat.png{query}"),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{query}");
        }
    }

    #[tokio::test]
    async fn public_invalid_key_needs_no_database() {
        let (status, body) = get(
            None,
            "/storage/v1/render/image/public/avatars/caf%C3%A9",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            json!({
                "statusCode": "400",
                "code": "InvalidKey",
                "error": "InvalidKey",
                "message": "Invalid key: caf\u{e9}",
            })
        );
    }

    #[tokio::test]
    async fn public_failures_without_a_lookup_stay_501() {
        for path in [
            "/storage/v1/render/image/public/avatars/cat.png",
            "/storage/v1/render/image/public/avatars/cat.png?width=abc",
        ] {
            let (status, _) = get(None, path, None).await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{path}");
        }
        let (status, _) = get(
            Some(unreachable_pool()),
            "/storage/v1/render/image/public/avatars/cat.png",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn authenticated_needs_the_service_role() {
        let anon = sign(&json!({ "role": "anon" }));
        let service = sign(&json!({ "role": "service_role" }));
        for bearer in [None, Some("bad"), Some(anon.as_str())] {
            let (status, _) = get(
                Some(unreachable_pool()),
                "/storage/v1/render/image/authenticated/avatars/a%23b",
                bearer,
            )
            .await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        }
        let (status, body) = get(
            Some(unreachable_pool()),
            "/storage/v1/render/image/authenticated/avatars/a%23b",
            Some(&service),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["message"], "Invalid key: a#b");
        let (status, _) = get(
            Some(unreachable_pool()),
            "/storage/v1/render/image/authenticated/avatars/cat.png",
            Some(&service),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn nested_object_names_reach_the_handler() {
        let token = sign(&json!({ "url": "avatars/folder/cat.png", "exp": future() }));
        let (status, body) = get(
            None,
            &format!("/storage/v1/render/image/sign/avatars/folder/other.png?token={token}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "InvalidSignature");
    }
}
