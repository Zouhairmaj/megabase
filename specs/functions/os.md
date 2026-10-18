# Functions: os (issue 76)

Status: draft
Level: 4
Unit id (`coverage/units.json`): `functions:op:op_system_memory_info`

Pin edge-runtime `v1.77.4`. Megabase runs a function as a plain `deno run`
child (`specs/functions/invoke.md`). The op is an edge-runtime `deno_core`
op that process does not have.

## Upstream

| Behavior | File:line |
|---|---|
| `op_system_memory_info` returns `Some(MemInfo)`. On Linux and Android it fills the struct from `libc::sysinfo` (`totalram`, `freeram`, `bufferram`, `totalswap`, `freeswap`, each times `mem_unit`). `available` is set to `free`, not to `MemAvailable`. `cached` stays 0. A failed `sysinfo` leaves the default zeros. Other operating systems return `Some(MemInfo::default())` | `ext/os/lib.rs:16`–`:43` |
| JSON names are camelCase: `total`, `free`, `available`, `buffers`, `cached`, `swapTotal`, `swapFree` | `ext/os/lib.rs:3`–`:14` |
| `Deno.systemMemoryInfo` is the all-zero object, a new one on every call. It does not call the op | `ext/os/os.js:37`–`:45`, `ext/runtime/js/denoOverrides.js:19` |
| `EdgeRuntime.systemMemoryInfo` calls the op, and only on the main worker | `ext/runtime/js/namespaces.js:28`–`:34` |
| Node `os.freemem` / `os.totalmem` read `Deno.systemMemoryInfo()`, so a user worker sees 0 | `ext/node/polyfills/os.ts:171`–`:178`, `:310`–`:311` |

`/proc/meminfo` reports the same counters `sysinfo` does: `MemTotal`,
`MemFree`, `Buffers`, `SwapTotal` and `SwapFree` are those fields in
kibibytes. Bytes are the kibibyte value times 1024 (`mem_unit` is the page
size, and the proc file already scaled by it).

## What a function observes

`Deno.systemMemoryInfo()` returns a new
`{ total: 0, free: 0, available: 0, buffers: 0, cached: 0, swapTotal: 0, swapFree: 0 }`.
The property is enumerable, writable and configurable, like the other
`osCalls` entries. That is `ext/os/os.js`, not the op. It is installed in
`crates/megabase-functions/src/os.rs` and is not marked: the same split as
`op_console_size`, whose observable value also comes from `os.js`.

## The op

`system_memory_info` is the op. On Linux it reads `/proc/meminfo` and maps
the five fields above. `available` is `free`. `cached` is 0. `MemAvailable`
and `Cached` in the file are ignored. A missing file, a non-Linux host, or
an unreadable counter yields zeros, and the value is still present (the op
returns `Some`, including `Some` of the default). Not marked: a user
function cannot call it. `EdgeRuntime.systemMemoryInfo` exists only on the
main worker, which Megabase does not run.

## Judge cases

None. The vendored `hello` function does not call `Deno.systemMemoryInfo`,
and a fixture function would be TypeScript outside `vendor/`. No protected
judge file is needed. Tests parse `/proc/meminfo`, call
`system_memory_info`, and run the `Deno.systemMemoryInfo` shim under
`node`.
