// Megabase Server - Gateway and assembly
// Routes requests to the appropriate component based on URL path
// Same URL layout as Supabase: /rest/v1, /auth/v1, /storage/v1, /realtime/v1, /functions/v1, /pg

use axum::{http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use megabase_core::Config;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::info;

pub use megabase_core::MegabaseNotImplemented;

pub fn create_router() -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .nest("/rest/v1", megabase_rest::router())
        .nest("/auth/v1", megabase_auth::router())
        .nest("/realtime/v1", megabase_realtime::router())
        .nest("/storage/v1", megabase_storage::router())
        .nest("/functions/v1", megabase_functions::router())
        .nest("/pooler", megabase_pooler::router())
        .nest("/pg", megabase_meta::router())
        .nest("/studio", megabase_studio::router())
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}

async fn root_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "name": "Megabase",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Supabase-compatible API server, written in Rust",
        "endpoints": {
            "rest": "/rest/v1",
            "auth": "/auth/v1",
            "realtime": "/realtime/v1",
            "storage": "/storage/v1",
            "functions": "/functions/v1",
            "pooler": "/pooler",
            "meta": "/pg",
            "studio": "/studio"
        }
    }))
}

async fn health_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "healthy",
            "version": env!("CARGO_PKG_VERSION")
        })),
    )
}

pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let addr = config.bind_address();
    info!("Starting Megabase server on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, create_router()).await?;

    Ok(())
}
