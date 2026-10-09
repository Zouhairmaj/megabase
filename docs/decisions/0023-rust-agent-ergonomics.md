# Rust agent ergonomics

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

Cloud Agent setup warms the registry and check cache (`cargo fetch`, then
`cargo check --workspace --all-targets`, in `.cursor/environment.json`).
Agents iterate with per-crate `cargo check` / `cargo clippy` and run full
tests only before pushing. `[workspace.lints.clippy]` denies the clone and
borrow lints; CI clippy stays `-D warnings`. `deny.toml` bans external
crates that are not on the `[bans] allow` list (the current lockfile graph,
plus `sqlx` and `jsonwebtoken`). Propose a new crate with the steps in
that file.

The same Cloud Agent install also links with mold when `mold` is on
`PATH`. CI test runs use cargo-nextest; local `just test` stays
`cargo test`.
