//! Edge Functions gateway and runtime (`/functions/v1`) for Megabase.
//!
//! Target behavior: the self-hosted Supabase stack, where Kong strips
//! `/functions/v1` and forwards to Supabase Edge Runtime (vendor/edge-runtime,
//! MIT), whose `main` service (`vendor/supabase/docker/volumes/functions/main`,
//! Apache-2.0) checks the JWT and hands the request to a user worker for
//! `/home/deno/functions/<name>`.
//!
//! Megabase runs the same steps in Rust and starts each function as a Deno
//! child process ([`DenoRuntime`]). Without `MEGABASE_FUNCTIONS_DIR` there
//! is nothing to serve and every request returns the structured
//! `MEGABASE_NOT_IMPLEMENTED` 501. Spec: `specs/functions/invoke.md`.

pub mod bootstrap;
pub mod env;
pub mod gate;
pub mod proxy;
pub mod runtime;
mod user_workers;
pub mod worker;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use megabase_core::{Config, Hs256};
use serde_json::json;
use tracing::warn;

pub use gate::GateError;
pub use proxy::{ProxyError, UPSTREAM_HOST};
pub use user_workers::{HeapStatistics, MemStat};
pub use worker::{
    entrypoint, BootError, DenoRuntime, Runtime, StartFuture, WorkerHandle, WorkerSpec,
    DEFAULT_BOOT_TIMEOUT, ENTRYPOINTS,
};

pub const COMPONENT: &str = "functions";

/// Kong route path of the `functions-v1` service (`strip_path: true`).
pub const ROUTE_PREFIX: &str = "/functions/v1/";

/// Lifetime of an API key Megabase signs when `SUPABASE_ANON_KEY` /
/// `SUPABASE_SERVICE_ROLE_KEY` are not configured: the demo keys in
/// `vendor/supabase/docker/.env.example` are valid for about five years.
const MINTED_KEY_LIFETIME_SECS: i64 = 5 * 365 * 24 * 60 * 60;

/// Settings that decide how `/functions/v1/{name}` is served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionsSettings {
    /// Directory holding one sub-directory per function
    /// (`/home/deno/functions` upstream).
    pub functions_dir: PathBuf,
    /// `VERIFY_JWT == "true"` upstream: require a valid JWT on every
    /// non-`OPTIONS` request.
    pub verify_jwt: bool,
}

/// Shared state of the functions router. Cheap to clone.
#[derive(Clone, Default)]
pub struct FunctionsState {
    inner: Option<Arc<Inner>>,
}

struct Inner {
    verify_jwt: bool,
    jwt: Option<Hs256>,
    workers: worker::Workers,
}

impl FunctionsState {
    /// No functions directory: the router answers 501.
    #[must_use]
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Serve functions from `settings.functions_dir`, verifying JWTs with
    /// `jwt` (`JWT_SECRET`) and starting workers with `runtime`.
    #[must_use]
    pub fn new(settings: FunctionsSettings, jwt: Option<Hs256>, runtime: impl Runtime) -> Self {
        Self {
            inner: Some(Arc::new(Inner {
                verify_jwt: settings.verify_jwt,
                jwt,
                workers: worker::Workers::new(settings.functions_dir, Box::new(runtime)),
            })),
        }
    }

    /// Whether a functions directory is configured.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// `EdgeRuntime.userWorkers.tryCleanupIdleWorkers`: how many idle
    /// workers acknowledged early drop. A disabled router returns 0.
    pub async fn cleanup_idle_workers(&self, timeout_ms: u64) -> usize {
        match &self.inner {
            Some(inner) => inner.workers.cleanup_idle_workers(timeout_ms).await,
            None => 0,
        }
    }

    /// `EdgeRuntime.userWorkers.memStats` for workers still in the pool.
    /// A disabled router returns an empty map, and each entry's `stats` is
    /// `None` (no isolate heap sample).
    pub async fn user_worker_mem_stats(&self) -> BTreeMap<String, MemStat> {
        match &self.inner {
            Some(inner) => inner.workers.mem_stats().await,
            None => BTreeMap::new(),
        }
    }

    /// [`Self::from_lookup`] over the process environment.
    #[must_use]
    pub fn from_env(config: &Config) -> Self {
        Self::from_lookup(config, |key| std::env::var(key).ok())
    }

