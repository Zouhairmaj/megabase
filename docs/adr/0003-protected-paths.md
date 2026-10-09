# ADR 0003: Protected-path policy and the Phase 0 bootstrap exception

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

## Context

GOAL.md forbids changing `vendor/` or `judge/` outside a reviewed branch,
and humans own `GOAL.md`, `MANIFESTO.md` and `HUMAN_LOG.md`. The Phase 0
bootstrap PR must *create* `vendor/`, `judge/` and `HUMAN_LOG.md`, so a
naive "reject any touch of these paths" check would block the one PR that
is allowed to introduce them.

## Decision

`tools/megabase-guard` is the policy. It is a pure function of the change
set plus a small amount of git context (base tree, head branch, pin
agreement). CI runs it on every pull request.

### Always rejected

- Edits to `GOAL.md` or `MANIFESTO.md`.
- Edits to `HUMAN_LOG.md` except the bootstrap case below.
- Edits to `vendor/`, `vendor.toml` or `.gitmodules` after bootstrap.
- Source files in languages other than Rust (`.py`, `.js`, `.ts`, `.go`,
  `.ex`, `.hs`, `.rb`, `.lua`, `.sh`, …) outside `vendor/`. SQL, TOML,
  YAML, Markdown and SVG stay allowed.

### Allowed only on `review/*` branches

- `judge/`
- `.github/`
- `tools/megabase-guard/`
- `CODEOWNERS`

The reviewer agent reviews those PRs. The orchestrator merges them.

### Bootstrap exception (self-expiring)

The exception is active **only if both** are true:

1. The merge base has none of `vendor.toml`, `judge/` or vendor gitlinks
   (the tree is still Day 0).
2. The head branch is exactly `cursor/phase-0-bootstrap-121c`.

Then the PR may add the protected tree, provided:

- `GOAL.md` and `MANIFESTO.md` are unchanged.
- `HUMAN_LOG.md` is **created empty** (zero bytes).
- Every `vendor/` gitlink matches `vendor.toml`.

The exception cannot apply to a later PR: after this lands, the base has
`vendor.toml` and condition 1 is false. It cannot apply to a different
branch name even before this lands. It is not a label anyone can add.

## Consequences

A required status check named **Protected paths** should be set on `main`
(human-only: branch protection). Agents keep the policy in this ADR and in
`PROGRESS.md` Decisions; they do not weaken it for convenience.
