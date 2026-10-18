//! User workers: one long-lived process per function, started on first use.
//!
//! Upstream `main` calls `EdgeRuntime.userWorkers.create({ servicePath })`
//! with `servicePath = /home/deno/functions/<name>` and reuses the worker
//! while it lives. Megabase starts the function's entrypoint with a
//! [`Runtime`] (normally [`DenoRuntime`]) and forwards requests to the
//! address it listens on.

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::time::Instant;
use tracing::{info, warn};

use crate::bootstrap;

/// Entrypoint names edge-runtime tries, in order
/// (`crates/base/src/runtime/mod.rs` `POTENTIAL_EXTS`, edge-runtime v1.77.4).
pub const ENTRYPOINTS: &[&str] = &[
    "index.ts",
    "index.tsx",
    "index.js",
    "index.mjs",
    "index.jsx",
];

/// How long a worker may take to accept connections after it starts.
/// This is the boot wait, separate from the wall-clock `workerTimeoutMs`
/// the pool applies after the process is listening (`specs/functions/workers.md`).
pub const DEFAULT_BOOT_TIMEOUT: Duration = Duration::from_secs(60);

/// Why a worker could not be started. `Display` follows edge-runtime's
/// `worker boot error` context; the handler prefixes `InvalidWorkerCreation: `.
#[derive(Debug, thiserror::Error)]
pub enum BootError {
    /// No `index.{ts,tsx,js,mjs,jsx}` in the function directory
    /// (edge-runtime `could not find an appropriate entrypoint`).
    #[error(
        "worker boot error: failed to bootstrap runtime: could not find an appropriate entrypoint"
    )]
    NoEntrypoint,
    /// The runtime binary could not be started.
    #[error("worker boot error: {0}")]
    Spawn(#[from] std::io::Error),
    /// The worker process ended before it accepted a connection.
    #[error("worker boot error: worker exited before listening ({0})")]
    Exited(ExitStatus),
    /// The worker did not accept a connection within the boot timeout.
    #[error("worker boot error: worker did not listen within {0:?}")]
    Timeout(Duration),
}

/// Environment variable carrying the entrypoint path to the bootstrap module.
pub const ENTRYPOINT_ENV: &str = "MEGABASE_FUNCTION_ENTRYPOINT";

/// What a [`Runtime`] needs to start one function.
#[derive(Debug, Clone, Copy)]
pub struct WorkerSpec<'a> {
    /// Function name: the first path segment after `/functions/v1/`.
    pub name: &'a str,
    /// The resolved `index.*` file.
    pub entrypoint: &'a Path,
}

/// A started worker: the HTTP/1.1 address it serves on and, when Megabase
/// owns it, its process (killed when the handle is dropped).
#[derive(Debug)]
pub struct WorkerHandle {
    addr: SocketAddr,
    child: Option<Child>,
}

impl WorkerHandle {
    /// A worker process listening on `addr`. The process is killed when the
    /// handle is dropped or replaced.
    #[must_use]
    pub fn process(addr: SocketAddr, child: Child) -> Self {
        Self {
            addr,
            child: Some(child),
        }
    }

    /// A worker Megabase does not own (it is always treated as running).
    #[must_use]
    pub fn external(addr: SocketAddr) -> Self {
        Self { addr, child: None }
    }

    /// Address requests are forwarded to.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Kill the process (if Megabase owns it) and wait for it, so it is
    /// reaped instead of left as a zombie.
    pub async fn stop(mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill().await;
        }
    }

    pub(crate) fn is_running(&mut self) -> bool {
        self.child
            .as_mut()
            .is_none_or(|child| matches!(child.try_wait(), Ok(None)))
    }
}

/// Future returned by [`Runtime::start`].
pub type StartFuture<'a> =
    Pin<Box<dyn Future<Output = Result<WorkerHandle, BootError>> + Send + 'a>>;

