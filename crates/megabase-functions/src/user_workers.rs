// Ported from supabase/edge-runtime ext/workers/lib.rs and ext/workers/user_workers.js (MIT), pin v1.77.4.

//! User-worker pool: the five `op_user_worker_*` ops of the main worker.
//!
//! The pinned `main` service calls `EdgeRuntime.userWorkers.create` and
//! `worker.fetch` for every function request. Megabase's gateway is that
//! main worker. Spec: `specs/functions/workers.md`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::extract::Request;
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method};
use axum::response::Response;
use uuid::Uuid;

use crate::proxy::{self, ProxyError};
use crate::worker::{entrypoint, BootError, Runtime, WorkerHandle, WorkerSpec};

/// `memory_limit_mb` when `create` omits it (edge-runtime `.cargo/config.toml`).
pub(crate) const DEFAULT_MEMORY_LIMIT_MB: u64 = 256;

/// `worker_timeout_ms` when `create` omits it (same file).
pub(crate) const DEFAULT_WORKER_TIMEOUT_MS: u64 = 400_000;

/// `memoryLimitMb` in the pinned `main` service.
pub(crate) const MAIN_MEMORY_LIMIT_MB: u64 = 150;

/// `workerTimeoutMs` in the pinned `main` service (`1 * 60 * 1000`).
pub(crate) const MAIN_WORKER_TIMEOUT_MS: u64 = 60_000;

/// Arguments of `op_user_worker_create` that change pool behavior.
#[derive(Debug, Clone)]
pub(crate) struct CreateOptions {
    /// Directory the caller names. The entrypoint is the final segment
    /// under the functions directory.
    pub service_path: String,
    /// Replaces `service_path` as the reuse key when set.
    pub pool_key: Option<String>,
    /// Skip reuse and start another worker.
    pub force_create: bool,
    /// `None` selects [`DEFAULT_MEMORY_LIMIT_MB`].
    pub memory_limit_mb: Option<u64>,
    /// `None` selects [`DEFAULT_WORKER_TIMEOUT_MS`]. `Some(0)` disables the
    /// wall clock.
    pub worker_timeout_ms: Option<u64>,
}

/// `(key, reused)` from `op_user_worker_create`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CreateResult {
    pub key: Uuid,
    pub reused: bool,
}

/// One `memStats` entry. `stats` is absent when the isolate heap cannot be
/// read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemStat {
    /// `servicePath` the worker was created with.
    pub service_path: String,
    /// V8 heap sample, or `None` when it cannot be read.
    pub stats: Option<HeapStatistics>,
}

/// V8 heap sample (`WorkerHeapStatistics`). Megabase has no isolate sample,
/// so [`Pool::mem_stats`] leaves this absent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HeapStatistics {
    pub total_heap_size: usize,
    pub total_heap_size_executable: usize,
    pub total_physical_size: usize,
    pub total_available_size: usize,
    pub total_global_handles_size: usize,
    pub used_global_handles_size: usize,
    pub used_heap_size: usize,
    pub malloced_memory: usize,
    pub external_memory: usize,
    pub peak_malloced_memory: usize,
}

/// Facts a test can read back after `create`.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerInfo {
    pub service_path: String,
    pub memory_limit_mb: u64,
    pub worker_timeout_ms: u64,
    pub retired: bool,
    pub in_flight: usize,
}

/// Why `op_user_worker_fetch_build` rejected the request.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum BuildError {
    /// `Method::from_bytes` failed.
    #[error("invalid HTTP method: {0}")]
    Method(String),
}

/// Why `op_user_worker_fetch_send` failed. `Display` is the string `main`
/// puts in `{ msg }` (`Error.prototype.toString` of the custom error).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum SendError {
    /// `WorkerError::RequestCancelledBySupervisor`.
    #[error("WorkerRequestCancelled: request has been cancelled by supervisor")]
    Cancelled,
    /// `WorkerError::WorkerAlreadyRetired`.
    #[error(
        "WorkerAlreadyRetired: request cannot be handled because the worker has already retired"
    )]
    Retired,
    /// `WorkerError::RequestIdleTimeout`.
    #[error("WorkerRequestIdleTimeout: request timed out")]
    IdleTimeout,
    /// Any other failure, including an unknown key.
    #[error("InvalidWorkerResponse: {0}")]
    Invalid(String),
}

/// Supervisor outcomes the `fetch_send` match turns into [`SendError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SupervisorReject {
    /// The supervisor cancelled the in-flight request.
    ///
    /// A Deno child has no supervisor cancel, so production code does not
    /// construct this. `fetch_send` still maps it.
    #[allow(dead_code)]
    Cancelled,
    /// The worker was retired before the request was accepted.
    Retired,
    /// The request sat idle past the supervisor timeout.
    ///
    /// A Deno child has no per-request idle timeout, so production code does
    /// not construct this. `fetch_send` still maps it.
    #[allow(dead_code)]
    IdleTimeout,
    /// A transport or pool failure with this message.
    Other(String),
}

