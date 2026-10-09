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

## Design

Mockups live in the [Kite identity file](https://kite.new/p/megabase-identity). The public site is generated from `site/` in Rust. No JavaScript framework.

## Supply chain

Megabase owns two Rust lockfiles: the workspace `Cargo.lock` and `site/Cargo.lock` (the static generator is a standalone crate). `just audit` and the CI `cargo-audit` matrix each run `cargo audit --file` once per owned lockfile. A workspace-only `cargo audit` misses `site/`. OpenSSF Scorecard's OSV check walks every `Cargo.lock` in the tree, including `site/`.

Lockfiles under `vendor/` belong to the pinned upstream spec. Agents never edit them ([ADR 0003](adr/0003-protected-paths.md)). Report issues in those trees upstream; do not add an OSV ignore unless a Scorecard finding is only in `vendor/` and cannot be fixed without bumping a pin.

`site/` used `resvg` 0.45, which pulled in unmaintained `rustybuzz` (RUSTSEC-2026-0206) and `ttf-parser` (RUSTSEC-2026-0192). `resvg` 0.48 shapes text with `harfrust` and `skrifa` instead. Workspace `cargo audit` did not see those crates because `site/` is excluded from the workspace.

## What a human does

Humans wrote the manifesto, GOAL.md and the initial setup. Every later human action is logged in HUMAN_LOG.md. The count is part of the result.
