//! Postgres Meta-compatible API (`/pg`) for Megabase.
//!
//! Target behavior: Postgres Meta, vendor/postgres-meta (Apache-2.0), pinned in `vendor/`.
//! `GET /health` and `GET /config/version` are served (`specs/meta/health-version.md`).
//! Every other request returns the structured `MEGABASE_NOT_IMPLEMENTED` 501.
//!
//! The pinned gateway runs `acl` with only the `admin` group on `/pg/`, so a
//! request whose `apikey` is not the service-role key gets Kong's 403 before
//! Postgres Meta sees it. Megabase recognizes the service-role key by
//! verifying it as an HS256 JWT with `role: service_role`.

// Ported from supabase/postgres-meta src/server/app.ts, src/server/routes/config.ts
// and src/lib/sql/version.sql.ts (Apache-2.0), pin v0.100.0.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use megabase_core::{kong, Hs256};
use serde_json::{json, Value};

pub const COMPONENT: &str = "meta";

/// `src/lib/sql/version.sql.ts`. `int8` columns become JSON numbers because
/// `src/lib/db.ts` parses safe integers as numbers.
const VERSION_SQL: &str = "SELECT
  version(),
  current_setting('server_version_num') :: int8 AS version_number,
  (
    SELECT
      COUNT(*) AS active_connections
    FROM
      pg_stat_activity
  ) AS active_connections,
  current_setting('max_connections') :: int8 AS max_connections";

/// Database pool and service-role verifier for `/pg`.
#[derive(Clone, Default)]
pub struct MetaState {
    pool: Option<sqlx::PgPool>,
    jwt: Option<Arc<Hs256>>,
}

impl MetaState {
    /// State with an optional pool and the `JWT_SECRET` verifier.
    #[must_use]
    pub fn new(pool: Option<sqlx::PgPool>, jwt: Option<Hs256>) -> Self {
        Self {
            pool,
            jwt: jwt.map(Arc::new),
        }
    }
}

/// `/pg` routes without a database or JWT secret: every served route is 403.
pub fn router() -> Router {
    router_with_state(MetaState::default())
}

/// `/pg` routes sharing `state`.
pub fn router_with_state(state: MetaState) -> Router {
    let acl_state = state.clone();
    Router::new()
        .route("/pg/health", get(health))
        .route("/pg/config/version", get(version))
        .with_state(state)
        .merge(megabase_core::not_implemented_router(COMPONENT))
        .method_not_allowed_fallback(megabase_core::not_implemented_handler(COMPONENT))
        .layer(middleware::from_fn_with_state(acl_state, admin_only))
}

/// Kong's `acl` plugin (`allow: [admin]`) on the `meta` service.
async fn admin_only(State(state): State<MetaState>, request: Request, next: Next) -> Response {
    if is_service_role(&state, request.headers()) {
        next.run(request).await
    } else {
        kong::acl_forbidden()
    }
}

fn is_service_role(state: &MetaState, headers: &HeaderMap) -> bool {
    let Some(jwt) = state.jwt.as_deref() else {
        return false;
    };
    kong::api_key(headers, None)
        .and_then(|key| jwt.verify(key).ok())
        .is_some_and(|claims| claims.role.as_deref() == Some("service_role"))
}

/// `app.get('/health')`: `{ date: new Date() }`, which Fastify writes as
/// `Date.prototype.toJSON` (UTC, milliseconds).
// megabase:unit meta:route:GET /pg/health
async fn health() -> Response {
    let date = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    fastify_json(StatusCode::OK, &json!({ "date": date }))
}

/// `GET /config/version`: the first row of `VERSION_SQL`.
// megabase:unit meta:route:GET /pg/config/version
async fn version(State(state): State<MetaState>) -> Response {
    let Some(pool) = state.pool.as_ref() else {
        return no_connection();
    };
    let row: Result<(String, i64, i64, i64), sqlx::Error> =
        sqlx::query_as(VERSION_SQL).fetch_one(pool).await;
    match row {
        Ok((version, version_number, active_connections, max_connections)) => fastify_json(
            StatusCode::OK,
            &json!({
                "version": version,
                "version_number": version_number,
                "active_connections": active_connections,
                "max_connections": max_connections,
            }),
        ),
        Err(error) => {
            tracing::error!(%error, "postgres-meta version query failed");
            let message = match error.as_database_error() {
                Some(database) => database.message().to_string(),
                None => "connection failed".to_string(),
            };
            fastify_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                &json!({ "error": message }),
            )
        }
    }
}

/// `src/server/routes/index.ts` `onRequest` with no `PG_CONNECTION`: the
/// thrown error reaches Fastify's default error handler.
fn no_connection() -> Response {
    fastify_json(
        StatusCode::INTERNAL_SERVER_ERROR,
        &json!({
            "statusCode": 500,
            "error": "Internal Server Error",
            "message": "failed to get upstream connection details",
        }),
    )
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

    fn key(role: &str) -> String {
        Hs256::new(SECRET)
            .unwrap()
            .sign(&json!({ "role": role }))
            .unwrap()
            .as_str()
            .to_string()
    }

    fn app() -> Router {
        router_with_state(MetaState::new(None, Some(Hs256::new(SECRET).unwrap())))
    }

    async fn get(app: Router, path: &str, apikey: Option<&str>) -> (StatusCode, Value) {
        let mut request = axum::http::Request::get(path);
        if let Some(apikey) = apikey {
            request = request.header("apikey", apikey);
        }
        let response = app
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn health_reports_a_millisecond_utc_date() {
        let (status, body) = get(app(), "/pg/health", Some(&key("service_role"))).await;
        assert_eq!(status, StatusCode::OK);
        let date = body["date"].as_str().unwrap();
        assert_eq!(date.len(), "2024-01-01T00:00:00.000Z".len(), "{date}");
        assert!(date.ends_with('Z'));
        assert_eq!(body.as_object().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn non_admin_keys_get_the_acl_403() {
        for apikey in [Some(key("anon")), Some("not-a-jwt".to_string()), None] {
            let (status, body) = get(app(), "/pg/health", apikey.as_deref()).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(body["message"], "You cannot consume this service");
        }
        let (status, _) = get(router(), "/pg/health", Some(&key("service_role"))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "no secret, no admin");
    }

    #[tokio::test]
    async fn version_without_a_database_is_the_connection_error() {
        let (status, body) = get(app(), "/pg/config/version", Some(&key("service_role"))).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["message"], "failed to get upstream connection details");
    }

    #[tokio::test]
    async fn other_routes_stay_501() {
        let (status, body) = get(app(), "/pg/tables", Some(&key("service_role"))).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["unit"], "GET /pg/tables");
    }
}
