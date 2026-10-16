# Functions: invoke an Edge Function (issue 381)

Status: draft
Unit ids (`coverage/units.json`): `functions:route:* /functions/v1/{function_name}`
Level: 4

`supabase.functions.invoke('hello', { body })` sends any method to
`/functions/v1/hello`. In the self-hosted stack Kong strips `/functions/v1`
and forwards to Supabase Edge Runtime, whose `main` service checks the JWT
(when `VERIFY_JWT` is on), picks the function from the first path segment,
and hands the request to a user worker running
`/home/deno/functions/<name>/index.ts`. The function sees the method,
headers (including `Authorization`), query and body, and its response goes
back to the client unchanged.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Kong route `/functions/v1/` → `http://functions:9000/`, `strip_path: true`, `cors`, no `key-auth` | `vendor/supabase/docker/volumes/api/kong.yml:344`–`:355` |
| Container env: `JWT_SECRET`, `SUPABASE_URL=http://kong:8000`, `SUPABASE_ANON_KEY`, `SUPABASE_SERVICE_ROLE_KEY`, `SUPABASE_DB_URL`, `VERIFY_JWT=${FUNCTIONS_VERIFY_JWT}`; functions mounted at `/home/deno/functions` | `vendor/supabase/docker/docker-compose.yml:442`–`:480` |
| `FUNCTIONS_VERIFY_JWT=false` default | `vendor/supabase/docker/.env.example:300` |
| `VERIFY_JWT` is on only for the exact string `true` | `vendor/supabase/docker/volumes/functions/main/index.ts:7` |
| `getAuthToken`: missing header, `Bearer` check, `split(' ')` | `vendor/supabase/docker/volumes/functions/main/index.ts:34`–`:45` |
| HS256 via `jose.jwtVerify(jwt, JWT_SECRET)`; ES256/RS256 via `SUPABASE_JWKS`; other `alg` invalid | `vendor/supabase/docker/volumes/functions/main/index.ts:47`–`:109` |
| Gate skipped for `OPTIONS`; 401 `{"msg":"Invalid JWT"}` or `{"msg": e.toString()}` | `vendor/supabase/docker/volumes/functions/main/index.ts:111`–`:129` |
| First path segment is the function; empty → 400 `missing function name in request` | `vendor/supabase/docker/volumes/functions/main/index.ts:131`–`:143` |
| `servicePath`, `workerTimeoutMs = 60000`, whole env passed to the worker | `vendor/supabase/docker/volumes/functions/main/index.ts:145`–`:154` |
| `userWorkers.create` + `worker.fetch(req)`; any error → 500 `{"msg": e.toString()}` | `vendor/supabase/docker/volumes/functions/main/index.ts:156`–`:172` |
| Entrypoint lookup `index.{ts,tsx,js,mjs,jsx}`; none → `could not find an appropriate entrypoint` | `vendor/edge-runtime/crates/base/src/runtime/mod.rs:525`, `:590` |
| Boot failures wrapped as `worker boot error` | `vendor/edge-runtime/crates/base/src/worker/worker_inner.rs:250` |
| Creation errors surface as `InvalidWorkerCreation` | `vendor/edge-runtime/ext/workers/lib.rs:306`–`:318`, `ext/runtime/js/errors.js:29` |

Pins: supabase `v1.26.08`, edge-runtime `v1.77.4` (`vendor.toml`).
`crates/megabase-functions/src/gate.rs` carries the port header.

## Inputs

Any method on `/functions/v1/<name>[/<rest>][?<query>]`, any headers and
body. `<name>` is the text between `/functions/v1/` and the next `/`, used
as-is (no percent-decoding), like the segment upstream reads from the
stripped path.

Configuration (`docs/configuration.md`): `MEGABASE_FUNCTIONS_DIR` plays
`/home/deno/functions`; `VERIFY_JWT` / `FUNCTIONS_VERIFY_JWT`; `JWT_SECRET`;
`MEGABASE_DENO`.

## Outputs

1. **JWT gate** (only when `VERIFY_JWT` is `true`, never for `OPTIONS`).
   Status 401, `Content-Type: application/json`, body `{"msg": ...}`:
   - no or empty `Authorization`: `Error: Missing authorization header`;
   - first space-separated word is not exactly `Bearer`:
     `Error: Auth header is not 'Bearer {token}'`;
   - the second word (missing → `undefined`) is not a 3- or 5-part token
     whose first part is base64url JSON object:
     `TypeError: Invalid Token or Protected Header formatting`;
   - otherwise `Invalid JWT` unless `alg` is `HS256` and the token verifies
     with `JWT_SECRET` under jose's rules: signature, payload object,
     `exp > now`, `nbf <= now`, numeric `iat` (zero clock tolerance).
     `SUPABASE_JWKS` is unset in the pinned compose file, so ES256/RS256
     tokens are always `Invalid JWT`.
2. **Name**: empty name (`/functions/v1/`, `/functions/v1//x`) → 400
   `{"msg":"missing function name in request"}`. Checked after the gate.
