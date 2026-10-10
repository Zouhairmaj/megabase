---
title: Contributing & architecture
description: How the agents work — vendor/ as the spec, the judge, levels, commits and the design rule.
section: project
order: 1
card: How the agents work: vendor/ as the spec, the judge, levels, Conventional Commits and the design rule.
---

# Contributing & architecture

Read GOAL.md and MANIFESTO.md first. This page is a short map, not a substitute.

## Architecture

One Cargo workspace, one final binary: `megabase`. One crate per Supabase-authored service (`megabase-rest`, `megabase-auth`, `megabase-realtime`, `megabase-storage`, `megabase-functions`, `megabase-pooler`, `megabase-meta`, `megabase-studio`), plus `megabase-core` (config, errors, HS256 JWT) and `megabase-server` (gateway). HTTP: axum on tokio. PostgreSQL stays external. JWT verification lives only in `megabase-core`; component crates must not copy it. With `DATABASE_URL` set, the binary installs the Auth SQL objects it implements.

## The loop

1. Read GOAL.md, PROGRESS.md, coverage/summary.json.
2. Take one Ready issue.
3. Spec the unit from `vendor/` if the spec file is missing.
4. Implement in Rust.
5. Run the judge (`just judge`). Mutating cases also compare database
   side-effects; judge harness changes stay on `review/*`.
6. Keep the commit only if total conformance does not fall.
7. Record: PR, coverage, PROGRESS.md for decisions that outlive one issue.

Failures return HTTP 501 with `{"code":"MEGABASE_NOT_IMPLEMENTED",...}`. Never a plausible but unverified answer.

## Hard rules (short)

- Rust only, with the exceptions in GOAL.md (configuration, compatibility SQL, Studio assets).
- Never modify `vendor/` or `judge/` on a feature branch.
- Never disable a test to make a score go up.
- Credit upstream in a header comment; keep NOTICE current.
- Conventional Commits for the site and docs; component unit commits follow GOAL.md.

## Checked queries

Auth DML is `sqlx::query!` / `sqlx::query_as!`. `.cargo/config.toml` sets
`SQLX_OFFLINE=true`, and `.sqlx/` is the committed query metadata. After
changing a query string, install `sqlx-cli` 0.8.6 (the `sqlx` pin in the
workspace `Cargo.toml`) and refresh the cache against a database that already
has the Auth schema:

```shell
cargo install sqlx-cli --version 0.8.6 --locked --no-default-features --features native-tls,postgres
SQLX_OFFLINE=false DATABASE_URL=postgres://… cargo sqlx prepare --workspace -- --all-targets
```

Commit the `.sqlx` files the command writes. A CI `cargo sqlx prepare --check`
step would live in `.github/` and needs its own `review/*` pull request.

## Fuzzing

OpenSSF Scorecard's Fuzzing check treats a Rust repo as fuzzed when a `*.rs`
file contains `libfuzzer_sys` (`checks/raw/fuzzing.go`, tool
`RustCargoFuzz`). `fuzz/` is a standalone cargo-fuzz workspace (not a member
of the root workspace) with three targets:

| Target | Exercises |
|---|---|
| `jwt` | `megabase-core` HS256 `verify_at` and `bearer_token` |
| `gateway_http` | gateway request-target parse and Kong prefix matching |
| `rest_query` | stub walker for PostgREST query strings (filter parsing is not implemented yet) |

Needs a nightly toolchain and `cargo-fuzz` 0.13.2 (the root
`rust-toolchain.toml` is stable; pass `+nightly`):

```shell
rustup toolchain install nightly
cargo install cargo-fuzz --locked --version 0.13.2
just fuzz jwt            # or gateway_http / rest_query; default 60s
just fuzz jwt 600
cargo +nightly fuzz list
# just fuzz pins --target to the host triple so a musl-built cargo-fuzz
# does not pick x86_64-unknown-linux-musl (ASan cannot link static musl).
```

`.github/workflows/fuzz.yml` runs each target for 60 seconds on pull
requests and 600 seconds by default on a nightly schedule and
`workflow_dispatch` (the `seconds` input can override that duration).
A crash uploads `fuzz/artifacts/`. Seed inputs live in `fuzz/corpus/<target>/`.
Do not commit `fuzz/target/`, `fuzz/artifacts/`, or `fuzz/coverage/`.

## Design

Mockups live in the [Kite identity file](https://kite.new/p/megabase-identity). The public site is generated from `site/` in Rust. No JavaScript framework.

## Supply chain

Megabase owns two Rust lockfiles: the workspace `Cargo.lock` and `site/Cargo.lock` (the static generator is a standalone crate). `just audit` and the CI `cargo-audit` matrix each run `cargo audit --file` once per owned lockfile. A workspace-only `cargo audit` misses `site/`. The workspace ignore list is `.cargo/audit.toml`: RUSTSEC-2023-0071 (`rsa` 0.9.10, no fixed release) is there because `sqlx-macros-core` always depends on `sqlx-mysql` and Megabase does not enable that driver. OpenSSF Scorecard's OSV check walks every `Cargo.lock` in the tree, including `site/`.

Lockfiles under `vendor/` belong to the pinned upstream spec. Agents never edit them ([ADR 0003](adr/0003-protected-paths.md)). Report issues in those trees upstream; do not add an OSV ignore unless a Scorecard finding is only in `vendor/` and cannot be fixed without bumping a pin.

`cargo vet --locked` checks the workspace graph against imported audits from Mozilla, Google, and the Bytecode Alliance (`supply-chain/`). Crates those imports do not cover are exemptions. `cargo-machete` fails CI on an unused dependency in `crates/`, `tools/`, `judge/`, `site/`, and `fuzz/` (`cargo-udeps` needs nightly). `cargo-hack check --locked --each-feature` runs only for workspace crates that declare features; today none do, so that job does not rebuild the workspace.

`site/` used `resvg` 0.45, which pulled in unmaintained `rustybuzz` (RUSTSEC-2026-0206) and `ttf-parser` (RUSTSEC-2026-0192). `resvg` 0.48 shapes text with `harfrust` and `skrifa` instead. Workspace `cargo audit` did not see those crates because `site/` is excluded from the workspace.

## What a human does

Humans wrote the manifesto, GOAL.md and the initial setup. Every later human action is logged in HUMAN_LOG.md. The count is part of the result.
