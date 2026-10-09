//! Gateway and assembly for Megabase.
//!
//! Routes requests the way the self-hosted Supabase gateway does (Kong,
//! `vendor/supabase/docker/volumes/api/kong.yml`, Apache-2.0): a request goes
//! to the first component whose Kong route path is a plain string prefix of the
//! request path, and everything else goes to Studio, like Kong's catch-all
//! `dashboard` route. Supavisor's management API is not routed through Kong
//! upstream, so it has no prefix here; see `docs/adr/0002-gateway-layout.md`.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use megabase_core::Config;
use tower::ServiceExt;
use tower_http::trace::TraceLayer;
use tracing::info;

/// Megabase's own liveness probe. The `/_megabase` prefix is not used by any
/// Supabase service, so it cannot shadow an upstream route.
pub const HEALTH_PATH: &str = "/_megabase/health";

/// `(Kong route path, component)` pairs. Matching is a plain string prefix
/// test, so `/auth/v1` without the trailing slash is Studio's, as in Kong.
pub const GATEWAY_ROUTES: &[(&str, &str)] = &[
    ("/auth/v1/", megabase_auth::COMPONENT),
    (
        "/.well-known/oauth-authorization-server",
        megabase_auth::COMPONENT,
    ),
    ("/rest/v1/", megabase_rest::COMPONENT),
    ("/graphql/v1", megabase_rest::COMPONENT),
    ("/realtime/v1/", megabase_realtime::COMPONENT),
    ("/storage/v1/", megabase_storage::COMPONENT),
    ("/functions/v1/", megabase_functions::COMPONENT),
    ("/pg/", megabase_meta::COMPONENT),
];

#[derive(Clone)]
struct Gateway {
    routes: Arc<Vec<(&'static str, Router)>>,
    fallback: Router,
}

fn component_router(component: &str) -> Router {
    match component {
        megabase_auth::COMPONENT => megabase_auth::router(),
        megabase_rest::COMPONENT => megabase_rest::router(),
        megabase_realtime::COMPONENT => megabase_realtime::router(),
        megabase_storage::COMPONENT => megabase_storage::router(),
        megabase_functions::COMPONENT => megabase_functions::router(),
        megabase_meta::COMPONENT => megabase_meta::router(),
        _ => megabase_studio::router(),
    }
}

pub fn create_router() -> Router {
    let gateway = Gateway {
        routes: Arc::new(
            GATEWAY_ROUTES
                .iter()
                .map(|(prefix, component)| (*prefix, component_router(component)))
                .collect(),
        ),
        fallback: megabase_studio::router(),
    };
    Router::new()
        .route(HEALTH_PATH, get(health))
        .fallback(dispatch)
        .with_state(gateway)
        .layer(TraceLayer::new_for_http())
}

async fn dispatch(State(gateway): State<Gateway>, request: Request) -> Response {
    let path = request.uri().path();
    let router = gateway
        .routes
        .iter()
        .find(|(prefix, _)| path.starts_with(prefix))
        .map(|(_, router)| router.clone())
        .unwrap_or_else(|| gateway.fallback.clone());
    match router.oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    }
    .into_response()
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "name": "megabase",
        "version": env!("CARGO_PKG_VERSION"),
        "status": "ok",
    }))
}

pub async fn run(config: Config) -> std::io::Result<()> {
    if let Some(url) = &config.database_url {
        info!("installing auth schema");
        megabase_auth::install_schema(url)
            .await
            .map_err(std::io::Error::other)?;
    } else {
        info!("DATABASE_URL unset; skipping auth schema install");
    }
    let addr = config.bind_address();
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("megabase listening on {}", listener.local_addr()?);
    axum::serve(listener, create_router()).await
}