/// Starts user workers.
pub trait Runtime: Send + Sync + 'static {
    /// Start a worker for `spec` and resolve once it accepts connections.
    ///
    /// # Errors
    ///
    /// [`BootError`] when the worker cannot be started.
    fn start<'a>(&'a self, spec: WorkerSpec<'a>) -> StartFuture<'a>;
}

/// Runs each function as `deno run --allow-all --no-prompt <bootstrap>`.
/// The bootstrap is the main module: it installs the `Deno.listen` shim and
/// then imports the function's entrypoint, whose path it reads (and removes)
/// from [`ENTRYPOINT_ENV`]. `--preload` is not used: it needs Deno 2.4 and
/// edge-runtime v1.77.4 bundles Deno 2.1.4.
///
/// The function's own `Deno.serve(...)` call binds the loopback port chosen
/// here through `DENO_SERVE_ADDRESS` (`tcp:127.0.0.1:<port>`), which
/// overrides the address of every `Deno.serve` in the process. The
/// [`crate::bootstrap`] sends servers built on TCP `Deno.listen` (std
/// `serve()`) to that same port. The child's stderr goes to the Megabase log,
/// one `warn` event per line.
/// The child inherits Megabase's environment plus [`DenoRuntime::new`]'s
/// `env`, like upstream `main` passes `Deno.env.toObject()` to the user
/// worker.
#[derive(Debug)]
pub struct DenoRuntime {
    binary: PathBuf,
    env: Vec<(String, String)>,
    boot_timeout: Duration,
    bootstrap: tokio::sync::OnceCell<PathBuf>,
}

