# Functions: user workers (issue 78)

Status: draft
Level: 4
Unit ids (`coverage/units.json`): `functions:op:op_user_worker_cleanup_idle_workers`,
`functions:op:op_user_worker_create`, `functions:op:op_user_worker_fetch_build`,
`functions:op:op_user_worker_fetch_send`, `functions:op:op_user_worker_mem_stats`.

These five ops are the `user_workers` extension. Only the **main** worker
installs them, as `EdgeRuntime.userWorkers`
(`ext/runtime/js/namespaces.js:28`–`:31`). The pinned `main` service calls
`create` then `fetch` for every `/functions/v1/{name}` request. It does not
call `tryCleanupIdleWorkers` or `memStats`.

Megabase has no Deno main worker. The functions gateway is that main
worker: `Workers::create_main` and `Workers::fetch` run the same two calls,
and the pool implements cleanup and memory stats for the same op contracts.
Spec inputs below are the op contracts. The gateway passes the arguments
`main` passes.

## Upstream

Pin edge-runtime `v1.77.4`, supabase `v1.26.08`.

| Behavior | File:line |
|---|---|
| Ops registered on the extension | `vendor/edge-runtime/ext/workers/lib.rs:73`–`:80` |
| `op_user_worker_create` | `vendor/edge-runtime/ext/workers/lib.rs:195`–`:321` |
| `op_user_worker_fetch_build` | `vendor/edge-runtime/ext/workers/lib.rs:483`–`:532` |
| `op_user_worker_fetch_send` | `vendor/edge-runtime/ext/workers/lib.rs:534`–`:694` |
| `op_user_worker_cleanup_idle_workers` | `vendor/edge-runtime/ext/workers/lib.rs:696`–`:718` |
| `op_user_worker_mem_stats` | `vendor/edge-runtime/ext/workers/lib.rs:720`–`:736` |
| JS `UserWorker.create` / `fetch` / `tryCleanupIdleWorkers` / `memStats` | `vendor/edge-runtime/ext/workers/user_workers.js:42`–`:183` |
| Worker error strings | `vendor/edge-runtime/ext/workers/errors.rs:4`–`:10` |
| Reuse, `force_create`, pool key, shutdown, cleanup, memory map | `vendor/edge-runtime/crates/base/src/worker/pool.rs:277`–`:800` |
| Wall-clock retire at half the limit, stop at the limit; early-drop ack | `vendor/edge-runtime/crates/base/src/worker/supervisor/strategy_per_worker.rs:185`–`:482` |
| Heap stat fields | `vendor/edge-runtime/crates/base_mem_check/src/lib.rs:5`–`:25` |
| Default limits (`256` MiB, `400000` ms) | `vendor/edge-runtime/.cargo/config.toml:4`–`:8` |
| `main` calls `create` with `servicePath`, `memoryLimitMb: 150`, `workerTimeoutMs: 60000`, `noModuleCache: false`, `envVars` | `vendor/supabase/docker/volumes/functions/main/index.ts:145`–`:164` |

`crates/megabase-functions/src/user_workers.rs` carries the port header and
the five unit markers. `fetch_build` is applied by
`crates/megabase-functions/src/proxy.rs` before the request is written.

## `op_user_worker_create`

Inputs (`UserWorkerCreateOptions`, camelCase): `servicePath`, optional
`poolKey`, `forceCreate` (default false), optional `memoryLimitMb`,
optional `workerTimeoutMs`. `main` also sends `noModuleCache: false` and
`envVars` (the process environment). Eszip, permissions, CPU limits, S3 and
OTel are absent on that call.

1. Pool key is `poolKey` when set, otherwise `servicePath`
   (`pool.rs:289`–`:292`).
2. When `forceCreate` is false and that key has a worker that is not
   retired and whose process is still running, the result is
   `(key, reused: true)` (`pool.rs:301`–`:313`). The key is a hyphenated
   lowercase UUID (`Uuid::new_v4`, `pool.rs:460`).
3. Otherwise a new worker is started and the result is `(key, reused: false)`.
4. `memoryLimitMb` defaults to `256`. `workerTimeoutMs` defaults to
   `400000`. `main` passes `150` and `60000`. The limit is stored with the
   worker. Megabase does not kill the process from a V8 heap sample; there
   is no isolate to read.
5. `workerTimeoutMs == 0` disables the wall clock. Any other value below
   `2` is raised to `2` (`strategy_per_worker.rs:185`–`:188`). The supervisor
   retires the worker after half the limit (the second interval tick; the
   first tick is immediate and does not retire) and stops it after the full
   limit (`:422`–`:445`). Megabase applies those same thresholds when the
   pool next runs `create`, `fetch_send`, `cleanup_idle_workers`, or
   `mem_stats`, rather than on a background interval. A retired worker is
   not reused. `fetch` of its key fails with `WorkerAlreadyRetired`. A worker
   past the full limit is already gone, so `fetch` reports
   `user worker not available`.
6. Boot failures surface as `InvalidWorkerCreation: {message}`. A missing
   `index.*` uses the existing entrypoint error. The gateway adds the
   `InvalidWorkerCreation:` prefix, as `main`'s `e.toString()` does.