/// `op_user_worker_fetch_send`'s error match (`ext/workers/lib.rs:633`).
pub(crate) fn map_supervisor_reject(reason: SupervisorReject) -> SendError {
    match reason {
        SupervisorReject::Cancelled => SendError::Cancelled,
        SupervisorReject::Retired => SendError::Retired,
        SupervisorReject::IdleTimeout => SendError::IdleTimeout,
        SupervisorReject::Other(message) => SendError::Invalid(message),
    }
}

/// Headers and target produced by `op_user_worker_fetch_build`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuiltRequest {
    /// Parsed method.
    pub method: Method,
    /// The URL unchanged.
    pub url: String,
    /// Headers after the op's rewrites.
    pub headers: HeaderMap,
}

/// Inputs of `op_user_worker_fetch_build`.
pub(crate) struct FetchBuildInput<'a> {
    /// Method bytes (`POST`, `GET`, …).
    pub method: &'a [u8],
    /// Request URL or request-target.
    pub url: &'a str,
    /// Header name and value bytes, in order. Empty names are skipped.
    pub headers: &'a [(Vec<u8>, Vec<u8>)],
    /// Whether the JS request has a body.
    pub has_body: bool,
}

// megabase:unit functions:op:op_user_worker_fetch_build
/// Build the request `UserWorker.fetch` hands to `fetch_send`.
///
/// # Errors
///
/// [`BuildError::Method`] when `method` is not an HTTP method.
pub(crate) fn fetch_build(input: &FetchBuildInput<'_>) -> Result<BuiltRequest, BuildError> {
    let method = Method::from_bytes(input.method)
        .map_err(|_| BuildError::Method(String::from_utf8_lossy(input.method).into_owned()))?;
    let mut headers = HeaderMap::new();
    let zero_length = !input.has_body && matches!(method, Method::POST | Method::PUT);
    for (name, value) in input.headers {
        if name.is_empty() {
            continue;
        }
        let Ok(name) = HeaderName::from_bytes(name) else {
            continue;
        };
        let mut value = HeaderValue::from_bytes(value).unwrap_or(HeaderValue::from_static(""));
        if zero_length && name == header::CONTENT_LENGTH {
            value = HeaderValue::from_static("0");
        }
        headers.append(name, value);
    }
    Ok(BuiltRequest {
        method,
        url: input.url.to_owned(),
        headers,
    })
}

/// Retired flag and in-flight count. `fetch_send` and idle-ack share this
/// lock so a request that has passed the retired check cannot be cleaned up.
struct Activity {
    retired: bool,
    removed: bool,
    in_flight: usize,
    early_drop_pending: bool,
}

/// Test seam: `begin_request` waits here while it still holds [`Activity`],
/// after the retired check and before `in_flight` is incremented.
#[cfg(test)]
struct EnterGap {
    started: Arc<std::sync::Barrier>,
    release: Arc<std::sync::Barrier>,
}

struct Slot {
    key: Uuid,
    pool_key: String,
    service_path: String,
    addr: std::net::SocketAddr,
    memory_limit_mb: u64,
    worker_timeout_ms: u64,
    created_ms: u64,
    /// `None` when the wall clock is disabled (`workerTimeoutMs == 0`).
    limit: Option<Duration>,
    activity: Mutex<Activity>,
    handle: tokio::sync::Mutex<Option<WorkerHandle>>,
    #[cfg(test)]
    enter_gap: Mutex<Option<EnterGap>>,
}

struct Catalog {
    workers: HashMap<Uuid, Arc<Slot>>,
    by_pool: HashMap<String, Vec<Uuid>>,
}

struct PoolInner {
    dir: PathBuf,
    runtime: Box<dyn Runtime>,
    catalog: Mutex<Catalog>,
    gates: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Start of the monotonic clock. Production reads
    /// `origin.elapsed()`; tests store milliseconds in [`PoolInner::now_ms`].
    origin: tokio::time::Instant,
    now_ms: Option<Arc<AtomicU64>>,
}

/// The user-worker pool behind the five ops.
pub(crate) struct Pool {
    inner: Arc<PoolInner>,
}

impl Pool {
    pub(crate) fn new(dir: PathBuf, runtime: Box<dyn Runtime>) -> Self {
        Self::with_clock(dir, runtime, None)
    }

    fn with_clock(dir: PathBuf, runtime: Box<dyn Runtime>, now_ms: Option<Arc<AtomicU64>>) -> Self {
        Self {
            inner: Arc::new(PoolInner {
                dir,
                runtime,
                catalog: Mutex::new(Catalog {
                    workers: HashMap::new(),
                    by_pool: HashMap::new(),
                }),
                gates: Mutex::new(HashMap::new()),
                origin: tokio::time::Instant::now(),
                now_ms,
            }),
        }
    }

    /// Pool whose wall clock is `now_ms` instead of `Instant::elapsed`.
    #[cfg(test)]
    fn with_manual_clock(dir: PathBuf, runtime: Box<dyn Runtime>, now_ms: Arc<AtomicU64>) -> Self {
        Self::with_clock(dir, runtime, Some(now_ms))
    }

