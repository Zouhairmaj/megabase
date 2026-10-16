//! Supabase Realtime-compatible API (`/realtime/v1`) for Megabase.
//!
//! Target behavior: Supabase Realtime, vendor/realtime (Apache-2.0), pinned in `vendor/`.
//! `GET /api/ping` is served (ported from `lib/realtime_web/controllers/ping_controller.ex`),
//! so are `POST /api/broadcast` and `POST /api/broadcast/:topic/events/:event`
//! (`http_api`), and so is the Phoenix socket at `GET /websocket` (`websocket`,
//! `protocol`, `hub`, `term`, `token`). Every other request returns the structured
//! `MEGABASE_NOT_IMPLEMENTED` 501. Each file names its upstream source.

// Ported from supabase/realtime lib/realtime_web/controllers/ping_controller.ex (Apache-2.0), pin v2.143.3.

use std::sync::Arc;

use axum::{
    http::header,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use megabase_core::Hs256;

mod cdc;
mod http_api;
mod hub;
mod protocol;
mod rate;
mod schema;
mod subscriptions;
mod term;
mod token;
mod websocket;

pub use schema::{install_schema, SchemaError};

pub const COMPONENT: &str = "realtime";

/// What the Realtime routes need: the tenant's JWT secret and the topics
/// every socket shares. Clones share the topics.
#[derive(Debug, Clone, Default)]
pub struct RealtimeState {
    jwt: Option<Hs256>,
    hub: Arc<hub::Hub>,
    /// Postgres changes of the tenant database (`DATABASE_URL`).
    cdc: Option<Arc<cdc::Cdc>>,
}

impl RealtimeState {
    /// State that verifies socket tokens with `jwt` (`JWT_SECRET`). With
    /// `None` the WebSocket route answers 501.
    #[must_use]
    pub fn new(jwt: Option<Hs256>) -> Self {
        Self {
            jwt,
            hub: Arc::default(),
            cdc: None,
        }
    }

    /// The state with the tenant database: channels may join with
    /// `postgres_changes`, and the replication poller starts on the current
    /// Tokio runtime. `slot_name_suffix` is `SLOT_NAME_SUFFIX`; the slot name
    /// is cluster-wide, so a second Realtime on the same cluster needs one.
    #[must_use]
    pub fn with_pool(mut self, pool: sqlx::PgPool, slot_name_suffix: Option<&str>) -> Self {
        let cdc = Arc::new(cdc::Cdc::new(pool, slot_name_suffix));
        tokio::spawn(Arc::clone(&cdc).run());
        self.cdc = Some(cdc);
        self
    }
}

/// Realtime routes under `/realtime/v1`.
pub fn router(state: RealtimeState) -> Router {
    Router::new()
        .route("/realtime/v1/api/ping", get(ping))
        .route("/realtime/v1/api/broadcast", post(http_api::broadcast))
        .route(
            "/realtime/v1/api/broadcast/:topic/events/:event",
            post(http_api::broadcast_single),
        )
        .route("/realtime/v1/websocket", get(websocket::websocket))
        .with_state(Arc::new(state))
        .merge(megabase_core::not_implemented_router(COMPONENT))
        .method_not_allowed_fallback(megabase_core::not_implemented_handler(COMPONENT))
}

/// `PingController.ping`: Phoenix `json/2` writes `{"message":"Success"}`
/// with an explicit charset.
// megabase:unit realtime:route:GET /realtime/v1/api/ping
async fn ping() -> Response {
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        r#"{"message":"Success"}"#,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn ping_answers_success() {
        let response = router(RealtimeState::default())
            .oneshot(
                Request::get("/realtime/v1/api/ping")
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
        assert_eq!(&body[..], br#"{"message":"Success"}"#);
    }

    #[tokio::test]
    async fn other_paths_stay_501() {
        let response = router(RealtimeState::default())
            .oneshot(
                Request::get("/realtime/v1/api/other")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }
}
