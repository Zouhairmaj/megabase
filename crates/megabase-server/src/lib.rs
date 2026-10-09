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

/// First Kong prefix that is a plain string prefix of `path`.
///
/// `None` is Studio, like Kong's catch-all `dashboard` route. `/auth/v1`
/// without the trailing slash is therefore Studio.
#[must_use]
pub fn match_gateway_route(path: &str) -> Option<(&'static str, &'static str)> {
    GATEWAY_ROUTES
        .iter()
        .copied()
        .find(|(prefix, _)| path.starts_with(prefix))
}

/// Parse `raw` as an HTTP request-target (origin-form or absolute-form) and
/// return the Kong component that would handle it.
///
/// Rejects non-UTF-8, NUL, and CR/LF so a request-target cannot smuggle a
/// second line. This is the surface the `gateway_http` cargo-fuzz target hits.
#[must_use]
pub fn gateway_component_for_target(raw: &[u8]) -> Option<&'static str> {
    let target = std::str::from_utf8(raw).ok()?;
    if target.bytes().any(|b| matches!(b, b'\0' | b'\r' | b'\n')) {
        return None;
    }
    let uri = target.parse::<axum::http::Uri>().ok()?;
    Some(
        match_gateway_route(uri.path())
            .map(|(_, component)| component)
            .unwrap_or(megabase_studio::COMPONENT),
    )
}

#[derive(Clone)]
struct Gateway {
    routes: Arc<Vec<(&'static str, Router)>>,
    fallback: Router,
}

fn component_router(component: &str, auth: &megabase_auth::AuthState) -> Router {
    match component {
        megabase_auth::COMPONENT => megabase_auth::router_with_state(auth.clone()),
        megabase_rest::COMPONENT => megabase_rest::router(),
        megabase_realtime::COMPONENT => megabase_realtime::router(),
        megabase_storage::COMPONENT => megabase_storage::router(),
        megabase_functions::COMPONENT => megabase_functions::router(),
        megabase_meta::COMPONENT => megabase_meta::router(),
        _ => megabase_studio::router(),
    }
}

/// Gateway with reference Auth defaults, no signing key, and no database.
///
/// Benches and the 501 route tests use this. `run` passes a configured
/// [`megabase_auth::AuthState`] so signup and logout share one backend.
pub fn create_router() -> Router {
    create_router_with(&megabase_auth::AuthState::reference())
}

/// Gateway whose `/auth/v1/` and OAuth discovery prefixes share `auth`.
pub fn create_router_with(auth: &megabase_auth::AuthState) -> Router {
    let gateway = Gateway {
        routes: Arc::new(
            GATEWAY_ROUTES
                .iter()
                .map(|(prefix, component)| (*prefix, component_router(component, auth)))
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
    let router = match_gateway_route(path)
        .and_then(|(prefix, _)| {
            gateway
                .routes
                .iter()
                .find(|(p, _)| *p == prefix)
                .map(|(_, router)| router.clone())
        })
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
    let mut auth = megabase_auth::AuthState::try_from_env().map_err(std::io::Error::other)?;
    auth.database_url = config.database_url.clone();
    if let Some(url) = auth.database_url.clone() {
        info!("installing auth schema");
        megabase_auth::install_schema(&url)
            .await
            .map_err(std::io::Error::other)?;
        auth.backend = megabase_auth::Backend::connect(&url)
            .await
            .map_err(std::io::Error::other)?;
    } else {
        info!("DATABASE_URL unset; skipping auth schema install");
    }
    let addr = config.bind_address();
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("megabase listening on {}", listener.local_addr()?);
    axum::serve(listener, create_router_with(&auth)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_gateway_route_uses_kong_prefix() {
        assert_eq!(
            match_gateway_route("/rest/v1/todos"),
            Some(("/rest/v1/", megabase_rest::COMPONENT))
        );
        assert_eq!(
            match_gateway_route("/auth/v1/token"),
            Some(("/auth/v1/", megabase_auth::COMPONENT))
        );
        assert_eq!(match_gateway_route("/rest/v1"), None);
        assert_eq!(match_gateway_route("/auth/v1"), None);
        assert_eq!(match_gateway_route("/"), None);
    }

    #[test]
    fn gateway_component_for_target_parses_origin_form() {
        assert_eq!(
            gateway_component_for_target(b"/auth/v1/token"),
            Some(megabase_auth::COMPONENT)
        );
        assert_eq!(
            gateway_component_for_target(b"/rest/v1/todos?id=eq.1"),
            Some(megabase_rest::COMPONENT)
        );
        assert_eq!(
            gateway_component_for_target(b"/rest/v1"),
            Some(megabase_studio::COMPONENT)
        );
        assert_eq!(gateway_component_for_target(b"\n/rest/v1/"), None);
        assert_eq!(gateway_component_for_target(b"/rest/v1/\r"), None);
        assert_eq!(gateway_component_for_target(&[0xff, 0xfe]), None);
    }
}