    /// `create` with the arguments the pinned `main` service passes for
    /// `name`.
    ///
    /// # Errors
    ///
    /// [`BootError`] when the worker cannot be started. The caller prefixes
    /// `InvalidWorkerCreation:`.
    pub(crate) async fn create_main(&self, name: &str) -> Result<Uuid, BootError> {
        let result = self
            .create(&CreateOptions {
                service_path: self.service_path(name),
                pool_key: None,
                force_create: false,
                memory_limit_mb: Some(MAIN_MEMORY_LIMIT_MB),
                worker_timeout_ms: Some(MAIN_WORKER_TIMEOUT_MS),
            })
            .await?;
        Ok(result.key)
    }

    fn service_path(&self, name: &str) -> String {
        self.inner.dir.join(name).to_string_lossy().into_owned()
    }

    // megabase:unit functions:op:op_user_worker_create
    /// Start or reuse a user worker.
    ///
    /// # Errors
    ///
    /// [`BootError`] when a new worker cannot be started.
    pub(crate) async fn create(&self, opts: &CreateOptions) -> Result<CreateResult, BootError> {
        let pool_key = opts
            .pool_key
            .clone()
            .unwrap_or_else(|| opts.service_path.clone());
        // Resolve the entrypoint before inserting a gate. A missing name
        // must not grow `gates` (the old `Workers::address` checked first).
        let name = function_name(&opts.service_path)?;
        let entry = entrypoint(&self.inner.dir, name)?;
        let gate = self.gate(&pool_key);
        let _hold = gate.lock().await;
        self.sweep().await;
        let reusable = if opts.force_create {
            None
        } else {
            self.find_reusable(&pool_key).await
        };
        if let Some(key) = reusable {
            return Ok(CreateResult { key, reused: true });
        }

        let handle = match self
            .inner
            .runtime
            .start(WorkerSpec {
                name,
                entrypoint: &entry,
            })
            .await
        {
            Ok(handle) => handle,
            Err(error) => {
                drop(_hold);
                drop(gate);
                self.inner.drop_gate_if_unused(&pool_key);
                return Err(error);
            }
        };
        let key = Uuid::new_v4();
        let memory_limit_mb = opts.memory_limit_mb.unwrap_or(DEFAULT_MEMORY_LIMIT_MB);
        let worker_timeout_ms = opts.worker_timeout_ms.unwrap_or(DEFAULT_WORKER_TIMEOUT_MS);
        let slot = Arc::new(Slot {
            key,
            pool_key: pool_key.clone(),
            service_path: opts.service_path.clone(),
            addr: handle.addr(),
            memory_limit_mb,
            worker_timeout_ms,
            created_ms: self.now_ms(),
            limit: wall_clock_limit(worker_timeout_ms),
            activity: Mutex::new(Activity {
                retired: false,
                removed: false,
                in_flight: 0,
                early_drop_pending: true,
            }),
            handle: tokio::sync::Mutex::new(Some(handle)),
            #[cfg(test)]
            enter_gap: Mutex::new(None),
        });
        {
            let mut catalog = self.catalog();
            catalog.workers.insert(key, Arc::clone(&slot));
            catalog.by_pool.entry(pool_key).or_default().push(key);
        }
        tracing::debug!(
            function = name,
            %key,
            memory_limit_mb = slot.memory_limit_mb,
            worker_timeout_ms = slot.worker_timeout_ms,
            "created user worker"
        );
        Ok(CreateResult { key, reused: false })
    }

    // megabase:unit functions:op:op_user_worker_fetch_send
    /// Send `request` to the worker `key` and stream its response.
    ///
    /// `target` is the request-target (`fetch_build`'s URL). The gateway
    /// passes the path after `/functions/v1`.
    ///
    /// # Errors
    ///
    /// [`SendError`] when the key is missing, the worker is retired, or the
    /// exchange fails.
    pub(crate) async fn fetch_send(
        &self,
        key: Uuid,
        request: Request,
        target: &str,
    ) -> Result<Response, SendError> {
        self.sweep().await;
        let Some(slot) = self.slot(key) else {
            return Err(map_supervisor_reject(SupervisorReject::Other(
                "user worker not available".to_owned(),
            )));
        };
        let _in_flight = begin_request(&slot)?;
        let addr = slot.addr;
        proxy::forward(addr, request, target)
            .await
            .map_err(|error| map_supervisor_reject(SupervisorReject::Other(error.to_string())))
    }

    // megabase:unit functions:op:op_user_worker_cleanup_idle_workers
    /// Ask every live worker to drop if it is idle. Returns how many
    /// acknowledged, then stops those workers.
    pub(crate) async fn cleanup_idle_workers(&self, timeout_ms: u64) -> usize {
        self.sweep().await;
        let slots: Vec<Arc<Slot>> = {
            let catalog = self.catalog();
            catalog
                .workers
                .values()
                .filter(|slot| !activity(slot).removed)
                .map(Arc::clone)
                .collect()
        };
        let limit = Duration::from_millis(timeout_ms);
        let mut doomed = Vec::new();
        for slot in slots {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let ack = acknowledge(&slot);
            let _ = tx.send(ack);
            let counted = matches!(tokio::time::timeout(limit, rx).await, Ok(Ok(true)));
            if counted {
                doomed.push(slot.key);
            }
        }
        let count = doomed.len();
        for key in doomed {
            self.shutdown(key).await;
        }
        count
    }

