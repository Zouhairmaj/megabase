// Megabase Realtime - Supabase Realtime-compatible websocket API
// Ported from Supabase Realtime (Apache-2.0 license) - see NOTICE
// Upstream: vendor/realtime

use axum::{
    http::Method,
    routing::{any, get},
    Router,
};
use megabase_core::MegabaseNotImplemented;

pub fn router() -> Router {
    Router::new()
        .route("/", get(root_handler))
        .route("/websocket", any(websocket_handler))
        .route("/socket/websocket", any(socket_websocket_handler))
        .fallback(fallback_handler)
}

async fn root_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::realtime("GET /realtime/v1/")
}

async fn websocket_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::realtime(format!("{} /realtime/v1/websocket", method))
}

async fn socket_websocket_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::realtime(format!("{} /realtime/v1/socket/websocket", method))
}

async fn fallback_handler(method: Method, uri: axum::http::Uri) -> MegabaseNotImplemented {
    MegabaseNotImplemented::realtime(format!("{} /realtime/v1{}", method, uri.path()))
}
