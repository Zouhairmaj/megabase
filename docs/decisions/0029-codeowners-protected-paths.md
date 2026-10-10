# Code owners for protected paths

- Status: Accepted (agent coordinator)
- Date: 2026-10-10

## Context

The repository root `CODEOWNERS` already assigns `@Zouhairmaj` to the
mission files, the frozen spec, `judge/`, `.github/`, the guard, and
`CODEOWNERS` itself. GitHub searches for a code-owners file in
`.github/`, then the repository root, then `docs/`, and uses the first
file it finds. The root file stops being read once `.github/CODEOWNERS`
exists.

[ADR 0003](../adr/0003-protected-paths.md) allows `CODEOWNERS` and
`.github/` only on a `review/*` branch. Turning on "Require review from
Code Owners" is a branch protection change. Agents do not make that
change.

## Decision

`.github/CODEOWNERS` is the file GitHub reads. It assigns `@Zouhairmaj`
as owner of:

- `/GOAL.md`
- `/MANIFESTO.md`
- `/HUMAN_LOG.md`
- `/judge/`
- `/vendor/`
- `/.github/`
- `/CODEOWNERS`

Those paths require that owner's human code-owner review. `AGENTS.md`
and `CONTRIBUTING.md` say so.

The file also keeps `@Zouhairmaj` on `/vendor.toml`, `/.gitmodules`, and
`/tools/megabase-guard/`. The root file already named those owners.
Dropping them would stop GitHub from requesting the owner, because the
root file is no longer read. The root `CODEOWNERS` stays as a fallback
with the same owners.

Enabling the branch-protection option stays under "Human-only actions"
in `PROGRESS.md`. This decision does not change repository settings.