    // megabase:unit functions:op:op_user_worker_mem_stats
    /// Heap snapshot of every worker still in the pool. `stats` is `None`:
    /// a Deno process has no isolate sample (`mem_check.read()` failing).
    pub(crate) async fn mem_stats(&self) -> BTreeMap<String, MemStat> {
        self.sweep().await;
        let catalog = self.catalog();
        catalog
            .workers
            .values()
            .filter(|slot| !activity(slot).removed)
            .map(|slot| {
                (
                    slot.key.to_string(),
                    MemStat {
                        service_path: slot.service_path.clone(),
                        stats: None,
                    },
                )
            })
            .collect()
    }

    /// Recorded limits and flags for `key`, when the worker is still in the
    /// pool. Applies the wall clock first.
    #[cfg(test)]
    pub(crate) async fn worker_info(&self, key: Uuid) -> Option<WorkerInfo> {
        self.sweep().await;
        let slot = self.slot(key)?;
        let activity = activity(&slot);
        if activity.removed {
            return None;
        }
        Some(WorkerInfo {
            service_path: slot.service_path.clone(),
            memory_limit_mb: slot.memory_limit_mb,
            worker_timeout_ms: slot.worker_timeout_ms,
            retired: activity.retired,
            in_flight: activity.in_flight,
        })
    }

    #[cfg(test)]
    fn gate_count(&self) -> usize {
        self.inner
            .gates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    fn gate(&self, pool_key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut gates = self
            .inner
            .gates
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let gate = gates
            .entry(pool_key.to_owned())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())));
        Arc::clone(gate)
    }

    fn catalog(&self) -> std::sync::MutexGuard<'_, Catalog> {
        self.inner
            .catalog
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn slot(&self, key: Uuid) -> Option<Arc<Slot>> {
        self.catalog().workers.get(&key).map(Arc::clone)
    }

    async fn find_reusable(&self, pool_key: &str) -> Option<Uuid> {
        let slots: Vec<Arc<Slot>> = {
            let catalog = self.catalog();
            catalog
                .by_pool
                .get(pool_key)
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| catalog.workers.get(id).map(Arc::clone))
                        .collect()
                })
                .unwrap_or_default()
        };
        for slot in slots {
            let inactive = {
                let activity = activity(&slot);
                activity.removed || activity.retired
            };
            if inactive {
                continue;
            }
            let running = slot
                .handle
                .lock()
                .await
                .as_mut()
                .is_some_and(WorkerHandle::is_running);
            if running {
                return Some(slot.key);
            }
            self.shutdown(slot.key).await;
        }
        None
    }

    fn now_ms(&self) -> u64 {
        if let Some(manual) = &self.inner.now_ms {
            return manual.load(Ordering::Acquire);
        }
        u64::try_from(self.inner.origin.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Retire at half the wall-clock limit and stop at the full limit.
    /// Evaluated when the pool is used, at the same thresholds as the
    /// supervisor's two interval ticks after the immediate first tick.
    async fn sweep(&self) {
        let now = self.now_ms();
        let slots: Vec<Arc<Slot>> = {
            let catalog = self.catalog();
            catalog.workers.values().map(Arc::clone).collect()
        };
        for slot in slots {
            let Some(limit) = slot.limit else {
                continue;
            };
            let period = limit.checked_div(2).unwrap_or(Duration::from_millis(1));
            let full = period.saturating_add(period);
            let elapsed = Duration::from_millis(now.saturating_sub(slot.created_ms));
            let stop = {
                let mut activity = activity(&slot);
                if activity.removed {
                    false
                } else if elapsed >= full {
                    true
                } else {
                    if elapsed >= period {
                        activity.retired = true;
                    }
                    false
                }
            };
            if stop {
                self.shutdown(slot.key).await;
            }
        }
    }

    async fn shutdown(&self, key: Uuid) {
        self.inner.shutdown(key).await;
    }
}

impl PoolInner {
    async fn shutdown(&self, key: Uuid) {
        let (slot, empty, pool_key) = {
            let mut catalog = self.catalog.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(slot) = catalog.workers.remove(&key) else {
                return;
            };
            let pool_key = slot.pool_key.clone();
            if let Some(ids) = catalog.by_pool.get_mut(&pool_key) {
                ids.retain(|id| *id != key);
            }
            let empty = catalog
                .by_pool
                .get(&pool_key)
                .is_some_and(|ids| ids.is_empty());
            if empty {
                catalog.by_pool.remove(&pool_key);
            }
            (slot, empty, pool_key)
        };
        {
            let mut activity = activity(&slot);
            activity.removed = true;
            activity.retired = true;
        }
        if empty {
            self.drop_gate_if_unused(&pool_key);
        }
        let handle = slot.handle.lock().await.take();
        if let Some(handle) = handle {
            handle.stop().await;
        }
    }

    /// Remove `pool_key`'s gate when this map entry is the only owner.
    /// A `create` that already cloned the `Arc` keeps its lock; the next
    /// idle moment drops the entry.
    fn drop_gate_if_unused(&self, pool_key: &str) {
        let mut gates = self.gates.lock().unwrap_or_else(PoisonError::into_inner);
        let unused = gates
            .get(pool_key)
            .is_some_and(|gate| Arc::strong_count(gate) == 1);
        if unused {
            gates.remove(pool_key);
        }
    }
}

fn wall_clock_limit(timeout_ms: u64) -> Option<Duration> {
    if timeout_ms == 0 {
        return None;
    }
    let ms = if timeout_ms < 2 { 2 } else { timeout_ms };
    Some(Duration::from_millis(ms))
}

fn function_name(service_path: &str) -> Result<&str, BootError> {
    let name = Path::new(service_path)
        .file_name()
        .and_then(|segment| segment.to_str())
        .unwrap_or("");
    if name.is_empty() {
        return Err(BootError::NoEntrypoint);
    }
    Ok(name)
}

fn activity(slot: &Slot) -> std::sync::MutexGuard<'_, Activity> {
    slot.activity.lock().unwrap_or_else(PoisonError::into_inner)
}

