# AGENTS.md — Megabase

Agents rebuild every Supabase component in Rust as one binary, checked by a
judge against the real, pinned Supabase stack. **[`GOAL.md`](GOAL.md) is the
mission, levels, roles and iteration loop; this file only says how to carry
them out in this repository.** It never relaxes GOAL.md or `MANIFESTO.md`.

Cursor Cloud Agent environment: [`.cursor/environment.json`](.cursor/environment.json) (install, start, and base image). Do not duplicate those commands here.

**At the start of every session**, read `GOAL.md` (and `MANIFESTO.md` once),
`PROGRESS.md`, `coverage/summary.json` and the
[board](https://github.com/users/Zouhairmaj/projects/1). Decisions are
files in `docs/decisions/` (`NNNN-slug.md`); add a new file instead of
editing a shared list. Before you edit a directory, read its own
`AGENTS.md` if it has one (`crates/`, `judge/`, `tools/`, `site/`). Nested
files add local detail and never loosen a rule here. Do not assume your
tool loads them automatically.

## Commands

Run every command from the repo root. All were verified; the judge also
needs Docker.

```bash
# setup
rustup toolchain install stable && rustc --version   # need >= 1.89 (MSRV); CI uses stable
cargo install just --locked
git submodule update --init --recursive              # vendor/ pins; coverage and judge need them

# iterate — Rust workflow, below
just fmt                              # cargo fmt --all
just run                              # serves :8000 (MEGABASE_PORT / MEGABASE_HOST)
just fuzz jwt                         # cargo-fuzz (nightly); also gateway_http, rest_query

# judge: when served behavior changed (Docker)
git fetch origin main && git restore --source=origin/main --worktree -- coverage/judge-results.json
just judge-up
just judge                            # prints the summary; exits non-zero on a regression
just judge-down                       # always run it, even if a step above failed

# finalize every iteration, in this order
git restore --source=HEAD --staged --worktree -- coverage/judge-results.json   # CI owns it
just coverage                         # regenerate generated files
just ci                               # fmt-check, clippy -D warnings, tests, coverage-check
cargo run --locked -p megabase-coverage -- verify-pins
git commit ...                        # message format: Task workflow step 7
git fetch origin main && just guard   # protected-path policy on the committed diff
git push -u origin HEAD
```

PR checks: **Build, MSRV 1.89, Coverage check, Protected paths, Container
image, Judge, Conventional Commits title, Fuzz, cargo-vet, cargo-machete,
cargo-hack**. GitHub does not enforce them yet.
Treat every one as required anyway. A pull request that only changes the
static site and site-only docs runs the site build and reports those other
checks as success without compiling the server
([Pull request checks](docs/contributing.md#pull-request-checks)). Pushes
to `main` and release dispatch still run the full set.

`fuzz/` is a standalone cargo-fuzz workspace (excluded from the root
workspace). Targets: `jwt` (`megabase-core` HS256 + `bearer_token`),
`gateway_http` (URI / Kong prefix matching), `rest_query` (stub query-string
walker until PostgREST filter parsing exists). Scorecard's Fuzzing check
detects `libfuzzer_sys` in those `*.rs` files. `.github/workflows/fuzz.yml`
runs each target for 60 seconds on pull requests that are not site-only, and 10 minutes on a schedule, and
uploads `fuzz/artifacts/` on a crash. Needs nightly and `cargo-fuzz` 0.13.2;
see [Contributing](docs/contributing.md#fuzzing).

## Rust workflow

Iterate with `cargo check -p <crate>` and
`cargo clippy -p <crate> --all-targets --locked -- -D warnings`. Run full
tests (`just ci`) only before pushing.

Share state as `Arc<AppState>` and borrow it. Do not call `.clone()` to
silence the borrow checker. The clone and borrow lints in
`[workspace.lints.clippy]` are deny. Every workspace crate sets
`lints.workspace = true`. `[workspace.lints.rust]` forbids `unsafe_code`.
`site/` repeats that lint (it is not a workspace member). `fuzz/` does
not: `libfuzzer_sys::fuzz_target!` expands to `unsafe`.

Behaviour is the pinned source in `vendor/` and the unit spec in `specs/`.
GOAL.md still decides disagreements: source beats docs, and the reference
stack beats source.

```rust
async fn get_todo(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Todo>, Error> {
    let todo = state.store.get(&id).await?;
    Ok(Json(todo))
}
```

A new external crate is rejected until its name is on the `[bans] allow`
list in `deny.toml`. The comment there is how to propose one.

**Generated files. Never hand-edit them; run the generator.**
- `just coverage` writes `coverage/**`, the
  `<!-- …:begin -->…<!-- …:end -->` blocks in `README.md`, `PROGRESS.md` and
  `docs/COMPATIBILITY.md`, and `docs/epics/**`.
- `coverage/judge-results.json` is written by `just judge` but owned by CI
  on `main`. Feature PRs never change it.
- `cargo run --locked -p megabase-backlog -- plan` writes `docs/backlog/PLAN.md`.

## Rust API style

Two references, not a second style guide:

- [Pragmatic Rust Guidelines](https://microsoft.github.io/rust-guidelines/guidelines/index.html) (Microsoft)
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)

- **Newtypes.** A domain value is its own type with a private field. A newtype that is not total is built with a fallible constructor (`M-STRONG-TYPES`, `M-STRONG-TYPES-GUARD`, `C-NEWTYPE`).
- **Errors.** Library crates expose a dedicated error type (`thiserror`) that implements `std::error::Error`, and convert with `From`. Binaries may use `anyhow`. Do not use `String` as the error (`M-ERRORS-CANONICAL-STRUCTS`, `C-GOOD-ERR`).
- **Panics.** A panic stops the process. Bad input and I/O return `Result`. Panic only when an internal invariant is already broken (`M-PANIC-IS-STOP`, `M-PANIC-ON-BUG`). Parsers and JWT verification stay panic-free on arbitrary bytes (the `fuzz/` targets).
- **Docs.** Public items start with one summary sentence. Add `# Errors` and `# Panics` when they apply (`M-CANONICAL-DOCS`, `C-FAILURE`).

## Who may change what

| Path | Rule |
|---|---|
| `crates/` | product code: one crate per component, plus `megabase-core` (shared), `megabase-server` (gateway), `megabase` (binary) |
| `specs/<component>/<unit>.md` | written from `vendor/` before the code |
| `tools/megabase-coverage`, `tools/megabase-backlog`, `site/` | see their `AGENTS.md` |
| `PROGRESS.md` | current state, blocked items, judge disputes |
| `docs/decisions/` | one file per decision (`NNNN-slug.md`). Add a file; do not edit a shared list |
| `docs/adr/` | architecture decisions. Changing a GOAL.md §4 choice needs an ADR |
| `judge/`, `.github/`, `tools/megabase-guard/`, `CODEOWNERS` | **only on a `review/*` branch**, approved by the reviewer agent ⛔ |
| `vendor/`, `vendor.toml`, `.gitmodules` | **never**. This is the frozen spec; only humans bump pins ⛔ |
| `GOAL.md`, `MANIFESTO.md`, `HUMAN_LOG.md` | **humans only** ⛔ |

⛔ marks paths the `Protected paths` check rejects. The only exception is the
self-expiring Phase 0 bootstrap case in
[ADR 0003](docs/adr/0003-protected-paths.md). CODEOWNERS also requests the
maintainer's review. Branch protection does not require that review today,
so reviewer-agent approval plus green checks allow the merge. If GitHub
starts requiring it, add `needs-human` and wait. Never route around it.

## Task workflow

The board must match reality at all times.

1. **Pick** the top **Ready** issue for your role and the current level.
   Prefer finishing started work. Take one issue at a time.
2. **Claim it before any work, including reading or spec work.**
   ```bash
   N=<issue>; U=https://github.com/Zouhairmaj/megabase/issues/$N; R=Zouhairmaj/megabase
   gh issue view $N -R $R --json assignees --jq '.assignees|length'   # must be 0
   gh issue edit $N -R $R --add-assignee @me
   gh issue comment $N -R $R --body "CLAIMED by <role>/<session-id>"
   ITEM=$(gh project item-list 1 --owner Zouhairmaj --format json --limit 1000 \
     --jq ".items[]|select(.content.url==\"$U\")|.id")
   [ -n "$ITEM" ] || ITEM=$(gh project item-add 1 --owner Zouhairmaj --url "$U" --format json --jq .id)
   gh project item-edit --project-id PVT_kwHOABCGZ84BmU5I --id "$ITEM" \
     --field-id PVTSSF_lAHOABCGZ84BmU5Izhk-YlM --single-select-option-id 47fc9ee4
   ```
   Status ids: Ready `c7446441`, In progress `47fc9ee4`, In review
   `7d8196e7`, Blocked `ea5545f1`, Done `98236657`. Rerun `item-edit` with
   the matching id at every later transition. Agents share one GitHub
   account, so the `CLAIMED` comment identifies who holds the claim. If
   there are two, the earlier one wins and the later agent picks another
   issue. To resume your own claim, comment `RESUMED by …`. If the board
   update fails, comment on the issue and do not start untracked work.
3. **Branch** from `main` as `issue-<n>-<slug>`. For changes to ⛔ review
   paths, use `review/issue-<n>-<slug>`.
4. **Spec:** if `specs/<component>/<unit>.md` is missing, write it from
   `vendor/`. Cover inputs, outputs, errors, edge cases and the upstream
   `file:line`.
5. **Implement** in Rust. Mark the code that serves a unit with
   `// megabase:unit <id>`, using ids from `coverage/units.json`.
6. **Verify** with the command sequence above. If conformance drops, revert
   and record what you tried on the issue. Never invent a number: without
   Docker, write `conformance not measured` and let the Judge check measure it.
7. **Commit** once per iteration, in GOAL.md §3 rule 8 format:
   `[component] unit: what changed (coverage X→Y, conformance A→B)`.
   **Open the PR** with
   `gh pr create -R $R --base main --title "<type>(<scope>): <lowercase subject>" --body-file <f>`,
   then set Status = In review. The title becomes the squash commit and the
   changelog entry. Allowed types and scopes are listed in
   `.github/workflows/semantic-pr.yml`, for example `feat(rest): add eq filter`.
   Leave out the scope for core, server or repo-wide changes.
   The body follows `.github/pull_request_template.md`, with `Closes #<n>`
   and the measured deltas. Keep the PR draft while work is in progress.
   **As soon as the work is complete and CI is green, mark it ready for
   review** — do not leave a finished PR in draft.
8. **Judge cases** for your units go in a separate PR on
   `review/issue-<n>-cases`, cross-linked to the feature PR. Each case must
   pass on the reference stack. The case PR merges first; then rebase the
   feature PR and rerun its checks.
9. **Blocked:** add the `blocked` label and a comment with the reason, set
   Status = Blocked, add a line under "Blocked items" in `PROGRESS.md`, and
   pick another issue. Stop after 3 failed attempts at the same failure.
   Keep the board current whenever you pause, hand off or resume.

## Definition of done

- [ ] Every PR check is green.
- [ ] New behavior has tests in its crate; a bug fix has a test that fails
      without the fix. The judge shows no regression.
- [ ] `just coverage` output is committed. Markers exist only for units you
      actually serve.
- [ ] The spec is written, ported code carries its credit header, and
      `NOTICE`/`LICENSES/` cover any new upstream.
- [ ] Docs are **edited in place**: rewrite the affected section, and never
      append text that restates existing content. One fact, one place.
- [ ] An issue comment records what changed and what you learned.
      `PROGRESS.md` holds state that outlives the issue (blocked items,
      judge disputes). A decision is a new file
      `docs/decisions/NNNN-slug.md`, not a new row in a shared list.

## Hard rules

**Never**
- Touch a ⛔ path outside its rule. If you think the judge is wrong, write
  it up under "Judge disputes" in `PROGRESS.md`, add the `judge-dispute`
  label, and move on.
- Add non-Rust source (`.py .js .ts .go .sh …`) outside `vendor/` ⛔.
  TOML, YAML, compatibility SQL, Markdown and SVG are allowed.
- Game the score: skip, `#[ignore]`, loosen or delete a test or case, add a
  marker for behavior you do not serve, or branch on judge inputs.
- Guess. Return HTTP 501 through
  `megabase_core::MegabaseNotImplemented::new(component, unit)` (GOAL.md §3 rule 5).
- Commit or print secrets, tokens or real data. The judge uses only
  `vendor/supabase/docker/.env.example` and fixtures. If push protection
  blocks a push, remove the secret; never bypass it.
- Force-push `main`, rewrite another agent's branch, or merge your own PR.
- Build UI before it is designed. Follow
  [GOAL.md Design (design-first, Kite)](GOAL.md). Link the Kite design in
  the PR when adding or changing a layout or component.
- Describe the visual identity in any terms other than those in
  `docs/brand/README.md`, or attribute it to another brand or product.

**Always**
- Port from `vendor/`. Source beats docs; the reference stack beats source.
  Ported code starts with this header:
  `// Ported from <upstream repo> <path> (<license>), pin <tag from vendor.toml>.`
- Write public text (comments, PRs, issues, docs) for readers who never saw
  your session. Record failed approaches and decisions briefly, and leave out
  prompt transcripts and private context.
- Write repository content in English.
- Follow [versioning and releases](docs/ROADMAP.md#versioning-and-releases).
  Do not invent a 1.0.0 or a minor bump; level gates use a `Release-As:`
  footer from the orchestrator.
- Keep docs current. Follow [GOAL.md Documentation](GOAL.md).
- Follow the secure-design record in [`docs/SECURE_DESIGN.md`](docs/SECURE_DESIGN.md)
  when changing authentication, cryptography, SQL, or request handling.

**Ask first**
- New broad-impact dependencies or MSRV changes: open an issue with your
  rationale, and get orchestrator and reviewer approval first.
- Human-only actions (repo settings, branch protection, secrets, app
  installs): open an issue labeled `needs-human`, list it under "Human-only
  actions" in `PROGRESS.md`, and continue with other work. Humans do the
  action and log it in `HUMAN_LOG.md`.

## Review and merge

- **Reviewer agent.** It is independent and has not seen the code under
  review. It applies the rules below to every PR and never merges. It uses
  `gh pr review <n> --approve` or `--request-changes`. GitHub does not let
  an account approve its own PR, so when the reviewer shares the author's
  account it posts a comment instead:
  `REVIEW: APPROVED <head-sha>` or `REVIEW: CHANGES REQUESTED <head-sha>`,
  followed by its findings.
- **CodeRabbit** reviews every PR (including drafts). **CodeQL** / GitHub
  Advanced Security does too. Fix each finding or dismiss it with a reason.
  Leave no unresolved CodeRabbit threads and no open CodeQL alerts.
- **Claude Code.** `@claude` mentions from `megabase-agent` only, via
  `.github/workflows/claude.yml` (60-minute timeout). Job permissions are
  `contents: write`, `pull-requests: write`, `issues: write`,
  `id-token: write`, and `actions: read`. When Claude implements an issue
  it opens the pull request itself with `gh pr create` (or
  `mcp__github__create_pull_request`) after pushing — it does not stop at a
  Create-a-PR compare link. If a PR for the branch already exists, it
  updates that PR (`gh pr edit`) instead of opening another. The title is
  Conventional Commits; the body follows
  `.github/pull_request_template.md` and includes `Closes #<n>`. The
  workflow prefers `MEGABASE_AGENT_GH_TOKEN` so CI runs automatically
  (`GITHUB_TOKEN` can open a PR, but its `pull_request` workflow runs
  require approval).
- **Orchestrator.** It alone merges (`gh pr merge <n> --squash`), and only
  when three things hold: the approval covers the current head SHA
  (`gh pr view <n> --json headRefOid`), no change request is open, and every
  check is green for that revision. Any new push requires a new review.
  After merging, it sets Status = Done. It also keeps Ready filled
  (`cargo run --locked -p megabase-backlog -- plan`, then `just backlog`) and
  writes `devlog/` and the weekly report (GOAL.md §10–11).
- **Changes to this file.** Propose them under "Suggested AGENTS.md changes"
  in your PR body. They land in their own `docs` PR.

## Code Review Rules

Flag any breach of the Hard rules, especially:
- protected-path diffs on the wrong branch type
- weakened or removed tests or cases, markers without behavior, judge-specific branches
- plausible responses where a 501 belongs
- ported code without its header, or a new upstream missing from `NOTICE`
- hand edits to generated files, or duplicated doc content

Leave formatting and lint to CI.
