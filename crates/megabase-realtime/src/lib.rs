//! Supabase Realtime-compatible API (`/realtime/v1`) for Megabase.
//!
//! Target behavior: Supabase Realtime, vendor/realtime (Apache-2.0), pinned in `vendor/`.
//! `GET /api/ping` is served (ported from `lib/realtime_web/controllers/ping_controller.ex`).
//! Every other request returns the structured `MEGABASE_NOT_IMPLEMENTED` 501.
//! When more logic is ported, each file gets a header naming the upstream
//! repository, path and license.

// Ported from supabase/realtime lib/realtime_web/controllers/ping_controller.ex (Apache-2.0), pin v2.143.3.

use axum::{
    http::header,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

pub const COMPONENT: &str = "realtime";

pub fn router() -> Router {
    Router::new()
        .route("/realtime/v1/api/ping", get(ping))
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
        let response = router()
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
        let response = router()
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