fn acknowledge(slot: &Slot) -> bool {
    let mut activity = activity(slot);
    if activity.removed || activity.in_flight != 0 || !activity.early_drop_pending {
        return false;
    }
    activity.early_drop_pending = false;
    activity.retired = true;
    true
}

struct InFlight(Arc<Slot>);

/// Mark one request in flight, or refuse a missing or retired worker.
///
/// The retired check and the `in_flight` increment hold [`Activity`] together.
/// Idle cleanup takes the same lock, so it cannot ack a worker that has
/// already passed the retired check.
fn begin_request(slot: &Arc<Slot>) -> Result<InFlight, SendError> {
    let mut activity = activity(slot);
    if activity.removed {
        return Err(map_supervisor_reject(SupervisorReject::Other(
            "user worker not available".to_owned(),
        )));
    }
    if activity.retired {
        return Err(map_supervisor_reject(SupervisorReject::Retired));
    }
    #[cfg(test)]
    {
        let gap = slot
            .enter_gap
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(gap) = gap {
            gap.started.wait();
            gap.release.wait();
        }
    }
    activity.in_flight += 1;
    Ok(InFlight(Arc::clone(slot)))
}

impl Drop for InFlight {
    fn drop(&mut self) {
        let mut activity = activity(&self.0);
        activity.in_flight = activity.in_flight.saturating_sub(1);
    }
}

/// Header pairs from a request map, in iteration order.
///
/// Values stay as raw bytes. `HeaderValue::to_str` rejects obs-text
/// (`0x80`–`0xFF`), which is legal in a header value, so the bytes are
/// copied instead of being replaced with an empty string.
pub(crate) fn header_pairs(headers: &HeaderMap) -> Vec<(Vec<u8>, Vec<u8>)> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str().as_bytes().to_vec(), value.as_bytes().to_vec()))
        .collect()
}

/// `true` when the HTTP request carries a body entity.
pub(crate) fn request_has_body(headers: &HeaderMap) -> bool {
    if let Some(len) = headers.get(header::CONTENT_LENGTH) {
        return len != "0";
    }
    headers.contains_key(header::TRANSFER_ENCODING)
}

impl From<BuildError> for ProxyError {
    fn from(error: BuildError) -> Self {
        ProxyError::Build(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize as StdAtomicUsize;

    use axum::body::Body;
    use axum::http::Request as HttpRequest;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "megabase-user-workers-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("hello")).unwrap();
        std::fs::write(dir.join("hello/index.ts"), "// fixture\n").unwrap();
        std::fs::create_dir_all(dir.join("other")).unwrap();
        std::fs::write(dir.join("other/index.ts"), "// fixture\n").unwrap();
        dir
    }

    struct FixedRuntime {
        addr: std::net::SocketAddr,
        starts: Arc<StdAtomicUsize>,
    }

    impl Runtime for FixedRuntime {
        fn start<'a>(&'a self, _spec: WorkerSpec<'a>) -> crate::worker::StartFuture<'a> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            let addr = self.addr;
            Box::pin(async move { Ok(WorkerHandle::external(addr)) })
        }
    }

    fn runtime() -> (FixedRuntime, Arc<StdAtomicUsize>) {
        let starts = Arc::new(StdAtomicUsize::new(0));
        (
            FixedRuntime {
                addr: std::net::SocketAddr::from(([127, 0, 0, 1], 9)),
                starts: Arc::clone(&starts),
            },
            starts,
        )
    }

    fn opts(service_path: &str) -> CreateOptions {
        CreateOptions {
            service_path: service_path.to_owned(),
            pool_key: None,
            force_create: false,
            memory_limit_mb: Some(150),
            worker_timeout_ms: Some(0),
        }
    }

    fn empty_request() -> Request {
        HttpRequest::builder().body(Body::empty()).unwrap()
    }

