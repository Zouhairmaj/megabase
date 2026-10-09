// Megabase Pooler - Supavisor-compatible connection pooler
// Ported from Supavisor (Apache-2.0 license) - see NOTICE
// Upstream: vendor/supavisor

use axum::{http::Method, routing::get, Router};
use megabase_core::MegabaseNotImplemented;

pub fn router() -> Router {
    Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .fallback(fallback_handler)
}

async fn root_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::pooler("GET /pooler/")
}

async fn health_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::pooler("GET /pooler/health")
}

async fn fallback_handler(method: Method, uri: axum::http::Uri) -> MegabaseNotImplemented {
    MegabaseNotImplemented::pooler(format!("{} /pooler{}", method, uri.path()))
}
