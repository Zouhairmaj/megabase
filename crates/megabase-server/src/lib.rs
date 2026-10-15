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
mod cors;
mod key_auth;
mod layers;
mod target;

use megabase_core::Config;
use tower::ServiceExt;
use tracing::info;

pub use cors::{cors_applies, CORS_ALLOW_METHODS, ROUTES_WITHOUT_CORS};
use key_auth::reject_missing_key;
pub use key_auth::{key_auth_applies, KEY_AUTH_EXEMPT_ROUTES, KEY_AUTH_ROUTES};
use layers::apply_http_layers;

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

/// Per-component state the gateway hands to each router.
struct Services<'a> {
    auth: &'a megabase_auth::AuthState,
    rest: &'a megabase_rest::RestState,
    meta: &'a megabase_meta::MetaState,
    storage: &'a megabase_storage::StorageState,
    functions: &'a megabase_functions::FunctionsState,
}

impl Services<'_> {
    fn router(&self, component: &str) -> Router {
        match component {
            megabase_auth::COMPONENT => megabase_auth::router_with_state(self.auth.clone()),
            megabase_rest::COMPONENT => megabase_rest::router(self.rest.clone()),
            megabase_realtime::COMPONENT => megabase_realtime::router(),
            megabase_storage::COMPONENT => {
                megabase_storage::router_with_state(self.storage.clone())
            }
            megabase_functions::COMPONENT => megabase_functions::router(self.functions.clone()),
            megabase_meta::COMPONENT => megabase_meta::router_with_state(self.meta.clone()),
            _ => megabase_studio::router(),
        }
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
///
/// Uses [`Config::default`] HTTP limits. [`create_router_from`] applies the
/// process config, including timeout and body limit.
pub fn create_router_with(auth: &megabase_auth::AuthState) -> Router {
    create_router_from(auth, &Config::default())
}

/// Gateway plus the production tower-http stack from `config`.
pub fn create_router_from(auth: &megabase_auth::AuthState, config: &Config) -> Router {
    let rest = megabase_rest::RestState::from_config(config);
    let meta = megabase_meta::MetaState::new(None, config.jwt_hs256().ok());
    let storage = megabase_storage::StorageState::new(None, config.jwt_hs256().ok());
    apply_http_layers(
        assemble(&Services {
            auth,
            rest: &rest,
            meta: &meta,
            storage: &storage,
            functions: &megabase_functions::FunctionsState::disabled(),
        }),
        config,
    )
}

fn assemble(services: &Services<'_>) -> Router {
    let gateway = Gateway {
        routes: Arc::new(
            GATEWAY_ROUTES
                .iter()
                .map(|(prefix, component)| (*prefix, services.router(component)))
                .collect(),
        ),
        fallback: megabase_studio::router(),
    };
    Router::new()
        .route(HEALTH_PATH, get(health))
        .fallback(dispatch)
        .with_state(gateway)
}

async fn dispatch(State(gateway): State<Gateway>, request: Request) -> Response {
    let path = request.uri().path();
    if let Some(rejection) = reject_missing_key(path, request.uri().query(), request.headers()) {
        return rejection;
    }
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
        info!("installing standard roles");
        megabase_auth::install_roles(&url)
            .await
            .map_err(std::io::Error::other)?;
        info!("installing platform schemas");
        megabase_auth::install_platform(&url)
            .await
            .map_err(std::io::Error::other)?;
        info!("installing auth schema");
        megabase_auth::install_schema(&url)
            .await
            .map_err(std::io::Error::other)?;
        auth.backend = megabase_auth::Backend::connect(&url)
            .await
            .map_err(std::io::Error::other)?;
    } else {
        info!("DATABASE_URL unset; skipping role, platform and auth schema install");
    }
    let mut rest = megabase_rest::RestState::from_config(&config);
    // REST, Postgres Meta and Storage share one pool; each request that
    // changes session state does so with `SET LOCAL` inside a transaction.
    let mut pool = None;
    if let Some(url) = config.database_url.as_deref() {
        info!("connecting rest pool");
        let shared = megabase_rest::connect(url)
            .await
            .map_err(std::io::Error::other)?;
        info!("installing storage bucket schema");
        megabase_storage::install_schema(&shared)
            .await
            .map_err(std::io::Error::other)?;
        rest = rest.with_pool(shared.clone());
        pool = Some(shared);
    }
    let meta = megabase_meta::MetaState::new(pool.clone(), config.jwt_hs256().ok());
    let storage = megabase_storage::StorageState::new(pool, config.jwt_hs256().ok());
    let functions = megabase_functions::FunctionsState::from_env(&config);
    if !functions.is_enabled() {
        info!("MEGABASE_FUNCTIONS_DIR unset; /functions/v1 answers 501");
    }
    let addr = config.bind_address();
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("megabase listening on {}", listener.local_addr()?);
    let app = apply_http_layers(
        assemble(&Services {
            auth: &auth,
            rest: &rest,
            meta: &meta,
            storage: &storage,
            functions: &functions,
        }),
        &config,
    );
    serve_until_drained(listener, app, shutdown_signal(), SHUTDOWN_DRAIN).await?;
    auth.backend.close().await;
    rest.close().await;
    info!("megabase stopped");
    Ok(())
}