    fn header_list(pairs: &[(&str, &str)]) -> Vec<(Vec<u8>, Vec<u8>)> {
        pairs
            .iter()
            .map(|(name, value)| (name.as_bytes().to_vec(), value.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn fetch_build_zeros_content_length_for_post_and_put_without_a_body() {
        let headers = header_list(&[
            ("content-length", "12"),
            ("x-bad", "a\nb"),
            ("", "skip"),
            ("not a header", "skip"),
            ("authorization", "Bearer t"),
        ]);
        let built = fetch_build(&FetchBuildInput {
            method: b"POST",
            url: "/hello?a=1",
            headers: &headers,
            has_body: false,
        })
        .unwrap();
        assert_eq!(built.method, Method::POST);
        assert_eq!(built.url, "/hello?a=1");
        assert_eq!(built.headers[header::CONTENT_LENGTH], "0");
        assert_eq!(built.headers["x-bad"], "");
        assert_eq!(built.headers[header::AUTHORIZATION], "Bearer t");
        assert!(!built.headers.contains_key("not a header"));

        let put_headers = header_list(&[("Content-Length", "4")]);
        let put = fetch_build(&FetchBuildInput {
            method: b"PUT",
            url: "/hello",
            headers: &put_headers,
            has_body: false,
        })
        .unwrap();
        assert_eq!(put.headers[header::CONTENT_LENGTH], "0");

        let kept_headers = header_list(&[("content-length", "4")]);
        let kept = fetch_build(&FetchBuildInput {
            method: b"POST",
            url: "/hello",
            headers: &kept_headers,
            has_body: true,
        })
        .unwrap();
        assert_eq!(kept.headers[header::CONTENT_LENGTH], "4");

        let get_headers = header_list(&[("content-length", "4")]);
        let get = fetch_build(&FetchBuildInput {
            method: b"GET",
            url: "/hello",
            headers: &get_headers,
            has_body: false,
        })
        .unwrap();
        assert_eq!(get.headers[header::CONTENT_LENGTH], "4");
    }

    #[test]
    fn fetch_build_keeps_obs_text_header_bytes() {
        let mut headers = HeaderMap::new();
        headers.append(
            HeaderName::from_static("x-obs"),
            HeaderValue::from_bytes(&[b'a', 0xff, b'b']).unwrap(),
        );
        let pairs = header_pairs(&headers);
        let built = fetch_build(&FetchBuildInput {
            method: b"GET",
            url: "/hello",
            headers: &pairs,
            has_body: false,
        })
        .unwrap();
        assert_eq!(built.headers["x-obs"].as_bytes(), &[b'a', 0xff, b'b']);
    }

    #[test]
    fn fetch_build_rejects_an_invalid_method() {
        let error = fetch_build(&FetchBuildInput {
            method: b"NO PE",
            url: "/",
            headers: &[],
            has_body: false,
        })
        .unwrap_err();
        assert_eq!(error, BuildError::Method("NO PE".into()));
    }

    #[test]
    fn supervisor_rejects_use_the_upstream_error_strings() {
        assert_eq!(
            map_supervisor_reject(SupervisorReject::Cancelled).to_string(),
            "WorkerRequestCancelled: request has been cancelled by supervisor"
        );
        assert_eq!(
            map_supervisor_reject(SupervisorReject::Retired).to_string(),
            "WorkerAlreadyRetired: request cannot be handled because the worker has already retired"
        );
        assert_eq!(
            map_supervisor_reject(SupervisorReject::IdleTimeout).to_string(),
            "WorkerRequestIdleTimeout: request timed out"
        );
        assert_eq!(
            map_supervisor_reject(SupervisorReject::Other("user worker not available".into()))
                .to_string(),
            "InvalidWorkerResponse: user worker not available"
        );
    }

    #[test]
    fn heap_statistics_default_to_zero() {
        let stats = HeapStatistics::default();
        assert_eq!(stats.used_heap_size, 0);
        assert_eq!(stats.external_memory, 0);
    }

    #[tokio::test]
    async fn create_reuses_a_live_worker_and_records_main_limits() {
        let dir = temp_dir("reuse");
        let (runtime, starts) = runtime();
        let pool = Pool::new(dir.clone(), Box::new(runtime));
        let key = pool.create_main("hello").await.unwrap();
        let info = pool.worker_info(key).await.unwrap();
        assert_eq!(info.memory_limit_mb, 150);
        assert_eq!(info.worker_timeout_ms, 60_000);
        assert!(info.service_path.ends_with("hello"));
        assert!(!info.retired);

        let again = pool.create_main("hello").await.unwrap();
        assert_eq!(again, key);
        assert_eq!(starts.load(Ordering::SeqCst), 1);

        let other = pool.create_main("other").await.unwrap();
        assert_ne!(other, key);
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        let stats = pool.mem_stats().await;
        assert_eq!(stats.len(), 2);
        assert!(stats[&key.to_string()].stats.is_none());
        assert_eq!(
            stats[&key.to_string()].service_path,
            dir.join("hello").to_string_lossy()
        );
    }

    #[tokio::test]
    async fn omitted_limits_use_the_edge_runtime_defaults() {
        let dir = temp_dir("defaults");
        let (runtime, _) = runtime();
        let pool = Pool::new(dir, Box::new(runtime));
        let mut options = opts("hello");
        options.memory_limit_mb = None;
        options.worker_timeout_ms = None;
        let created = pool.create(&options).await.unwrap();
        assert!(!created.reused);
        let info = pool.worker_info(created.key).await.unwrap();
        assert_eq!(info.memory_limit_mb, 256);
        assert_eq!(info.worker_timeout_ms, 400_000);
    }

    #[tokio::test]
    async fn force_create_keeps_the_previous_worker_addressable() {
        let dir = temp_dir("force");
        let (runtime, starts) = runtime();
        let pool = Pool::new(dir, Box::new(runtime));
        let first = pool.create(&opts("hello")).await.unwrap();
        let mut forced = opts("hello");
        forced.force_create = true;
        let second = pool.create(&forced).await.unwrap();
        assert!(!second.reused);
        assert_ne!(first.key, second.key);
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        let third = pool.create(&opts("hello")).await.unwrap();
        assert!(third.reused);
        assert_eq!(third.key, first.key);
        assert_eq!(pool.mem_stats().await.len(), 2);
    }

    #[tokio::test]
    async fn a_pool_key_reuses_across_service_paths() {
        let dir = temp_dir("pool-key");
        let (runtime, starts) = runtime();
        let pool = Pool::new(dir, Box::new(runtime));
        let mut first = opts("hello");
        first.pool_key = Some("shared".into());
        let created = pool.create(&first).await.unwrap();
        let mut second = opts("other");
        second.pool_key = Some("shared".into());
        let reused = pool.create(&second).await.unwrap();
        assert!(reused.reused);
        assert_eq!(reused.key, created.key);
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_missing_entrypoint_does_not_start_a_worker() {
        let dir = temp_dir("missing");
        let (runtime, starts) = runtime();
        let pool = Pool::new(dir, Box::new(runtime));
        let error = pool.create(&opts("nope")).await.unwrap_err();
        assert!(matches!(error, BootError::NoEntrypoint));
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        assert!(pool.mem_stats().await.is_empty());
        assert_eq!(pool.gate_count(), 0);
        for index in 0..8 {
            let error = pool
                .create(&opts(&format!("missing-{index}")))
                .await
                .unwrap_err();
            assert!(matches!(error, BootError::NoEntrypoint));
        }
        assert_eq!(pool.gate_count(), 0);
        let error = pool.create(&opts("")).await.unwrap_err();
        assert!(matches!(error, BootError::NoEntrypoint));
        assert_eq!(pool.gate_count(), 0);
    }

    #[tokio::test]
    async fn the_gate_is_dropped_when_the_last_worker_stops() {
        let dir = temp_dir("gate");
        let (runtime, _) = runtime();
        let pool = Pool::new(dir, Box::new(runtime));
        let created = pool.create(&opts("hello")).await.unwrap();
        assert_eq!(pool.gate_count(), 1);
        assert!(pool.worker_info(created.key).await.is_some());
        assert_eq!(pool.cleanup_idle_workers(0).await, 1);
        assert!(pool.worker_info(created.key).await.is_none());
        assert_eq!(pool.gate_count(), 0);
    }

    #[test]
    fn cleanup_cannot_ack_a_request_that_passed_the_retired_check() {
        let started = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        let slot = Arc::new(Slot {
            key: Uuid::nil(),
            pool_key: "hello".into(),
            service_path: "hello".into(),
            addr: std::net::SocketAddr::from(([127, 0, 0, 1], 9)),
            memory_limit_mb: 150,
            worker_timeout_ms: 0,
            created_ms: 0,
            limit: None,
            activity: Mutex::new(Activity {
                retired: false,
                removed: false,
                in_flight: 0,
                early_drop_pending: true,
            }),
            handle: tokio::sync::Mutex::new(None),
            enter_gap: Mutex::new(Some(EnterGap {
                started: Arc::clone(&started),
                release: Arc::clone(&release),
            })),
        });

        let fetching = Arc::clone(&slot);
        let fetcher = std::thread::spawn(move || begin_request(&fetching));
        started.wait();

        let acking = Arc::clone(&slot);
        let acker = std::thread::spawn(move || acknowledge(&acking));
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !acker.is_finished(),
            "idle-ack ran between the retired check and the in_flight increment"
        );

        release.wait();
        let in_flight = fetcher.join().unwrap().unwrap();
        assert!(!acker.join().unwrap());
        assert_eq!(activity(&slot).in_flight, 1);
        assert!(!activity(&slot).retired);
        drop(in_flight);
        assert_eq!(activity(&slot).in_flight, 0);
        assert!(acknowledge(&slot));
    }

    #[tokio::test]
    async fn fetch_send_reports_a_missing_key_and_keeps_a_failed_worker() {
        let dir = temp_dir("fetch");
        let (runtime, _) = runtime();
        let pool = Pool::new(dir, Box::new(runtime));
        let missing = pool
            .fetch_send(Uuid::new_v4(), empty_request(), "/hello")
            .await
            .unwrap_err();
        assert_eq!(
            missing.to_string(),
            "InvalidWorkerResponse: user worker not available"
        );

        let created = pool.create(&opts("hello")).await.unwrap();
        let failed = pool
            .fetch_send(created.key, empty_request(), "/hello")
            .await
            .unwrap_err();
        assert!(
            failed.to_string().starts_with("InvalidWorkerResponse:"),
            "{failed}"
        );
        assert!(pool.worker_info(created.key).await.is_some());
        assert_eq!(pool.worker_info(created.key).await.unwrap().in_flight, 0);
        let again = pool.create(&opts("hello")).await.unwrap();
        assert!(again.reused);
        assert_eq!(again.key, created.key);
    }

    #[tokio::test]
    async fn cleanup_stops_idle_workers_and_spares_a_busy_one() {
        let dir = temp_dir("cleanup");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let starts = Arc::new(StdAtomicUsize::new(0));
        let runtime = FixedRuntime {
            addr,
            starts: Arc::clone(&starts),
        };
        let pool = Pool::new(dir, Box::new(runtime));
        let idle = pool.create(&opts("hello")).await.unwrap().key;
        let mut busy_opts = opts("other");
        busy_opts.pool_key = Some("busy".into());
        let busy = pool.create(&busy_opts).await.unwrap().key;

        let pool = Arc::new(pool);
        let sending = Arc::clone(&pool);
        let pending =
            tokio::spawn(async move { sending.fetch_send(busy, empty_request(), "/other").await });
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if pool.worker_info(busy).await.unwrap().in_flight == 1 {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "fetch did not start");
            tokio::task::yield_now().await;
        }

        assert_eq!(pool.cleanup_idle_workers(0).await, 1);
        assert!(pool.worker_info(idle).await.is_none());
        assert!(pool.worker_info(busy).await.is_some());
        assert_eq!(pool.mem_stats().await.len(), 1);

        drop(listener);
        let _ = pending.await;
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if pool.worker_info(busy).await.unwrap().in_flight == 0 {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "fetch did not finish");
            tokio::task::yield_now().await;
        }
        assert_eq!(pool.cleanup_idle_workers(5).await, 1);
        assert!(pool.mem_stats().await.is_empty());
        assert_eq!(pool.cleanup_idle_workers(5).await, 0);
    }

    #[tokio::test]
    async fn wall_clock_retires_at_half_and_stops_at_the_limit() {
        let dir = temp_dir("wall");
        let (runtime, _) = runtime();
        let clock = Arc::new(AtomicU64::new(0));
        let pool = Pool::with_manual_clock(dir, Box::new(runtime), Arc::clone(&clock));
        let mut options = opts("hello");
        options.worker_timeout_ms = Some(1_000);
        let created = pool.create(&options).await.unwrap();

        clock.store(499, Ordering::Release);
        let reused = pool.create(&options).await.unwrap();
        assert!(reused.reused);
        assert_eq!(reused.key, created.key);

        clock.store(500, Ordering::Release);
        let error = pool
            .fetch_send(created.key, empty_request(), "/hello")
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "WorkerAlreadyRetired: request cannot be handled because the worker has already retired"
        );
        let replacement = pool.create(&options).await.unwrap();
        assert!(!replacement.reused);
        assert_ne!(replacement.key, created.key);

        clock.store(1_000, Ordering::Release);
        let stats = pool.mem_stats().await;
        assert!(!stats.contains_key(&created.key.to_string()));
        assert!(stats.contains_key(&replacement.key.to_string()));
        assert!(pool.worker_info(replacement.key).await.unwrap().retired);
    }

