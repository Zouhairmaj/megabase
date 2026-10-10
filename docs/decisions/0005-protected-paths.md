# Protected paths

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

[ADR 0003](../adr/0003-protected-paths.md). `MANIFESTO.md` and `GOAL.md`
are rejected except the lead-approved Design and Documentation sections
on the Phase 0 bootstrap branch (`GOAL.md` only), and except a `review/*`
modification whose same diff appends a Completed `HUMAN_LOG.md` entry that
names the file in backticks ([0029](0029-stop-publishing-cost.md)). `HUMAN_LOG.md` is
rejected except when that bootstrap branch creates it empty, and except the
bounded `review/*` Pending and Completed edits below. `vendor/`,
`vendor.toml`, and `.gitmodules` are always rejected after bootstrap.
`judge/`, CI, and the guard change only on `review/*`. The Phase 0
bootstrap exception applies solely to branch
`cursor/phase-0-bootstrap-121c` when the merge base has none of
`vendor.toml`, `judge/`, or vendor gitlinks. The full condition is in
[ADR 0003](../adr/0003-protected-paths.md). `release-please--branches--*` may change only `CHANGELOG.md`,
`.release-please-manifest.json`, the `[workspace.package]` version in `Cargo.toml`, quoted version lines in `Cargo.lock`, and delete
`release-as` from `release-please-config.json`. `review/*` may append or
complete `HUMAN_LOG.md` Pending items (grows must start a new `- ` item;
shrinks must end at a `- **Date**` item) and grow the Completed section.
