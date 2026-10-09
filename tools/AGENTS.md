# tools/: repository tooling (Rust)

Extends the root `AGENTS.md`; it does not relax it.

- `megabase-coverage` (`update`, `check`, `verify-pins`): if you change
  extraction or rendering, commit the regenerated output (`just coverage`)
  in the same PR, and explain any change in the unit count in the PR body.
  README coverage/conformance shields.io badges read
  `coverage/badge-coverage.json` and `coverage/badge-conformance.json`
  (shields endpoint schema). Colors: `#e05d44` below 50%, `#fe7d37` from
  50% to under 90%, `#00D892` at 90% and above. SVG/treemap chips keep
  the brand greens in `badge_color`.
- `megabase-backlog`: `plan` writes `docs/backlog/PLAN.md`. `sync` writes
  to GitHub and is run by the orchestrator only. Keep it idempotent: it
  matches on `megabase-id`.
- `megabase-guard` ⛔ implements `docs/adr/0003-protected-paths.md`. Never
  weaken it, and update the ADR in the same PR as any rule change.
- Treemap and badge style changes go through the root design gate.
  Regenerating data reuses the approved style.