    #[tokio::test]
    async fn a_zero_timeout_does_not_retire_and_a_one_ms_timeout_is_two_ms() {
        let dir = temp_dir("zero");
        let (runtime, _) = runtime();
        let clock = Arc::new(AtomicU64::new(0));
        let pool = Pool::with_manual_clock(dir, Box::new(runtime), Arc::clone(&clock));
        let created = pool.create(&opts("hello")).await.unwrap();
        clock.store(3_600_000, Ordering::Release);
        let reused = pool.create(&opts("hello")).await.unwrap();
        assert!(reused.reused);
        assert_eq!(reused.key, created.key);

        let mut tiny = opts("other");
        tiny.worker_timeout_ms = Some(1);
        let short = pool.create(&tiny).await.unwrap();
        clock.store(clock.load(Ordering::Acquire) + 1, Ordering::Release);
        assert!(pool.worker_info(short.key).await.unwrap().retired);
        clock.store(clock.load(Ordering::Acquire) + 1, Ordering::Release);
        assert!(pool.worker_info(short.key).await.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_exited_process_is_replaced() {
        let dir = temp_dir("exited");
        let starts = Arc::new(StdAtomicUsize::new(0));
        let runtime = ExitedRuntime {
            starts: Arc::clone(&starts),
        };
        let pool = Pool::new(dir, Box::new(runtime));
        let first = pool.create(&opts("hello")).await.unwrap();
        let second = pool.create(&opts("hello")).await.unwrap();
        assert!(!second.reused);
        assert_ne!(first.key, second.key);
        assert!(pool.worker_info(first.key).await.is_none());
        assert_eq!(starts.load(Ordering::SeqCst), 2);
    }

    #[cfg(unix)]
    struct ExitedRuntime {
        starts: Arc<StdAtomicUsize>,
    }

    #[cfg(unix)]
    impl Runtime for ExitedRuntime {
        fn start<'a>(&'a self, _spec: WorkerSpec<'a>) -> crate::worker::StartFuture<'a> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                let mut child = tokio::process::Command::new("true").spawn()?;
                let _ = child.wait().await?;
                Ok(WorkerHandle::process(
                    std::net::SocketAddr::from(([127, 0, 0, 1], 9)),
                    child,
                ))
            })
        }
    }
}