/// How long in-flight requests may finish after SIGINT or SIGTERM.
/// The listener stops as soon as the signal arrives. This bound is independent
/// of `MEGABASE_HTTP_TIMEOUT_MS`: a slow request must not hold the Auth pool
/// open for the whole request deadline.
const SHUTDOWN_DRAIN: std::time::Duration = std::time::Duration::from_secs(10);

/// Stop accepting when `shutdown` completes. Wait up to `drain` for in-flight
/// requests, then drop the server future so the caller can close the pool.
async fn serve_until_drained(
    listener: tokio::net::TcpListener,
    app: Router,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    drain: std::time::Duration,
) -> std::io::Result<()> {
    let (tx, mut graceful) = tokio::sync::watch::channel(false);
    // Connection tasks are spawned, so dropping this future stops the accept
    // loop and lets the caller close the pool; it does not wait out
    // `MEGABASE_HTTP_TIMEOUT_MS`.
    let server = serve(listener, app, async move {
        let _ = graceful.wait_for(|stopping| *stopping).await;
    });
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => return result,
        () = shutdown => {}
    }
    info!("shutdown signal received");
    let _ = tx.send(true);
    tokio::select! {
        result = server => result?,
        () = tokio::time::sleep(drain) => {
            info!("shutdown drain elapsed; closing open connections");
        }
    }
    Ok(())
}

/// `axum::serve(listener, app).with_graceful_shutdown(signal)` (axum 0.7.9
/// `serve.rs`, HTTP/1), with every connection read through [`target::Encoded`] so a
/// raw `"`, `<` or `>` in the request-target reaches the router instead of
/// failing in hyper with an empty `400`.
async fn serve(
    listener: tokio::net::TcpListener,
    app: Router,
    signal: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    use hyper::server::conn::http1::Builder;
    use hyper_util::rt::TokioIo;
    use hyper_util::service::TowerToHyperService;

    let (signal_tx, signal_rx) = tokio::sync::watch::channel(());
    let signal_tx = Arc::new(signal_tx);
    tokio::spawn(async move {
        signal.await;
        drop(signal_rx);
    });
    let (close_tx, close_rx) = tokio::sync::watch::channel(());
    loop {
        let stream = tokio::select! {
            accepted = accept(&listener) => match accepted {
                Some(stream) => stream,
                None => continue,
            },
            () = signal_tx.closed() => break,
        };
        let io = TokioIo::new(target::Encoded::new(stream));
        let service = TowerToHyperService::new(app.clone());
        let signal_tx = Arc::clone(&signal_tx);
        let close_rx = close_rx.clone();
        tokio::spawn(async move {
            // HTTP/1 only, as axum without its `http2` feature. Upgrades are
            // needed for websockets.
            let conn = Builder::new().serve_connection(io, service).with_upgrades();
            tokio::pin!(conn);
            let closed = signal_tx.closed();
            tokio::pin!(closed);
            let mut draining = false;
            loop {
                tokio::select! {
                    result = conn.as_mut() => {
                        if let Err(error) = result {
                            tracing::debug!(%error, "failed to serve connection");
                        }
                        break;
                    }
                    () = &mut closed, if !draining => {
                        draining = true;
                        conn.as_mut().graceful_shutdown();
                    }
                }
            }
            drop(close_rx);
        });
    }
    drop(close_rx);
    drop(listener);
    close_tx.closed().await;
    Ok(())
}

