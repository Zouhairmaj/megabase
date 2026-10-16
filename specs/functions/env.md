# Functions: worker environment (`Deno.env`) (issue 72)

Status: draft
Unit ids (`coverage/units.json`): `functions:op:op_set_env`,
`functions:op:op_env`, `functions:op:op_get_env`, `functions:op:op_delete_env`
Level: 4

An Edge Function reads its configuration with `Deno.env.get("NAME")` (or
`Deno.env.toObject()`, or `process.env` through the Node polyfill, which
calls `Deno.env`). In edge-runtime a user worker has no access to the
process environment: `Deno.env` is `SUPABASE_ENV`, backed by the map of
variables the worker was created with. The function can read that map but
not change it.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| `SUPABASE_ENV`: `get`, `toObject`, `set`, `has`, `delete` over the four ops; `get` maps a missing key to `undefined` | `vendor/edge-runtime/ext/env/env.js:3`–`:29` |
| `Deno.env` is `readOnly(SUPABASE_ENV)` (not writable, configurable, enumerable) | `vendor/edge-runtime/ext/runtime/js/bootstrap.js:576`, `ext/runtime/js/fieldUtils.js:26` |
| `op_set_env`: always `Err(not_supported())` | `vendor/edge-runtime/ext/env/lib.rs:29`–`:36` |
| `op_env`: `check_env_all`, then a copy of `EnvVars` | `vendor/edge-runtime/ext/env/lib.rs:38`–`:44` |
| `op_get_env`: `check_env` (skipped for `NODE_DEBUG`, `NODE_OPTIONS`), empty key, `=` / NUL key, lookup | `vendor/edge-runtime/ext/env/lib.rs:46`–`:71`, `ext/node/lib.rs:170` |
| `op_delete_env`: no-op | `vendor/edge-runtime/ext/env/lib.rs:73`–`:74` |
| User workers get `allow_env: Some([])` (every variable allowed) | `vendor/edge-runtime/crates/base/src/runtime/permissions.rs:19`–`:22` |
| `EnvVars` = the `envVars` given to `userWorkers.create` plus `SB_EXECUTION_ID` = the worker key | `vendor/edge-runtime/crates/base/src/runtime/mod.rs:1150`–`:1179` |
| The worker key is a fresh `Uuid::new_v4()` per worker | `vendor/edge-runtime/crates/base/src/worker/pool.rs:460`–`:475` |
| `main` passes `Object.entries(Deno.env.toObject())` (the container env) | `vendor/supabase/docker/volumes/functions/main/index.ts:145`–`:154` |
| Node `process.env` goes through `Deno.env.get/toObject/set/delete` | `vendor/edge-runtime/ext/node/polyfills/_process/process.ts:48`–`:106` |
| Container env of the functions service | `vendor/supabase/docker/docker-compose.yml:442`–`:480` |

Pins: edge-runtime `v1.77.4`, supabase `v1.26.08` (`vendor.toml`).
`crates/megabase-functions/src/env.rs` carries the port header and the four
unit markers.

## The map

Upstream: the container environment of `main` (compose sets `JWT_SECRET`,
`SUPABASE_URL`, `SUPABASE_ANON_KEY`, `SUPABASE_SERVICE_ROLE_KEY`,
`SUPABASE_DB_URL`, `VERIFY_JWT`, plus the image's own variables), plus
`SB_EXECUTION_ID`.

Megabase: the worker process environment (Megabase's environment plus the
`SUPABASE_*` defaults in `specs/functions/invoke.md` Outputs 5), snapshotted
once when the worker starts, minus the variables only Megabase uses to run
the worker (`DENO_SERVE_ADDRESS`, `MEGABASE_FUNCTION_ENTRYPOINT`), plus
`SB_EXECUTION_ID` = `crypto.randomUUID()` (lowercase hyphenated v4, the
format of `Uuid::to_string`). A restarted worker gets a new id.

## Inputs and outputs

`key` and `value` are strings.

1. **`Deno.env.get(key)`** (`op_get_env`). In order:
   - the permission check always passes (user workers allow every variable);
   - `""` → `TypeError: Key is an empty string.`;
   - a key containing `=` or NUL → `TypeError: Key contains invalid
     characters: <key as Rust {:?}>`, for example `"A=B"` gives
     `Key contains invalid characters: "A=B"` and `"A\0B"` gives
     `Key contains invalid characters: "A\0B"`;
   - otherwise the value, or `undefined` when the key is not in the map.
2. **`Deno.env.has(key)`**: `get(key) !== undefined`, so it throws the same
   errors as `get`.
3. **`Deno.env.toObject()`** (`op_env`): a new plain object with every
   entry of the map on each call. Changing it does not change the map.
4. **`Deno.env.set(key, value)`** (`op_set_env`): always throws
   `Deno.errors.NotSupported` with message `The operation is not supported`
   (`deno_core::error::not_supported`). The map is unchanged.
5. **`Deno.env.delete(key)`** (`op_delete_env`): returns `undefined` and
   does nothing; the variable is still visible afterwards.
6. `Deno.env` itself cannot be reassigned (non-writable property).

## Edge cases

- The map never changes after the worker starts, so a value is the same
  for every request the worker serves.
- `process.env.X = "y"` reaches `set` and throws, as upstream.
- Rust `{:?}` escaping is reproduced for `\0 \t \r \n \\ "` and C0/C1
  control characters (`\u{1}`). Rust also escapes other non-printable and
  grapheme-extending code points; Megabase prints those unescaped. Only the
  error message text differs.
- Non-string arguments are not part of this unit: upstream converts them in
  `deno_core` op glue that is not vendored. Megabase rejects a non-string
  `key` to `get`/`has` with `TypeError: Expected string` (not measured).

## Serving

`crates/megabase-functions/src/bootstrap.rs` builds each worker's main
module: it reads the worker address, reads and deletes
`MEGABASE_FUNCTION_ENTRYPOINT` with Deno's own `Deno.env`, then runs
`env::SOURCE` (snapshot and `Deno.env` replacement), then imports the
function. The real process environment is never written by the function.

## Judge cases

None in this PR. The only vendored user function,
`vendor/supabase/docker/volumes/functions/hello/index.ts`, does not read
`Deno.env`, and repository fixtures cannot add TypeScript (GOAL.md rule 1),
so no request can observe these ops on the reference stack. The existing
`functions.invoke.*` cases still boot every worker through the new
bootstrap. Tests: `crates/megabase-functions/src/env.rs` and
`bootstrap.rs` unit tests.
