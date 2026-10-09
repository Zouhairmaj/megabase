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
| Graphic and UI design | [GOAL.md Design (design-first, Kite)](GOAL.md) (templates and layouts only); [Kite: Megabase identity](https://kite.new/p/megabase-identity), `docs/brand/README.md` |
| Project docs | [GOAL.md Documentation](GOAL.md); content lives in `docs/` markdown |
| Coverage denominator | `coverage/units.json` (extracted from `vendor/`) |
| Correctness | `judge/` against the pinned reference stack |
| Versioning and releases | [`docs/ROADMAP.md`](docs/ROADMAP.md#versioning-and-releases) |
| Vulnerability reports | [`SECURITY.md`](SECURITY.md) |

## Security

Report vulnerabilities privately as described in [`SECURITY.md`](SECURITY.md)
(email **agent@megabase.sh**, or GitHub private advisories). Do not open a
public issue. Owned Rust lockfiles (`Cargo.lock`, `site/Cargo.lock`) are
audited by `just audit` and by the CI `cargo-audit` matrix (`cargo audit
--file` once per lockfile). `vendor/` lockfiles are the frozen upstream spec.

## Releases

PRs are squash-merged, and the PR title becomes the changelog entry.
`conformance` titles land in the **Conformance/judge** section. Versioning,
cadence and who may merge a release PR are in
[`docs/ROADMAP.md`](docs/ROADMAP.md#versioning-and-releases) (do not restate
them here). The Release workflow keeps `Cargo.lock` in sync; if a release
PR's lockfile is still stale, run `cargo update -w` on that branch.
Signed binaries and the GHCR image: [`docs/install.md`](docs/install.md).

## Questions

Use [GitHub Discussions](https://github.com/Zouhairmaj/megabase/discussions).
Agents ask humans only through issues labeled `needs-human`.
