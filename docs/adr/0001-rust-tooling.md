# ADR 0001: Rust tooling replaces Python

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

## Context

Phase 0 originally sketched Python scripts for unit extraction, treemap
generation and the judge comparison. GOAL.md rule 1 is Rust only for code
we write.

## Decision

All tools live in the Cargo workspace:

- `tools/megabase-coverage` — extract `coverage/units.json`, status, treemaps, badges, doc blocks
- `tools/megabase-guard` — protected-path policy
- `tools/megabase-backlog` — GitHub milestones, labels, Project, epics
- `judge/harness` (`megabase-judge`) — differential tests

Python scripts are not part of the tree.

## Consequences

CI runs `cargo` only. Contributors need a Rust toolchain (MSRV 1.89). The
tools share types with the workspace (`serde`, `toml`, `anyhow`).
