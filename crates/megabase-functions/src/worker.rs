//! User workers: one long-lived process per function, started on first use.
//!
//! Upstream `main` calls `EdgeRuntime.userWorkers.create({ servicePath })`
//! with `servicePath = /home/deno/functions/<name>` and reuses the worker
//! while it lives. Megabase starts the function's entrypoint with a
//! [`Runtime`] (normally [`DenoRuntime`]) and forwards requests to the
//! address it listens on.

use std::collections::HashMap;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::time::Instant;
use tracing::info;

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
/// Upstream `main` gives a worker `workerTimeoutMs = 60 * 1000`.
pub const DEFAULT_BOOT_TIMEOUT: Duration = Duration::from_secs(60);

/// Why a worker could not be started. `Display` follows edge-runtime's
/// `worker boot error` context; the handler prefixes `InvalidWorkerCreation: `.
#[derive(Debug, thiserror::Error)]
pub enum BootError {
    /// No `index.{ts,tsx,js,mjs,jsx}` in the function directory
    /// (edge-runtime `could not find an appropriate entrypoint`).
    #[error("worker boot error: could not find an appropriate entrypoint")]
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

    fn is_running(&mut self) -> bool {
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

/// Runs each function as `deno run --allow-all --no-prompt <entrypoint>`.
///
/// The function's own `Deno.serve(...)` call binds the loopback port chosen
/// here through `DENO_SERVE_ADDRESS` (`tcp:127.0.0.1:<port>`), which
/// overrides the address of every `Deno.serve` in the process. The child
/// inherits Megabase's environment plus [`DenoRuntime::new`]'s `env`, like
/// upstream `main` passes `Deno.env.toObject()` to the user worker.
#[derive(Debug)]
pub struct DenoRuntime {
    binary: PathBuf,
    env: Vec<(String, String)>,
    boot_timeout: Duration,
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
        let mut command = Command::new(&self.binary);
        command
            .arg("run")
            .arg("--allow-all")
            .arg("--no-prompt")
            .arg(spec.entrypoint)
            .envs(self.env.iter().map(|(key, value)| (key, value)))
            .env("DENO_SERVE_ADDRESS", format!("tcp:{addr}"))
            .stdin(Stdio::null())
            .kill_on_drop(true);
        if let Some(dir) = spec.entrypoint.parent() {
            command.current_dir(dir);
        }
        let mut child = command.spawn()?;
        info!(function = spec.name, %addr, "starting edge function worker");
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

type Slot = Arc<tokio::sync::Mutex<Option<WorkerHandle>>>;

/// Running workers by function name.
pub(crate) struct Workers {
    dir: PathBuf,
    runtime: Box<dyn Runtime>,
    slots: Mutex<HashMap<String, Slot>>,
}

impl Workers {
    pub(crate) fn new(dir: PathBuf, runtime: Box<dyn Runtime>) -> Self {
        Self {
            dir,
            runtime,
            slots: Mutex::default(),
        }
    }

    /// Address of a running worker for `name`, starting one if needed.
    /// Boots of the same function are serialized; other functions proceed.
    pub(crate) async fn address(&self, name: &str) -> Result<SocketAddr, BootError> {
        let entry = entrypoint(&self.dir, name)?;
        let slot = Arc::clone(
            self.slots
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(name.to_owned())
                .or_default(),
        );
        let mut current = slot.lock().await;
        if let Some(handle) = current.as_mut() {
            if handle.is_running() {
                return Ok(handle.addr());
            }
        }
        if let Some(old) = current.take() {
            old.stop().await;
        }
        let handle = self
            .runtime
            .start(WorkerSpec {
                name,
                entrypoint: &entry,
            })
            .await?;
        let addr = handle.addr();
        *current = Some(handle);
        Ok(addr)
    }

    /// Drop the worker for `name` so the next request starts a new one.
    pub(crate) async fn forget(&self, name: &str) {
        let slot = self
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .map(Arc::clone);
        if let Some(slot) = slot {
            let old = slot.lock().await.take();
            if let Some(old) = old {
                old.stop().await;
            }
        }
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
            "worker boot error: could not find an appropriate entrypoint"
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
}
