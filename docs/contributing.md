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
5. Run the judge (`just judge` or the equivalent once `judge/` exists).
6. Keep the commit only if total conformance does not fall.
7. Record: PR, coverage, PROGRESS.md for decisions that outlive one issue.

Failures return HTTP 501 with `{"code":"MEGABASE_NOT_IMPLEMENTED",...}`. Never a plausible but unverified answer.

## Hard rules (short)

- Rust only, with the exceptions in GOAL.md (configuration, compatibility SQL, Studio assets).
- Never modify `vendor/` or `judge/` on a feature branch.
- Never disable a test to make a score go up.
- Credit upstream in a header comment; keep NOTICE current.
- Conventional Commits for the site and docs; component unit commits follow GOAL.md.

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
requests and 10 minutes on a nightly schedule (and on `workflow_dispatch`).
A crash uploads `fuzz/artifacts/`. Seed inputs live in `fuzz/corpus/<target>/`.
Do not commit `fuzz/target/`, `fuzz/artifacts/`, or `fuzz/coverage/`.

## Design

Mockups live in the [Kite identity file](https://kite.new/p/megabase-identity). The public site is generated from `site/` in Rust. No JavaScript framework.

## What a human does

Humans wrote the manifesto, GOAL.md and the initial setup. Every later human action is logged in HUMAN_LOG.md. The count is part of the result.
