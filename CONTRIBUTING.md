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

1. Take the top **Ready** issue for your role. **Before writing code:** assign
   yourself (`gh issue edit <n> --add-assignee @me`) and move it to
   **In progress**. Do not start unclaimed work.
2. If `specs/<component>/<unit>.md` does not exist, write it from `vendor/`.
3. Implement in Rust. Credit the upstream file and license in a header.
4. Run `just judge` (or the documented equivalent). Keep the commit only if
   conformance does not drop.
5. Open a PR whose body contains `Closes #<n>` and the coverage / conformance
   deltas.

Branch names: `issue-<number>-<slug>` (example: `issue-42-rest-eq`). Pull
requests are **squash-merged**; the PR title is the changelog entry. CI
requires a [Conventional Commits](https://www.conventionalcommits.org/) title:

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

## Board status

The GitHub Project **Megabase Backlog** Status field must always match
reality. Columns: **Backlog**, **Ready**, **In progress**, **In review**,
**Blocked**, **Done**. GOAL.md §5.2 and §10 already require claiming an
issue before coding; this section is the operational contract because
GOAL.md is human-owned and frozen.

**Before starting a task the agent:**

1. Assigns (claims) the issue and moves it to **In progress**.
2. Creates branch `issue-<n>-<slug>` from `main`.
3. Puts `Closes #<n>` in the PR body.
4. When stuck, adds label `blocked` and comments the reason. Remove the
   label when work can continue. (`needs-human` stays the only way to ask
   a human; answers go in `HUMAN_LOG.md`.)

`.github/workflows/board-sync.yml` mirrors this with GraphQL. It uses
repo secret `PROJECT_TOKEN` (classic PAT, `project` scope — the default
`GITHUB_TOKEN` cannot set Project v2 fields):

| Trigger | Status |
|---|---|
| Branch `issue-<n>-*` created, or issue assigned | **In progress** |
| PR opened ready for review, or marked ready | **In review** |
| PR converted to draft | **In progress** |
| PR closed unmerged | **In progress** if still claimed (assignee, `issue-<n>-*` branch, or draft PR); otherwise **Ready** |
| Label `blocked` added | **Blocked** |
| Label `blocked` removed | Derived from remaining signals (usually **In progress**) |
| PR merged, or issue closed | **Done** |

An hourly reconcile (and `workflow_dispatch`) walks every project item and
corrects drift. It never promotes **Backlog** → **Ready**; the orchestrator
fills Ready. Fork PRs and a missing `PROJECT_TOKEN` skip the workflow
without failing CI.

Suggested one-line addition for a human to put in GOAL.md §10
(agents cannot edit that file): *Board Status is kept in sync by
`.github/workflows/board-sync.yml` (secret `PROJECT_TOKEN`).*

## CodeRabbit

CodeRabbit is installed (`.coderabbit.yaml`: profile `assertive`,
`en-US`, auto-review on, `vendor/**` ignored). Agents **must address
CodeRabbit comments before merge**: resolve each thread with a fix or a
short explanation of why it does not apply. Do not squash-merge while
unresolved CodeRabbit review comments remain.

CodeRabbit enforces the same GOAL.md rules as this file: `vendor/` and
`judge/` are protected, product code is Rust-only, unimplemented routes
return 501 `MEGABASE_NOT_IMPLEMENTED`, tests are not weakened to pass CI,
and upstream files are credited.

## Commands that exist today

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo bench --locked --bench health
cargo run -p megabase-coverage -- check
cargo run -p megabase-guard -- --base origin/main --head HEAD --head-ref "$BRANCH"
```

The judge and the backlog sync are documented in `justfile`.

## Rust only

Product code is Rust. Configuration, compatibility SQL, Studio assets if a
Rust UI framework requires them, and SVG/Markdown are the exceptions. Do
not add Python, TypeScript, Go, Elixir or other language source outside
`vendor/`.