    /// Build the state from environment variables:
    ///
    /// - `MEGABASE_FUNCTIONS_DIR`: functions directory; unset or empty
    ///   disables the component.
    /// - `MEGABASE_DENO`: Deno executable, default `deno` on `PATH`.
    /// - `VERIFY_JWT` (the edge-runtime variable), else
    ///   `FUNCTIONS_VERIFY_JWT` (the `.env` name): exactly `true` turns the
    ///   JWT gate on.
    ///
    /// Workers get `SUPABASE_URL`, `SUPABASE_ANON_KEY`,
    /// `SUPABASE_SERVICE_ROLE_KEY` and `SUPABASE_DB_URL`; see [`worker_env`].
    #[must_use]
    pub fn from_lookup(config: &Config, lookup: impl Fn(&str) -> Option<String>) -> Self {
        let Some(functions_dir) = lookup("MEGABASE_FUNCTIONS_DIR").filter(|dir| !dir.is_empty())
        else {
            return Self::disabled();
        };
        let verify_jwt = lookup("VERIFY_JWT")
            .or_else(|| lookup("FUNCTIONS_VERIFY_JWT"))
            .is_some_and(|raw| raw == "true");
        let deno = lookup("MEGABASE_DENO")
            .filter(|bin| !bin.is_empty())
            .unwrap_or_else(|| "deno".to_owned());
        let jwt = config.jwt_hs256().ok();
        let env = worker_env(config, jwt.as_ref(), &lookup, unix_now());
        Self::new(
            FunctionsSettings {
                functions_dir: functions_dir.into(),
                verify_jwt,
            },
            jwt,
            DenoRuntime::new(deno, env),
        )
    }
}

/// Variables the pinned compose file gives the edge-runtime container, as
/// Megabase fills them. A value already in the environment wins.
///
/// - `SUPABASE_URL`: the gateway, `http://kong:8000` upstream; here
///   Megabase's own listener (loopback when bound to a wildcard address).
/// - `SUPABASE_ANON_KEY` / `SUPABASE_SERVICE_ROLE_KEY`: from the `.env`
///   names `ANON_KEY` / `SERVICE_ROLE_KEY`, else HS256 keys signed with
///   `JWT_SECRET` (`role`, `iss: supabase`, `iat`, `exp`), like the demo keys.
/// - `SUPABASE_DB_URL`: `DATABASE_URL`.
#[must_use]
pub fn worker_env(
    config: &Config,
    jwt: Option<&Hs256>,
    lookup: &impl Fn(&str) -> Option<String>,
    now_unix: i64,
) -> Vec<(String, String)> {
    let host = match config.host.as_str() {
        "" | "0.0.0.0" => "127.0.0.1".to_owned(),
        "::" | "[::]" => "[::1]".to_owned(),
        host if host.contains(':') && !host.starts_with('[') => format!("[{host}]"),
        host => host.to_owned(),
    };
    let key = |names: [&str; 2], role: &str| {
        names.into_iter().find_map(lookup).or_else(|| {
            jwt.and_then(|jwt| {
                jwt.sign(&json!({
                    "role": role,
                    "iss": "supabase",
                    "iat": now_unix,
                    "exp": now_unix + MINTED_KEY_LIFETIME_SECS,
                }))
                .ok()
                .map(|token| token.as_str().to_owned())
            })
        })
    };
    [
        (
            "SUPABASE_URL",
            lookup("SUPABASE_URL").or_else(|| Some(format!("http://{host}:{}", config.port))),
        ),
        (
            "SUPABASE_ANON_KEY",
            key(["SUPABASE_ANON_KEY", "ANON_KEY"], "anon"),
        ),
        (
            "SUPABASE_SERVICE_ROLE_KEY",
            key(
                ["SUPABASE_SERVICE_ROLE_KEY", "SERVICE_ROLE_KEY"],
                "service_role",
            ),
        ),
        (
            "SUPABASE_DB_URL",
            lookup("SUPABASE_DB_URL").or_else(|| config.database_url.clone()),
        ),
    ]
    .into_iter()
    .filter_map(|(name, value)| value.map(|value| (name.to_owned(), value)))
    .collect()
}

/// Router for the `/functions/v1/` prefix (full request paths).
pub fn router(state: FunctionsState) -> Router {
    match state.inner {
        Some(inner) => Router::new().fallback(invoke).with_state(inner),
        None => megabase_core::not_implemented_router(COMPONENT),
    }
}

fn message(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "msg": msg.into() }))).into_response()
}