impl Drop for DenoRuntime {
    fn drop(&mut self) {
        if let Some(dir) = self.bootstrap.get().and_then(|path| path.parent()) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

impl DenoRuntime {
    /// Runtime that starts `binary` (a Deno 2 executable) with `env` added to
    /// the inherited environment.
    #[must_use]
    pub fn new(binary: impl Into<PathBuf>, env: Vec<(String, String)>) -> Self {
        Self {
            binary: binary.into(),
            env,
            boot_timeout: DEFAULT_BOOT_TIMEOUT,
            bootstrap: tokio::sync::OnceCell::new(),
        }
    }

    /// Replace [`DEFAULT_BOOT_TIMEOUT`].
    #[must_use]
    pub fn with_boot_timeout(mut self, timeout: Duration) -> Self {
        self.boot_timeout = timeout;
        self
    }

    async fn spawn(&self, spec: WorkerSpec<'_>) -> Result<WorkerHandle, BootError> {
        // The port is free when the OS hands it out; the child binds it a
        // moment later. Another process could take it in between, which
        // shows up as a boot timeout or a worker that exits.
        let addr = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?.local_addr()?;
        let preload = self
            .bootstrap
            .get_or_try_init(|| async { bootstrap::install(&std::env::temp_dir()) })
            .await?;
        // The bootstrap builds a `file://` URL from this path and the child
        // runs in the function directory, so a relative functions dir must
        // be made absolute here.
        let entrypoint = std::path::absolute(spec.entrypoint)?;
        let mut command = Command::new(&self.binary);
        command
            .arg("run")
            .arg("--allow-all")
            .arg("--no-prompt")
            .arg(preload)
            .envs(self.env.iter().map(|(key, value)| (key, value)))
            .env(ENTRYPOINT_ENV, &entrypoint)
            .env("DENO_SERVE_ADDRESS", format!("tcp:{addr}"))
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(dir) = entrypoint.parent() {
            command.current_dir(dir);
        }
        let mut child = command.spawn()?;
        info!(function = spec.name, %addr, "starting edge function worker");
        if let Some(stderr) = child.stderr.take() {
            let function = spec.name.to_owned();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    warn!(function = %function, "edge function worker stderr: {line}");
                }
            });
        }
        let deadline = Instant::now() + self.boot_timeout;
        loop {
            if let Some(status) = child.try_wait()? {
                return Err(BootError::Exited(status));
            }
            if TcpStream::connect(addr).await.is_ok() {
                return Ok(WorkerHandle::process(addr, child));
            }
            if Instant::now() >= deadline {
                let _ = child.kill().await;
                return Err(BootError::Timeout(self.boot_timeout));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

impl Runtime for DenoRuntime {
    fn start<'a>(&'a self, spec: WorkerSpec<'a>) -> StartFuture<'a> {
        Box::pin(self.spawn(spec))
    }
}

/// The entrypoint of function `name` under `dir`, as edge-runtime resolves
/// `servicePath`.
///
/// `.` and `..` (and names with a backslash or NUL) never name a function:
/// upstream sees the WHATWG-normalized URL path, where dot segments are gone.
///
/// # Errors
///
/// [`BootError::NoEntrypoint`] when no `index.*` file exists.
pub fn entrypoint(dir: &Path, name: &str) -> Result<PathBuf, BootError> {
    if matches!(name, "" | "." | "..") || name.contains(['\\', '\0']) {
        return Err(BootError::NoEntrypoint);
    }
    let service = dir.join(name);
    ENTRYPOINTS
        .iter()
        .map(|file| service.join(file))
        .find(|path| path.is_file())
        .ok_or(BootError::NoEntrypoint)
}

/// Running workers, one pool for the five `op_user_worker_*` ops.
pub(crate) struct Workers {
    pool: crate::user_workers::Pool,
}

impl Workers {
    pub(crate) fn new(dir: PathBuf, runtime: Box<dyn Runtime>) -> Self {
        Self {
            pool: crate::user_workers::Pool::new(dir, runtime),
        }
    }

    /// `EdgeRuntime.userWorkers.create` as the pinned `main` service calls it.
    ///
    /// # Errors
    ///
    /// [`BootError`] when the worker cannot be started.
    pub(crate) async fn create_main(&self, name: &str) -> Result<uuid::Uuid, BootError> {
        self.pool.create_main(name).await
    }

    /// `worker.fetch` for `key`.
    ///
    /// # Errors
    ///
    /// [`crate::user_workers::SendError`] when the worker cannot answer.
    pub(crate) async fn fetch(
        &self,
        key: uuid::Uuid,
        request: axum::extract::Request,
        target: &str,
    ) -> Result<axum::response::Response, crate::user_workers::SendError> {
        self.pool.fetch_send(key, request, target).await
    }

    pub(crate) async fn cleanup_idle_workers(&self, timeout_ms: u64) -> usize {
        self.pool.cleanup_idle_workers(timeout_ms).await
    }

    pub(crate) async fn mem_stats(
        &self,
    ) -> std::collections::BTreeMap<String, crate::user_workers::MemStat> {
        self.pool.mem_stats().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "megabase-functions-worker-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn entrypoint_follows_edge_runtime_extension_order() {
        let dir = temp_dir("order");
        std::fs::create_dir_all(dir.join("hello")).unwrap();
        std::fs::write(dir.join("hello/index.js"), "").unwrap();
        assert_eq!(
            entrypoint(&dir, "hello").unwrap(),
            dir.join("hello/index.js")
        );
        std::fs::write(dir.join("hello/index.ts"), "").unwrap();
        assert_eq!(
            entrypoint(&dir, "hello").unwrap(),
            dir.join("hello/index.ts")
        );
    }

    #[test]
    fn missing_or_unsafe_names_have_no_entrypoint() {
        let dir = temp_dir("missing");
        std::fs::create_dir_all(dir.join("empty")).unwrap();
        std::fs::write(dir.join("index.ts"), "").unwrap();
        for name in ["nope", "empty", "", ".", "..", "a\\b", "a\0b"] {
            assert!(
                matches!(entrypoint(&dir, name), Err(BootError::NoEntrypoint)),
                "{name:?}"
            );
        }
        assert_eq!(
            BootError::NoEntrypoint.to_string(),
            "worker boot error: failed to bootstrap runtime: could not find an appropriate entrypoint"
        );
    }

    #[tokio::test]
    async fn a_missing_binary_is_a_spawn_error() {
        let dir = temp_dir("spawn");
        std::fs::write(dir.join("index.ts"), "").unwrap();
        let runtime = DenoRuntime::new(dir.join("no-such-deno"), Vec::new());
        let error = runtime
            .start(WorkerSpec {
                name: "spawn",
                entrypoint: &dir.join("index.ts"),
            })
            .await
            .unwrap_err();
        assert!(matches!(error, BootError::Spawn(_)), "{error}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_worker_that_exits_is_a_boot_error() {
        let dir = temp_dir("exit");
        std::fs::write(dir.join("index.ts"), "").unwrap();
        // `true` ignores its arguments and exits without listening.
        let runtime = DenoRuntime::new("true", Vec::new());
        let error = runtime
            .start(WorkerSpec {
                name: "exit",
                entrypoint: &dir.join("index.ts"),
            })
            .await
            .unwrap_err();
        assert!(matches!(error, BootError::Exited(_)), "{error}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_worker_runs_the_bootstrap_as_its_main_module() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = temp_dir("preload");
        std::fs::write(dir.join("index.ts"), "").unwrap();
        // A stand-in `deno` that records its arguments and exits.
        let fake = dir.join("fake-deno");
        std::fs::write(
            &fake,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > args\nprintf '%s' \"$MEGABASE_FUNCTION_ENTRYPOINT\" > entry\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let runtime = DenoRuntime::new(&fake, Vec::new());
        let error = runtime
            .start(WorkerSpec {
                name: "preload",
                entrypoint: &dir.join("index.ts"),
            })
            .await
            .unwrap_err();
        assert!(matches!(error, BootError::Exited(_)), "{error}");

        let preload = runtime.bootstrap.get().unwrap().clone();
        let entry = dir.join("index.ts");
        let args = std::fs::read_to_string(dir.join("args")).unwrap();
        assert_eq!(
            args.lines().collect::<Vec<_>>(),
            [
                "run",
                "--allow-all",
                "--no-prompt",
                preload.to_str().unwrap(),
            ]
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("entry")).unwrap(),
            entry.to_str().unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(&preload).unwrap(),
            bootstrap::module()
        );
        drop(runtime);
        assert!(!preload.parent().unwrap().exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_relative_entrypoint_is_passed_as_an_absolute_path() {
        use std::os::unix::fs::PermissionsExt as _;

        let tools = temp_dir("relative-tools");
        let fake = tools.join("fake-deno");
        std::fs::write(
            &fake,
            "#!/bin/sh\nprintf '%s' \"$MEGABASE_FUNCTION_ENTRYPOINT\" > \"$0.entry\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

        // A functions dir relative to the current directory, like
        // `MEGABASE_FUNCTIONS_DIR=vendor/supabase/docker/volumes/functions`.
        let relative = PathBuf::from(format!("megabase-relative-{}", std::process::id()));
        std::fs::create_dir_all(relative.join("hello")).unwrap();
        std::fs::write(relative.join("hello/index.ts"), "").unwrap();
        let runtime = DenoRuntime::new(&fake, Vec::new());
        let entry = entrypoint(&relative, "hello").unwrap();
        assert!(entry.is_relative());
        let result = runtime
            .start(WorkerSpec {
                name: "hello",
                entrypoint: &entry,
            })
            .await;
        let absolute = std::env::current_dir().unwrap().join(&entry);
        std::fs::remove_dir_all(&relative).unwrap();

        assert!(matches!(result, Err(BootError::Exited(_))));
        let seen = std::fs::read_to_string(tools.join("fake-deno.entry")).unwrap();
        assert_eq!(seen, absolute.to_str().unwrap());
    }
}