/// One accepted stream. A reset before `accept` returns is skipped; any other
/// error (such as `EMFILE`) is logged and retried after a second, as axum does.
async fn accept(listener: &tokio::net::TcpListener) -> Option<tokio::net::TcpStream> {
    match listener.accept().await {
        Ok((stream, _)) => Some(stream),
        Err(error) => {
            if !matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::ConnectionReset
            ) {
                tracing::error!(%error, "accept error");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            None
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async { tokio::signal::ctrl_c().await };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => match signal.recv().await {
                Some(()) => Ok(()),
                None => Err(std::io::Error::other("SIGTERM listener closed")),
            },
            Err(error) => Err(error),
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<std::io::Result<()>>();
    first_delivered_signal(ctrl_c, terminate).await;
}

/// A listener that fails stays pending so the other signal can still stop
/// the process. Completing on the error would shut the server down at startup.
async fn first_delivered_signal(
    ctrl_c: impl std::future::Future<Output = std::io::Result<()>>,
    terminate: impl std::future::Future<Output = std::io::Result<()>>,
) {
    tokio::select! {
        result = ctrl_c => hold_unless_delivered(result, "ctrl-c").await,
        result = terminate => hold_unless_delivered(result, "SIGTERM").await,
    }
}

async fn hold_unless_delivered(result: std::io::Result<()>, name: &str) {
    if let Err(error) = result {
        tracing::error!(%error, "failed to listen for {name}");
        std::future::pending::<()>().await;
    }
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

    #[tokio::test]
    async fn shutdown_drain_returns_while_a_handler_is_pending() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (trigger, shutdown) = tokio::sync::oneshot::channel::<()>();
        let drain = std::time::Duration::from_millis(200);
        let server = tokio::spawn(serve_until_drained(
            listener,
            Router::new().route(
                "/hang",
                get(|| async {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    "done"
                }),
            ),
            async move {
                let _ = shutdown.await;
            },
            drain,
        ));

        let mut stream = None;
        for _ in 0..50 {
            match tokio::net::TcpStream::connect(addr).await {
                Ok(connected) => {
                    stream = Some(connected);
                    break;
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
        }
        let mut stream = stream.expect("listener");
        tokio::io::AsyncWriteExt::write_all(
            &mut stream,
            b"GET /hang HTTP/1.1\r\nHost: localhost\r\n\r\n",
        )
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let started = std::time::Instant::now();
        trigger.send(()).unwrap();
        server.await.unwrap().unwrap();
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "drain waited {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn a_raw_json_arrow_in_the_target_reaches_the_router() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (trigger, shutdown) = tokio::sync::oneshot::channel::<()>();
        let app =
            Router::new().route(
                "/echo",
                get(|request: Request| async move {
                    request.uri().query().unwrap_or_default().to_string()
                }),
            );
        let server = tokio::spawn(serve(listener, app, async move {
            let _ = shutdown.await;
        }));
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        // Two requests on one connection; the second follows a sized body.
        stream
            .write_all(
                b"POST /echo?a->b HTTP/1.1\r\nHost: h\r\nContent-Length: 4\r\n\r\nx->y\
                  GET /echo?select=payload->user->>name&b=eq.\"<x>\" HTTP/1.1\r\nHost: h\r\n\
                  Connection: close\r\n\r\n",
            )
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        // `POST` has no route: 405, not hyper's empty 400.
        assert!(response.starts_with("HTTP/1.1 405"), "{response}");
        assert!(
            response.ends_with("select=payload-%3Euser-%3E%3Ename&b=eq.%22%3Cx%3E%22"),
            "{response}"
        );
        assert!(!response.contains(" 400 "), "{response}");
        trigger.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .expect("idle server stops")
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn a_failed_signal_listener_does_not_finish_shutdown() {
        let failed = async { Err(std::io::Error::other("listener failed")) };
        let other = std::future::pending::<std::io::Result<()>>();
        let finished = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            first_delivered_signal(failed, other),
        )
        .await;
        assert!(finished.is_err());
    }

    #[tokio::test]
    async fn a_delivered_signal_finishes_shutdown() {
        let delivered = async { Ok(()) };
        let other = std::future::pending::<std::io::Result<()>>();
        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            first_delivered_signal(delivered, other),
        )
        .await
        .expect("a delivered signal completes shutdown");
    }
}
