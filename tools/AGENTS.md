# tools/: repository tooling (Rust)

Extends the root `AGENTS.md`; it does not relax it.

- `megabase-coverage` (`update`, `check`, `verify-pins`): if you change
  extraction or rendering, commit the regenerated output (`just coverage`)
  in the same PR, and explain any change in the unit count in the PR body.
- `megabase-backlog`: `plan` writes `docs/backlog/PLAN.md`. `sync` writes
  to GitHub and is run by the orchestrator only. Keep it idempotent: it
  matches on `megabase-id`.
- `megabase-guard` ⛔ implements `docs/adr/0003-protected-paths.md`. Never
  weaken it, and update the ADR in the same PR as any rule change.
- Treemap and badge style changes go through the root design gate.
  Regenerating data reuses the approved style.
