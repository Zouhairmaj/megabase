// Megabase Storage - Supabase Storage-compatible file storage API
// Ported from Supabase Storage (Apache-2.0 license) - see NOTICE
// Upstream: vendor/storage

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
        .route("/bucket", any(bucket_list_handler))
        .route("/bucket/{bucket_id}", any(bucket_handler))
        .route("/object/list/{bucket_id}", any(object_list_handler))
        .route("/object/move", any(object_move_handler))
        .route("/object/copy", any(object_copy_handler))
        .nest("/object/sign", object_sign_router())
        .nest("/object/public", object_public_router())
        .nest("/object", object_router())
        .nest("/upload/resumable", upload_resumable_router())
        .fallback(fallback_handler)
}

fn object_router() -> Router {
    Router::new().fallback(object_handler)
}

fn object_sign_router() -> Router {
    Router::new().fallback(object_sign_handler)
}

fn object_public_router() -> Router {
    Router::new().fallback(object_public_handler)
}

fn upload_resumable_router() -> Router {
    Router::new().fallback(upload_resumable_handler)
}

async fn root_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::storage("GET /storage/v1/")
}

async fn bucket_list_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::storage(format!("{} /storage/v1/bucket", method))
}

async fn bucket_handler(method: Method, Path(bucket_id): Path<String>) -> MegabaseNotImplemented {
    MegabaseNotImplemented::storage(format!("{} /storage/v1/bucket/{}", method, bucket_id))
}

async fn object_handler(method: Method, uri: axum::http::Uri) -> MegabaseNotImplemented {
    MegabaseNotImplemented::storage(format!("{} /storage/v1/object{}", method, uri.path()))
}

async fn object_list_handler(
    method: Method,
    Path(bucket_id): Path<String>,
) -> MegabaseNotImplemented {
    MegabaseNotImplemented::storage(format!("{} /storage/v1/object/list/{}", method, bucket_id))
}

async fn object_move_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::storage(format!("{} /storage/v1/object/move", method))
}

async fn object_copy_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::storage(format!("{} /storage/v1/object/copy", method))
}

async fn object_sign_handler(method: Method, uri: axum::http::Uri) -> MegabaseNotImplemented {
    MegabaseNotImplemented::storage(format!("{} /storage/v1/object/sign{}", method, uri.path()))
}

async fn object_public_handler(method: Method, uri: axum::http::Uri) -> MegabaseNotImplemented {
    MegabaseNotImplemented::storage(format!(
        "{} /storage/v1/object/public{}",
        method,
        uri.path()
    ))
}

async fn upload_resumable_handler(method: Method, uri: axum::http::Uri) -> MegabaseNotImplemented {
    MegabaseNotImplemented::storage(format!(
        "{} /storage/v1/upload/resumable{}",
        method,
        uri.path()
    ))
}

async fn fallback_handler(method: Method, uri: axum::http::Uri) -> MegabaseNotImplemented {
    MegabaseNotImplemented::storage(format!("{} /storage/v1{}", method, uri.path()))
}
