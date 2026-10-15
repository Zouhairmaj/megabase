# Edge Functions run in a Deno child process

- Status: Accepted
- Date: 2026-10-15

Issue [#381](https://github.com/Zouhairmaj/megabase/issues/381). Edge
Functions are TypeScript written against Deno's APIs (`Deno.serve`,
`Deno.env`, `jsr:` / `npm:` imports). Something has to execute them.

Options considered:

1. Embed V8 through `deno_core` / `deno_runtime` in the `megabase` binary.
   That is hundreds of new crates, a large build, `unsafe` FFI under the
   workspace's `forbid(unsafe_code)` product crates, and still not the
   full Deno API surface functions rely on.
2. Port edge-runtime's TypeScript `main` service into the repository. Not
   allowed: no non-Rust source outside `vendor/`.
3. Keep the gateway, JWT gate, routing and supervision in Rust, and run
   each function with an external Deno 2 executable.

**Decision.** Option 3. `megabase-functions` reproduces Kong's
`functions-v1` route and the self-hosted `main` service (JWT gate, name
routing, error bodies) in Rust, starts `deno run --allow-all --no-prompt
<dir>/<name>/index.ts` once per function with `DENO_SERVE_ADDRESS` set to
a loopback port, and proxies requests to it over HTTP/1.1 with `hyper`'s
client (new crates `want` and `try-lock`). Deno is an external runtime
dependency, like PostgreSQL: Megabase is still one binary, and without
`MEGABASE_FUNCTIONS_DIR` the component answers 501.

Consequences: operators install Deno next to Megabase (the container
image does not ship it yet). Per-function memory and CPU limits, idle
shutdown and JWKS verification are not reproduced. Spec:
[`specs/functions/invoke.md`](../../specs/functions/invoke.md).