3. **Worker**: `index.ts`, `index.tsx`, `index.js`, `index.mjs`,
   `index.jsx` in `<dir>/<name>/`, first match wins. The worker is started
   once and reused while its process runs (upstream reuses a live user
   worker for the same `servicePath`).
4. **Forward**: request-target `/<name>[/<rest>][?<query>]` (Kong
   `strip_path`), same method, all end-to-end headers including
   `Authorization`, `Host: functions:9000` (Kong does not preserve the
   host, so `req.url` inside the function is `http://functions:9000/...`),
   and the body, streamed. The function's status, headers and body come
   back streamed; hop-by-hop headers are dropped both ways.
5. **Environment** of the worker: Megabase's environment (upstream passes
   `Deno.env.toObject()`), plus `SUPABASE_URL`, `SUPABASE_ANON_KEY`,
   `SUPABASE_SERVICE_ROLE_KEY`, `SUPABASE_DB_URL` when not already set
   (see `docs/configuration.md`). `SUPABASE_URL` points at Megabase, so a
   function can call `/rest/v1` with the forwarded JWT.

CORS: Kong's `cors` plugin answers full preflights (`OPTIONS` with
`Origin` and `Access-Control-Request-Method`) before the function runs;
the gateway layer in `megabase-server` does the same (`specs/server/cors.md`).
Any other `OPTIONS` reaches the function, without the JWT gate.

## Errors

| When | Status | Body (upstream) |
|---|---|---|
| Gate failures above | 401 | `{"msg": "<message>"}` |
| Empty function name | 400 | `{"msg":"missing function name in request"}` |
| No entrypoint (missing function, `.`, `..`, `\`, NUL in the name) | 500 | `{"msg":"InvalidWorkerCreation: worker boot error: failed to bootstrap runtime: could not find an appropriate entrypoint"}` |
| Worker could not start (Deno missing, exits, no listener in 60 s) | 500 | `{"msg":"InvalidWorkerCreation: worker boot error: <Megabase detail>"}` |
| Worker connection or HTTP exchange fails | 500 | `{"msg":"InvalidWorkerResponse: <Megabase detail>"}`; the worker is dropped and restarted on the next request |

The missing-function body is derived from source; the issue text expects
404 (hosted Supabase), but the self-hosted `main` service returns 500. The
judge case settles it. The detail text of the other 500s is Megabase's own:
upstream's comes from edge-runtime internals (eszip build, V8 boot).

## Edge cases

- `Authorization: Bearer <jwt> extra` verifies `<jwt>` (`split(' ')`).
- `Bearer` with two spaces gives an empty token → header formatting error.
- With `VERIFY_JWT` off, no header is required; the function still gets
  whatever `Authorization` the client sent.
- Each function runs in its own `deno run --allow-all --no-prompt` process
  with `DENO_SERVE_ADDRESS=tcp:127.0.0.1:<port>`; concurrent first requests
  for the same function wait for one boot.
- Servers built on TCP `Deno.listen` work unchanged, whatever port they
  ask for: the legacy `serve()` of `https://deno.land/std@0.168.0/http/server.ts`
  (`Deno.listen({ port: 8000 })` + `Deno.serveHttp`). In edge-runtime every
  `Deno.listen` of a user worker receives the worker's requests and
  `Deno.serveHttp` serves them (`vendor/edge-runtime/ext/runtime/js/denoOverrides.js:98`–`:103`,
  `ext/runtime/js/http.js:64`, `:120`). Megabase runs a bootstrap module
  (`crates/megabase-functions/src/bootstrap.rs`, the main module of
  `deno run`, which then imports `index.ts`) that routes TCP `Deno.listen` to the worker's own `Deno.serve` and
  provides `Deno.serveHttp` for those connections, so two std-`serve()`
  functions never compete for `:8000`. Unix-socket listeners are untouched.
- Without `MEGABASE_FUNCTIONS_DIR` every request is the structured 501.

## Out of scope

- `SUPABASE_JWKS` / asymmetric verification, per-function `verify_jwt`
  (upstream TODO in the compose file), memory and CPU limits
  (`memoryLimitMb = 150`), worker idle timeout, import maps.
- Kong's `X-Forwarded-*` request headers.
- WebSocket upgrades to a function (hop-by-hop `Upgrade` is dropped).
- The edge-runtime ops (`functions:op:*`) are Deno internals, served by
  Deno itself, not by Megabase.

## Judge cases

To add on `review/issue-381-cases` (needs a functions fixture directory for
both stacks, `MEGABASE_FUNCTIONS_DIR` and Deno on the Megabase side, and
`FUNCTIONS_VERIFY_JWT=true` for the 401 case):

- `POST /functions/v1/<echo>` with a JSON body and the anon key → 200 echo.
- `GET /functions/v1/<echo>?a=1` → method and query echoed.
- `POST /functions/v1/missing` → status and body of a missing function.
- `verify_jwt`: no header, `Basic x`, garbage token, bad signature → 401.
- A function that calls `${SUPABASE_URL}/rest/v1/...` with the forwarded
  `Authorization` header.
- Issue #400: a function using std@0.168.0 `serve()`, one using
  `Deno.serve`, and two std-`serve()` functions invoked concurrently (both
  200, no `AddrInUse`).