7. The entrypoint is `{functions_dir}/{final path segment}`. `main`'s
   `servicePath` is `/home/deno/functions/{name}`; the gateway passes
   `{functions_dir}/{name}`. The segment is the same check as
   `worker::entrypoint` (`.` / `..` / `\` / NUL have no entrypoint).

`noModuleCache: false` is Deno's default module cache (`deno run` without
`--reload`). `envVars` are the variables the process was started with
(`DenoRuntime`), which is the map `main` copies from its own environment.

JS `UserWorker.create` throws `TypeError: service path must be defined`
when `servicePath` is empty and there is no eszip
(`user_workers.js:146`–`:148`). That check is not this op. `main` returns
400 for an empty name before `create`, and the gateway does too.

## `op_user_worker_fetch_build`

Input: `method` (bytes), `url` (string), `headers` (list of pairs),
`hasBody` (bool). Output: the method, the same URL, and the header list
(`lib.rs:483`–`:532`).

- An empty header name is skipped.
- A header value that is not a legal HTTP value becomes an empty value
  (`HeaderValue::from_bytes` failing, then `HeaderValue::from_static("")`).
  Bytes that `HeaderValue` accepts, including obs-text (`0x80`–`0xFF`),
  are forwarded unchanged.
- A header name that is not a legal HTTP name is skipped. The op
  `unwrap`s and panics; Megabase does not panic on a request.
- When `hasBody` is false, the method is `POST` or `PUT`, and a
  `content-length` header is present, that value is `0`.

The gateway sets `hasBody` from the incoming request: false when
`content-length` is `0` or absent and there is no `transfer-encoding`,
otherwise true. It then sends the built headers to the worker with
request-target `url` and `Host: functions:9000` (see
`specs/functions/invoke.md`).

## `op_user_worker_fetch_send`

Input: worker `key`, and the request built above. Output: the worker's
status, canonical reason phrase, headers and body (`lib.rs:627`–`:693`).
The gateway streams that response back. Null-body and redirect handling
stays in `proxy.rs` (the JS `UserWorker.fetch` wrapper, not this op).

Errors, as `e.toString()` on the custom error (`lib.rs:633`–`:649`,
`errors.rs`):

| Condition | Message |
|---|---|
| Key missing, or the worker was already shut down | `InvalidWorkerResponse: user worker not available` |
| Key present and retired | `WorkerAlreadyRetired: request cannot be handled because the worker has already retired` |
| Supervisor cancelled the request | `WorkerRequestCancelled: request has been cancelled by supervisor` |
| Supervisor idle timeout | `WorkerRequestIdleTimeout: request timed out` |
| The HTTP exchange failed | `InvalidWorkerResponse: {transport error}` |

The Deno process has no supervisor cancel or per-request idle timeout, so
the gateway does not raise those two. `fetch_send` still maps them, so a
later supervisor can return the same strings. A transport error does not
drop the worker; the next `create` reuses it while the process is running.
A process that has exited is shut down and the next `create` starts another.

`in_flight` counts requests inside `fetch_send`. It stays up until the
client finishes or drops the streamed body, and it drops immediately when
the send fails before a response is built. Idle cleanup must not stop a
worker while a body is still being read.

## `op_user_worker_cleanup_idle_workers`

Input: `timeoutMs` (`usize`; Megabase takes `u64` milliseconds). Output: how
many workers acknowledged early drop (`lib.rs:696`–`:718`,
`strategy_per_worker.rs:472`–`:482`).

A worker acknowledges when it has no in-flight request and has not already
acknowledged. The retired check and the in-flight increment in `fetch_send`
share one lock with that ack, so a request that has passed the retired
check cannot be cleaned up. The ack is sent before the process is stopped,
so a `timeoutMs` of `0` still counts an ack that is already ready (the
oneshot is sent, then waited on). A busy worker is not counted and is not
stopped. Each acknowledged worker is then retired and stopped, and leaves
the pool. Workers that were already removed are not part of the scan.

`main` never calls this. The gateway exposes it on the pool.

## `op_user_worker_mem_stats`

Output: a map of worker key → `{ servicePath, stats }` for every worker
still in the pool, including one that is retired but not yet stopped
(`lib.rs:720`–`:736`, `pool.rs:776`–`:799`). `stats` is the V8 heap sample,
or absent when `mem_check.read()` fails.

Megabase workers are OS processes, not isolates the gateway can sample, so
`stats` is absent for every entry. Keys and `servicePath` match the workers
`create` returned and cleanup has not removed.

## Judge cases

None. `create` and `fetch` are what a client sees as
`/functions/v1/{name}`, already covered as `functions:route:*`. The vendored
`hello` function does not call `EdgeRuntime.userWorkers`, and that namespace
is not installed in a user worker. A fixture that called `tryCleanupIdleWorkers`
or `memStats` would be TypeScript outside `vendor/`. No protected judge file
is added.

## Tests

`crates/megabase-functions/src/user_workers.rs` and the invoke tests in
`crates/megabase-functions/tests/invoke.rs`.
