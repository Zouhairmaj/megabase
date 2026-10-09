// Megabase REST - PostgREST-compatible REST API
// Ported from PostgREST (MIT license) - see NOTICE
// Upstream: vendor/postgrest

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
        .route("/rpc/{function}", any(rpc_handler))
        .fallback(table_handler)
}

async fn root_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::rest("GET /rest/v1/")
}

async fn table_handler(method: Method, uri: axum::http::Uri) -> MegabaseNotImplemented {
    MegabaseNotImplemented::rest(format!("{} /rest/v1{}", method, uri.path()))
}

async fn rpc_handler(method: Method, Path(function): Path<String>) -> MegabaseNotImplemented {
    MegabaseNotImplemented::rest(format!("{} /rest/v1/rpc/{}", method, function))
}
