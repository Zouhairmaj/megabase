// Megabase Functions - Supabase Edge Functions-compatible serverless runtime
// Ported from Supabase Edge Runtime (Apache-2.0 license) - see NOTICE
// Upstream: vendor/edge-runtime

use axum::{
    extract::Path,
    http::Method,
    routing::{any, get},
    Router,
};
use megabase_core::MegabaseNotImplemented;

pub fn router() -> Router {
    Router::new()
        .route("/", get(root_handler))
        .route("/{function_name}", any(function_handler))
        .fallback(function_path_handler)
}

async fn root_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::functions("GET /functions/v1/")
}

async fn function_handler(
    method: Method,
    Path(function_name): Path<String>,
) -> MegabaseNotImplemented {
    MegabaseNotImplemented::functions(format!("{} /functions/v1/{}", method, function_name))
}

async fn function_path_handler(method: Method, uri: axum::http::Uri) -> MegabaseNotImplemented {
    MegabaseNotImplemented::functions(format!("{} /functions/v1{}", method, uri.path()))
}
