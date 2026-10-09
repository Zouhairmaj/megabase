# Protected paths

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

[ADR 0003](../adr/0003-protected-paths.md). `GOAL.md` and `MANIFESTO.md`
are always rejected, apart from the Phase 0 bootstrap sections named in
that ADR. `HUMAN_LOG.md` is rejected except the bootstrap case and the
bounded `review/*` edits below. `vendor/`, `vendor.toml`, and `.gitmodules`
are always rejected after bootstrap. `judge/`, CI, and the guard change
only on `review/*`. The Phase 0 bootstrap exception applies solely to
branch `cursor/phase-0-bootstrap-121c` while `main` still has no
`vendor.toml`. `release-please--branches--*` may change only `CHANGELOG.md`,
`.release-please-manifest.json`, `Cargo.toml`, `Cargo.lock`, and delete
`release-as` from `release-please-config.json`. `review/*` may append or
complete `HUMAN_LOG.md` Pending items (grows must start a new `- ` item;
shrinks must end at a `- **Date**` item) and grow the Completed section.
