# tools/: repository tooling (Rust)

Extends the root `AGENTS.md`; it does not relax it.

- `megabase-coverage` (`update`, `check`, `verify-pins`, `conformance`,
  `judge-history`): if you change
  extraction or rendering, commit the regenerated output (`just coverage`)
  in the same PR, and explain any change in the unit count in the PR body.
  README coverage/conformance shields.io badges and the README treemap
  picture read the copies published on the `gh-pages` branch
  (`coverage/badge-coverage.json`, `coverage/badge-conformance.json`,
  `coverage/treemap.png`, `coverage/treemap-light.png`). PRs still
  commit the generated files so `check` can reject drift. The committed
  `coverage/judge-results.json` is the regression baseline (decision
  0029). Generated prose does not quote its pass count as the
  conformance score. That score is the Judge publication on `gh-pages`
  and megabase.sh. `judge-history.json` on that branch is the
  per-commit record release notes use.
  Colors:
  `#e05d44` below 50%, `#fe7d37` from 50% to under 90%, `#00D892` at 90%
  and above. SVG/treemap chips keep the brand greens in `badge_color`.
- `megabase-backlog`: `plan` writes `docs/backlog/PLAN.md`. `sync` writes
  to GitHub and is run by the orchestrator only. Keep it idempotent: it
  matches on `megabase-id`.
- `megabase-guard` ⛔ implements `docs/adr/0003-protected-paths.md`. Never
  weaken it, and update the ADR in the same PR as any rule change.
- Treemap and badge style changes go through the root design gate.
  Regenerating data reuses the approved style.
