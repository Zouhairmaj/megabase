# Protected paths

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

[ADR 0003](../adr/0003-protected-paths.md). `MANIFESTO.md` is always
rejected. `GOAL.md` is always rejected except the lead-approved Design and
Documentation sections on the Phase 0 bootstrap branch. `HUMAN_LOG.md` is
rejected except when that bootstrap branch creates it empty, and except the
bounded `review/*` Pending and Completed edits below. `vendor/`,
`vendor.toml`, and `.gitmodules` are always rejected after bootstrap.
`judge/`, CI, and the guard change only on `review/*`. The Phase 0 bootstrap exception applies solely to
branch `cursor/phase-0-bootstrap-121c` while `main` still has no
`vendor.toml`. `release-please--branches--*` may change only `CHANGELOG.md`,
`.release-please-manifest.json`, `Cargo.toml`, `Cargo.lock`, and delete
`release-as` from `release-please-config.json`. `review/*` may append or
complete `HUMAN_LOG.md` Pending items (grows must start a new `- ` item;
shrinks must end at a `- **Date**` item) and grow the Completed section.