// megabase:unit functions:route:* /functions/v1/{function_name}
async fn invoke(State(inner): State<Arc<Inner>>, request: Request) -> Response {
    if inner.verify_jwt && request.method() != Method::OPTIONS {
        if let Err(error) = gate::check(request.headers(), inner.jwt.as_ref(), unix_now()) {
            return message(StatusCode::UNAUTHORIZED, error.to_string());
        }
    }

    // Kong strips the route path; upstream `main` reads the first segment.
    let rest = request
        .uri()
        .path()
        .strip_prefix(ROUTE_PREFIX)
        .unwrap_or_default();
    let name = rest.split('/').next().unwrap_or_default().to_owned();
    if name.is_empty() {
        return message(StatusCode::BAD_REQUEST, "missing function name in request");
    }
    let target = match request.uri().query() {
        Some(query) => format!("/{rest}?{query}"),
        None => format!("/{rest}"),
    };

    let key = match inner.workers.create_main(&name).await {
        Ok(key) => key,
        Err(error) => {
            warn!(function = %name, %error, "edge function did not start");
            return message(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("InvalidWorkerCreation: {error}"),
            );
        }
    };
    match inner.workers.fetch(key, request, &target).await {
        Ok(response) => response,
        Err(error) => {
            warn!(function = %name, %error, "edge function did not answer");
            message(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
        }
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const SECRET: &str = "super-secret-jwt-token-with-at-least-32-characters-long";

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        move |key| map.get(key).cloned()
    }

    fn config() -> Config {
        Config::from_lookup(lookup(&[("JWT_SECRET", SECRET), ("MEGABASE_PORT", "8100")])).unwrap()
    }

    fn env_map(pairs: Vec<(String, String)>) -> HashMap<String, String> {
        pairs.into_iter().collect()
    }

    #[tokio::test]
    async fn a_disabled_router_has_no_user_workers() {
        let state = FunctionsState::disabled();
        assert_eq!(state.cleanup_idle_workers(0).await, 0);
        assert!(state.user_worker_mem_stats().await.is_empty());
    }

    #[test]
    fn without_a_functions_dir_the_component_is_disabled() {
        assert!(!FunctionsState::from_lookup(&config(), lookup(&[])).is_enabled());
        let empty = lookup(&[("MEGABASE_FUNCTIONS_DIR", "")]);
        assert!(!FunctionsState::from_lookup(&config(), empty).is_enabled());
        let set = lookup(&[("MEGABASE_FUNCTIONS_DIR", "/srv/functions")]);
        assert!(FunctionsState::from_lookup(&config(), set).is_enabled());
    }

    #[test]
    fn worker_env_signs_keys_and_points_at_megabase() {
        let config = config();
        let jwt = config.jwt_hs256().unwrap();
        let env = env_map(worker_env(&config, Some(&jwt), &lookup(&[]), 1_000));
        assert_eq!(env["SUPABASE_URL"], "http://127.0.0.1:8100");
        let anon = jwt.verify_at(&env["SUPABASE_ANON_KEY"], 1_000).unwrap();
        assert_eq!(anon.role.as_deref(), Some("anon"));
        assert_eq!(anon.raw["iss"], "supabase");
        let service = jwt
            .verify_at(&env["SUPABASE_SERVICE_ROLE_KEY"], 1_000)
            .unwrap();
        assert_eq!(service.role.as_deref(), Some("service_role"));
        assert!(!env.contains_key("SUPABASE_DB_URL"));
    }

    #[test]
    fn worker_env_prefers_configured_values() {
        let config = Config::from_lookup(lookup(&[
            ("MEGABASE_HOST", "::"),
            ("DATABASE_URL", "postgres://db"),
        ]))
        .unwrap();
        let env = env_map(worker_env(
            &config,
            None,
            &lookup(&[
                ("ANON_KEY", "anon-key"),
                ("SUPABASE_SERVICE_ROLE_KEY", "svc"),
            ]),
            0,
        ));
        assert_eq!(env["SUPABASE_URL"], "http://[::1]:8000");
        assert_eq!(env["SUPABASE_ANON_KEY"], "anon-key");
        assert_eq!(env["SUPABASE_SERVICE_ROLE_KEY"], "svc");
        assert_eq!(env["SUPABASE_DB_URL"], "postgres://db");

        let explicit = env_map(worker_env(
            &config,
            None,
            &lookup(&[("SUPABASE_URL", "http://kong:8000")]),
            0,
        ));
        assert_eq!(explicit["SUPABASE_URL"], "http://kong:8000");
        assert!(!explicit.contains_key("SUPABASE_ANON_KEY"));
    }
}
