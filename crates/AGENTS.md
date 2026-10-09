# crates/: product code

Extends the root `AGENTS.md`; it does not relax it.

- Routes under the Supabase prefixes (`/rest/v1`, `/auth/v1`, …, see
  `docs/adr/0002-gateway-layout.md`) follow the pinned gateway exactly.
  Megabase-only endpoints live under `/_megabase/` only (for example
  `/_megabase/health`, used by the CI image smoke test).
- Shared code (config, errors, JWT) goes in `megabase-core`. Never copy
  helpers between component crates.
- Keep the catch-all 501 for every path not yet ported:
  `megabase-server/tests/not_implemented.rs` must stay green.
- Put one `// megabase:unit <id>` marker per unit, on the handler that serves
  it, once its behavior matches the spec. Then run `just coverage`.
- Tests live in the crate (`src/` unit tests, `tests/` integration tests).
  `cargo test` has no network or Docker; anything that needs the reference
  stack is a judge case.
- Iterate with `cargo test -p <crate>` and
  `cargo clippy -p <crate> --all-targets --locked -- -D warnings`.
- Parser and JWT changes should stay panic-free on arbitrary input; the
  cargo-fuzz targets in `fuzz/` (`jwt`, `gateway_http`, `rest_query`) exist
  to catch that. See the root `AGENTS.md` and `docs/contributing.md`.
- `megabase`, `megabase-core` and `megabase-server` must build on Rust 1.89
  (CI MSRV job).
