# Edge Function workers preload a listen bootstrap

- Status: Accepted
- Date: 2026-10-15

Issue [#400](https://github.com/Zouhairmaj/megabase/issues/400). Many
functions still use the legacy `serve()` of
`https://deno.land/std@0.168.0/http/server.ts`, which calls
`Deno.listen({ port: 8000 })` and then `Deno.serveHttp(conn)`. In
edge-runtime this works: a user worker's `Deno.listen` receives the
worker's requests whatever address it asks for, and `Deno.serveHttp`
exists (`vendor/edge-runtime/ext/runtime/js/denoOverrides.js`,
`ext/runtime/js/http.js`). Under a plain Deno 2 child process
([0042](0042-functions-deno-child-process.md)) the function binds the real
`:8000` (a second function fails with `AddrInUse`), ignores
`DENO_SERVE_ADDRESS`, and Deno 2 no longer has `Deno.serveHttp`.

Options considered:

1. Give every worker `PORT=<port>`. std `serve()` does not read it.
2. An import map redirecting `deno.land/std@*/http/server.ts`. Import maps
   match exact prefixes, so every std version would need an entry, and a
   function's own `deno.json` import map would conflict.
3. A network namespace per worker so `:8000` is private. Needs privileges
   Megabase does not have in a container.
4. Preload a small bootstrap before the function (`deno run --preload`)
   that does what edge-runtime does in JavaScript: route TCP `Deno.listen`
   to the worker's port and provide `Deno.serveHttp` for those connections.

**Decision.** Option 4. The bootstrap is a JavaScript string constant in
`crates/megabase-functions/src/bootstrap.rs` (no non-Rust source file in
the repository), written once per `DenoRuntime` to a fresh private
temporary directory and removed when the runtime is dropped. On the first
TCP `Deno.listen` it starts one `Deno.serve` (bound through
`DENO_SERVE_ADDRESS`), hands each request to the listener as a
one-request connection, and answers `Deno.serveHttp` for those
connections. `Deno.serve` itself and non-TCP listeners are untouched.

Consequences: `MEGABASE_DENO` must be Deno 2.4 or later (`--preload`). A
function's main module is still its own `index.ts`, so `import.meta.main`
and config discovery are unchanged. Raw TCP servers that read bytes from
`Deno.listen` connections are not supported, as in edge-runtime, where
those connections only carry HTTP. Spec:
[`specs/functions/invoke.md`](../../specs/functions/invoke.md).
