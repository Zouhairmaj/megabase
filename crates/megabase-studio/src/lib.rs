// Megabase Studio - Supabase Studio-compatible dashboard API
// Ported from Supabase Studio (Apache-2.0 license) - see NOTICE
// Upstream: vendor/supabase/apps/studio

use axum::{http::Method, routing::get, Router};
use megabase_core::MegabaseNotImplemented;

pub fn router() -> Router {
    Router::new()
        .route("/", get(root_handler))
        .nest("/api", api_router())
        .fallback(fallback_handler)
}

fn api_router() -> Router {
    Router::new().fallback(api_handler)
}

async fn root_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::studio("GET /studio/")
}

async fn api_handler(method: Method, uri: axum::http::Uri) -> MegabaseNotImplemented {
    MegabaseNotImplemented::studio(format!("{} /studio/api{}", method, uri.path()))
}

async fn fallback_handler(method: Method, uri: axum::http::Uri) -> MegabaseNotImplemented {
    MegabaseNotImplemented::studio(format!("{} /studio{}", method, uri.path()))
}
