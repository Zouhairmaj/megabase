# Rust supply-chain and API checks

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

Recorded on `main` as decision 23 in #176. Workspace package metadata,
`[workspace.dependencies]`, and `lints.workspace = true` were already in
place ([0023](0023-rust-agent-ergonomics.md)). This adds
`[workspace.lints.rust] unsafe_code = "forbid"` and moves the remaining
direct deps (`bcrypt`, `chrono`, `uuid`, `criterion`) into
`[workspace.dependencies]`. `site/` repeats the forbid lint because it
stays outside the workspace. `fuzz/` does not: the libfuzzer harness
emits `unsafe`. `cargo-vet` 0.10 imports Mozilla, Google, and Bytecode
Alliance audits; everything else is an exemption in
`supply-chain/config.toml` (36 audited, 184 exempted at introduction).
CI runs `cargo vet --locked`. Unused dependencies use `cargo-machete`
(stable). `cargo-udeps` needs nightly, so it is not the CI tool.
`cargo hack check --each-feature` runs only on workspace crates that
declare features, so it does not rebuild the workspace while none do.
API style for new code is the short list in `AGENTS.md` (Pragmatic Rust
Guidelines and Rust API Guidelines): newtypes, dedicated error types,
`Result` for input failures, and rustdoc. `megabase-server` no longer
depends on `megabase-pooler`; Supavisor has no Kong prefix (ADR 0002)
and the crate was unused.
