# Contributing to Megabase

Agents follow `GOAL.md`. This file is the equivalent for anyone (agent or
human) writing code, docs or design.

## Sources of truth

| What | Where |
|---|---|
| Mission and guardrails | `MANIFESTO.md`, `GOAL.md` (humans only) |
| What is being worked on | GitHub Project **Megabase Backlog** |
| Decisions and overall state | `PROGRESS.md` |
| Compatibility contract | `docs/COMPATIBILITY.md` |
| Graphic and UI design | [Kite: Megabase identity](https://kite.new/p/megabase-identity) |
| Coverage denominator | `coverage/units.json` (extracted from `vendor/`) |
| Correctness | `judge/` vs the pinned reference stack |

## Design-first (graphics and UI)

**All UI goes through Kite first.** Decided by the agent coordinator.
Recorded here, in `docs/brand/README.md` and in `PROGRESS.md` under
Decisions.

1. **Kite is the source of truth** for every graphic and UI element: logos,
   badges, treemap styles, diagrams, the project website, Studio-related UI
   and social images. Agents design in
   [the Megabase identity file](https://kite.new/p/megabase-identity)
   (public mockups). When the repo and that file differ, the Kite file
   wins and the repository is updated.
2. **LLM committee, then implement.** Every UI change is reviewed by
   several external LLMs (the committee) in Kite. Apply the corrections
   there, and only then implement in this repository. Do not draw UI
   directly in code.

The Website epic on the backlog is the first work that follows this
workflow end to end. Its Status page embeds the generated Status treemap
(`coverage/treemap.svg` / `coverage/treemap-light.svg`), the same files as
the README.

## Loop

1. Take the top **Ready** issue for your role, move it to **In progress**.
2. If `specs/<component>/<unit>.md` does not exist, write it from `vendor/`.
3. Implement in Rust. Credit the upstream file and license in a header.
4. Run `just judge` (or the documented equivalent). Keep the commit only if
   conformance does not drop.
5. Open a PR with `Closes #<issue>` and the coverage / conformance deltas.

Branch names: `issue-<number>-<slug>`. Pull requests are **squash-merged**;
the PR title is the changelog entry. CI requires a
[Conventional Commits](https://www.conventionalcommits.org/) title:

```
<type>(<scope>)?: <description>
```

Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `ci`, `build`,
`chore`, `conformance`.

Optional scopes: `rest`, `auth`, `realtime`, `storage`, `functions`,
`pooler`, `meta`, `studio`, `judge`, `coverage`, `site`.

`conformance` is for judge-case and reference-comparison work; it lands in
the **Conformance/judge** changelog section. Releases are cut by
release-please (v4, rust, `bump-minor-pre-major`) from `main`; the single
version is `workspace.package.version` in `Cargo.toml` (initial `0.0.0`).
Each GitHub Release body is annotated with the coverage / conformance
delta versus the previous tag. The rust strategy bumps
`workspace.package.version`; if the release PR's lockfile is stale, regenerate
it with `cargo generate-lockfile` on that PR.

GOAL.md section 8 still shows an older `[component] unit: …` example. That
file is human-owned and frozen. Agents follow this file and the CI check.

Include coverage and conformance deltas in the PR body
(`coverage X%→Y%, conformance A%→B%`).

## Commands that exist today

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo run -p megabase-coverage -- check
cargo run -p megabase-guard -- --base origin/main --head HEAD --head-ref "$BRANCH"
```

The judge and the backlog sync are documented in `justfile`.

## Rust only

Product code is Rust. Configuration, compatibility SQL, Studio assets if a
Rust UI framework requires them, and SVG/Markdown are the exceptions. Do
not add Python, TypeScript, Go, Elixir or other language source outside
`vendor/`.
