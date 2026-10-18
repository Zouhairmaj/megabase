# Functions: runtime ops (issue 77, PRs 1 and 2 of 3)

Status: draft
Level: 4
Unit ids (`coverage/units.json`), PR 1: `functions:op:op_bootstrap_color_depth`,
`op_bootstrap_unstable_args`, `op_cancel_drop_token`,
`op_check_outbound_rate_limit`, `op_console_size`, `op_http_start`,
`op_http_upgrade_raw2`, `op_http_upgrade_raw2_fence`,
`op_http_upgrade_websocket2`, `op_is_runtime_init`.

These are Rust ops of the edge-runtime `runtime` extension. Edge-runtime
embeds its own `deno_core`; its user-worker JavaScript calls the ops
directly. Megabase runs a function as a plain `deno run` child process
(`specs/functions/invoke.md`), which has no such ops. A unit is served only
when a function can observe the same behavior, and the code that serves it
carries the marker. No unit of PR 1 has a marker yet: the table says why.

## Upstream and Megabase equivalent

Pin edge-runtime `v1.77.4`.

| Unit | Upstream behavior (file:line) | Observable by a function? | Megabase |
|---|---|---|---|
| `op_bootstrap_color_depth` | returns `1` (`ext/runtime/ops/bootstrap.rs:49`); used by `node:tty` `getColorDepth()` (`ext/node/polyfills/tty.js:136`) | Only through `node:tty` on a `WriteStream`; a worker has no TTY | Not served. Deno's own `node:tty` answers (not measured against the reference) |
| `op_bootstrap_unstable_args` | returns `[]` (`ext/runtime/lib.rs:471`); spread into `child_process` fork args (`ext/node/polyfills/child_process.ts:148`) | No: user workers cannot spawn | Not served |
| `op_cancel_drop_token` | cancels a `DropToken` resource (`ext/runtime/lib.rs:435`); called by `ext/runtime/js/namespaces.js:24` when the worker is terminated | No: internal termination plumbing | Not served. Megabase ends a worker by killing the process |
| `op_check_outbound_rate_limit` | `u32::MAX` unless a `TraceRateLimiter` is in state (`ext/runtime/lib.rs:477`); called by the Node `http` client (`ext/node/polyfills/http.ts:501`) | Never limits without a limiter, and no limiter is configured for user workers in the pinned `main` | Not served; the result `u32::MAX` means "allowed", which is what Deno does |
| `op_console_size` | writes nothing (`ext/runtime/lib.rs:295`); `Deno.consoleSize()` is `{columns: 80, rows: 24}` (`ext/os/os.js:46`) | Yes: `Deno.consoleSize()` | `crate::runtime::SOURCE` installs the same fixed size. Not marked: the op itself is the no-op, the observable value comes from `os.js` |
| `op_http_start` | turns an in-memory duplex stream into an HTTP connection with fake peer `0.0.0.0:9999` (`ext/runtime/ops/http_start.rs:20`); called by `Deno.serveHttp` (`ext/runtime/js/http.js:67`) | Yes, through `Deno.serveHttp(conn)` | Already served by the bootstrap's `WorkerHttpConn` (`decision 0043`, `crates/megabase-functions/src/bootstrap.rs`). The bootstrap does not use a duplex stream, so it is not marked as this op |
| `op_http_upgrade_websocket2` | upgrades the request's connection to a WebSocket (`ext/runtime/ops/http.rs:237`) | Yes: `Deno.upgradeWebSocket` | Deno's own implementation runs, but the gateway proxy does not forward `Upgrade` requests, so no WebSocket reaches a function. Not served |
| `op_http_upgrade_raw2` | raw `Upgrade` stream (`ext/runtime/ops/http.rs:280`) | Yes: `Deno.upgradeHttpRaw` | Same gateway limit. Not served |
| `op_http_upgrade_raw2_fence` | waits for the upgrade response (`ext/runtime/ops/http.rs:375`) | Yes, with the op above | Same gateway limit. Not served |
| `op_is_runtime_init` | `RuntimeState::is_init()` (`ext/runtime/lib.rs:281`); gates `bootstrap.js:752`, `:763` | No: worker lifecycle plumbing | Not served |

PR 2 units: `functions:op:op_is_terminal`, `op_main_module`, `op_mi_collect`,
`op_net_accept`, `op_net_listen`, `op_net_unsupported`, `op_raise_segfault`,
`op_read_line_prompt`, `op_runtime_memory_usage`, `op_runtime_metrics`.
Only `op_runtime_memory_usage` is served and marked (`runtime.rs`).

| Unit | Upstream behavior (file:line) | Observable by a function? | Megabase |
|---|---|---|---|
| `op_is_terminal` | commented out (`ext/runtime/lib.rs:275`), not registered (`:498`); no behavior | No | Not served |
| `op_main_module` | the service main module after a read-permission check (`ext/runtime/ops/bootstrap.rs:26`); `Deno.mainModule` (`js/bootstrap.js:579`) | Yes, but the value is edge-runtime's service path; Megabase's main module is its generated bootstrap file, and a faithful value needs the reference stack | Not served |
| `op_mi_collect` | forces an mimalloc collect if the symbol exists (`lib.rs:446`) | No: `EdgeRuntime.miCollect` is on the `main` worker only (`js/namespaces.js:36`) | Not served |
| `op_net_listen` | no-op listener on `0.0.0.0:9999` (`ops/net.rs:114`) | Only as `Deno.serveHttp` plumbing | Not served; the bootstrap serves HTTP itself (see `op_http_start`) |
| `op_net_accept` | receives the next in-memory duplex stream (`ops/net.rs:130`) | Same | Not served |
| `op_net_unsupported` | `not_supported` error (`ops/net.rs:231`) | Replaces other net APIs; which ones is not verified against the reference | Not served |
| `op_raise_segfault` | null dereference (`lib.rs:382`) | No: `main` worker only (`js/namespaces.js:35`) | Not served; the crate forbids `unsafe` |
| `op_read_line_prompt` | returns `None` (`lib.rs:355`), registered at `:502` | No caller found in the vendored JS | Not served |
| `op_runtime_memory_usage` | `{rss: 0, heapTotal, heapUsed, external}` from V8 heap statistics (`lib.rs:339`); replaces `Deno.memoryUsage` (`js/bootstrap.js:719`) | Yes: `Deno.memoryUsage()` | Served: keeps Deno's heap numbers, `rss: 0` |
| `op_runtime_metrics` | heap and shared statistics (`lib.rs:303`) | No: `EdgeRuntime.getRuntimeMetrics`, `main` worker only (`js/namespaces.js:32`) | Not served |

## Memory usage

`Deno.memoryUsage()` returns a new `{ rss: 0, heapTotal, heapUsed, external }`
object; the last three come from Deno's own call. Upstream hardcodes `rss`
to 0 "for security". Deno's result has the same four keys.

## Console size

`Deno.consoleSize()` returns a new `{ columns: 80, rows: 24 }` object on
every call, whatever the host terminal. The property stays writable and
configurable, like the upstream `osCalls` entry.

## Judge cases

None. The only vendored user function
(`vendor/supabase/docker/volumes/functions/hello/index.ts`) does not call
these APIs, and fixtures cannot add TypeScript (GOAL.md rule 1). No
protected judge file is needed. Tests: `crates/megabase-functions/src/runtime.rs`
and the `bootstrap.rs` module test.

## Open for the next PRs

`op_http_upgrade_*` need WebSocket support in the gateway proxy
(`crates/megabase-functions/src/proxy.rs`) first. That is a separate change.
