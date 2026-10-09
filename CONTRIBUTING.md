# Contributing to Megabase

Megabase is written by agents. The mission and guardrails are in
[`GOAL.md`](GOAL.md) and [`MANIFESTO.md`](MANIFESTO.md), which humans own.
**Commands, the task workflow, PR titles, boundaries and the definition of
done are in [`AGENTS.md`](AGENTS.md)**, and they apply to every contributor,
agent or human. This file covers only what AGENTS.md does not.

## Sources of truth

| What | Where |
|---|---|
| Mission and guardrails | `MANIFESTO.md`, `GOAL.md` (humans only) |
| How to work in this repo | `AGENTS.md` (plus nested `AGENTS.md` per directory) |
| What is being worked on | GitHub Project [Megabase Backlog](https://github.com/users/Zouhairmaj/projects/1) |
| Decisions and overall state | `PROGRESS.md` |
| Compatibility contract | `docs/COMPATIBILITY.md` |
| Graphic and UI design | [Kite: Megabase identity](https://kite.new/p/megabase-identity), see `docs/brand/README.md` |
| Coverage denominator | `coverage/units.json` (extracted from `vendor/`) |
| Correctness | `judge/` against the pinned reference stack |

## Releases

PRs are squash-merged, and the PR title becomes the changelog entry.
`conformance` titles land in the **Conformance/judge** section. release-please
(v4, `rust` strategy, `bump-minor-pre-major`) cuts releases from `main`. The
single version is `workspace.package.version` in `Cargo.toml`, starting at
`0.0.0`. Each GitHub Release body is annotated with the coverage and
conformance delta against the previous tag. If a release PR's lockfile is
stale, run `cargo generate-lockfile` on that PR.

## Questions

Use [GitHub Discussions](https://github.com/Zouhairmaj/megabase/discussions).
Agents ask humans only through issues labeled `needs-human`.
