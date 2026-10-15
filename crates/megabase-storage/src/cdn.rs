//! `DELETE /cdn`, `/cdn/{bucketName}` and `/cdn/{bucketName}/{*}`: CDN cache purge
//! (`specs/storage/cdn.md`).

// Ported from supabase/storage src/http/routes/cdn/index.ts,
// src/http/routes/cdn/purgeCache.ts, src/storage/cdn/cdn-cache-manager.ts,
// src/http/plugins/jwt.ts and src/internal/errors/codes.ts (Apache-2.0),
// pin v1.80.2.

use axum::{
    extract::{OriginalUri, Query, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::Response,
};
use serde_json::{json, Value};

use super::{fastify_json, not_implemented, verified_role, StorageState};

/// The only role `enforceJwtRoles` admits (`dbServiceRole`).
const SERVICE_ROLE: &str = "service_role";

/// Whether the query string passes `purgeQuerySchema`: other keys are
/// allowed, and `transformations` appears at most once as `true` or
/// `false`. Anything else fails Fastify validation, which is not ported.
fn valid_purge_query(uri: &Uri) -> bool {
    let Ok(Query(pairs)) = Query::<Vec<(String, String)>>::try_from_uri(uri) else {
        return false;
    };
    let mut transformations = pairs.iter().filter(|(key, _)| key == "transformations");
    match (transformations.next(), transformations.next()) {
        (None, _) => true,
        (Some((_, value)), None) => value == "true" || value == "false",
        (Some(_), Some(_)) => false,
    }
}

/// Every purge route: query validation, JWT auth with the service role,
/// then `CdnCacheManager.purge()`. Megabase has no
/// `CDN_PURGE_ENDPOINT_URL` setting, so the purge always throws
/// `MissingParameter`, as storage-api does when the variable is unset.
// megabase:unit storage:route:DELETE /storage/v1/cdn
// megabase:unit storage:route:DELETE /storage/v1/cdn/{bucketName}
// megabase:unit storage:route:DELETE /storage/v1/cdn/{bucketName}/{*}
pub(super) async fn purge_cache(
    State(state): State<StorageState>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    if !valid_purge_query(&uri) {
        return not_implemented(&method, &uri);
    }
    // A missing header fails `authSchema` validation and an invalid JWT is
    // `AccessDenied` with the verifier's message; neither is ported.
    let Some(role) = verified_role(&state, &headers) else {
        return not_implemented(&method, &uri);
    };
    if role.as_deref() != Some(SERVICE_ROLE) {
        return fastify_json(StatusCode::FORBIDDEN, &invalid_role());
    }
    fastify_json(StatusCode::BAD_REQUEST, &missing_endpoint())
}

/// `enforceJwtRole`: `AccessDenied` rendered with status 403.
fn invalid_role() -> Value {
    json!({
        "statusCode": "403",
        "error": "Unauthorized",
        "message": "Access denied: Invalid role",
        "code": "AccessDenied",
    })
}

/// `ERRORS.MissingParameter('CDN_PURGE_ENDPOINT_URL is not set')`, status 400.
fn missing_endpoint() -> Value {
    json!({
        "statusCode": "400",
        "error": "MissingParameter",
        "message": "Missing Required Parameter CDN_PURGE_ENDPOINT_URL is not set",
        "code": "MissingParameter",
    })
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use megabase_core::Hs256;
    use tower::ServiceExt;

    use super::super::{header, router_with_state};
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

    async fn delete(path: &str, bearer: Option<&str>) -> (StatusCode, Option<Value>) {
        let app = router_with_state(StorageState::new(None, Some(Hs256::new(SECRET).unwrap())));
        let mut request = axum::http::Request::delete(path);
        if let Some(bearer) = bearer {
            request = request.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
        }
        let response = app
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        if status != StatusCode::NOT_IMPLEMENTED {
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "application/json; charset=utf-8"
            );
        }
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&body).ok())
    }

    const PATHS: [&str; 5] = [
        "/storage/v1/cdn",
        "/storage/v1/cdn/",
        "/storage/v1/cdn/avatars",
        "/storage/v1/cdn/avatars/cat.png",
        "/storage/v1/cdn/avatars/folder/cat.png",
    ];

    #[tokio::test]
    async fn service_role_purge_reports_the_missing_endpoint() {
        let service = token(SERVICE_ROLE);
        for path in PATHS {
            for query in ["", "?transformations=true", "?transformations=false&x=1"] {
                let (status, body) = delete(&format!("{path}{query}"), Some(&service)).await;
                assert_eq!(status, StatusCode::BAD_REQUEST, "{path}{query}");
                assert_eq!(body, Some(missing_endpoint()));
            }
        }
    }

    #[tokio::test]
    async fn other_roles_are_denied() {
        for role in ["anon", "authenticated"] {
            for path in PATHS {
                let (status, body) = delete(path, Some(&token(role))).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
                assert_eq!(body, Some(invalid_role()));
            }
        }
    }

    #[tokio::test]
    async fn unported_validation_and_auth_failures_stay_501() {
        let service = token(SERVICE_ROLE);
        for path in [
            "/storage/v1/cdn?transformations=yes",
            "/storage/v1/cdn?transformations=",
            "/storage/v1/cdn/avatars?transformations=true&transformations=false",
            // An empty wildcard fails the `minLength: 1` params schema.
            "/storage/v1/cdn/avatars/",
        ] {
            let (status, _) = delete(path, Some(&service)).await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{path}");
        }
        for bearer in [Some("bad"), None] {
            let (status, _) = delete("/storage/v1/cdn", bearer).await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        }
    }
}
