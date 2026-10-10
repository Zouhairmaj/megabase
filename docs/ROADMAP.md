# Roadmap

Megabase is built in the five levels in `GOAL.md`. A level does not start
until the previous one meets the conformance threshold in `PROGRESS.md`.
The GitHub Project **Megabase Backlog** is the live board; this page is the
plan those issues implement.

## Critical path to "apps run unmodified"

```
core JWT + error types
        │
        ├─► Auth SQL (auth.users, auth.uid(), auth.jwt())
        │         │
        │         ├─► signup / token / user / logout     ──► Level 1 Auth
        │         └─► (OAuth, OTP, magic link: Level 2)
        │
        └─► REST table CRUD
                  │
                  ├─► filters, order, limit, Prefer, RPC ──► Level 1 REST
                  │
                  └─► embedding, aggregates (same level, after CRUD)
```

Level 1 is the critical path. REST resource routes, `GET /{relation}` with
`eq` / `select` / `order`, and Auth email/password plus `auth.uid()` are
what most apps need before anything else. Admin APIs, media types and
exotic operators are Level 1 scope but not on the critical path.

## Levels

| Level | What ships | Threshold | Status |
|---|---|---|---|
| 1 | `/rest/v1` + `/auth/v1` email/password, JWT, `auth.users`, `auth.uid()`, `auth.jwt()` | ≥95% conformance, no P0 security issues | In progress (Auth admin GET/DELETE, health, settings, autoconfirm signup, logout; REST resource routes and horizontal filters) |
| 2 | OAuth, magic links, OTP, `/storage/v1` + `storage.objects` RLS | ≥90%, Level 1 held | Backlog |
| 3 | `/realtime/v1` (replication, broadcast, presence) | ≥85%, Levels 1–2 held | Backlog |
| 4 | `/functions/v1`, pooler, Postgres Meta, **Studio test** | ≥80%, all held | Backlog |
| 5 | Studio front end served from the binary (stretch; deferred) | feasibility first | Deferred |

## Views of the work

- **Board** (Project, grouped by Status): Backlog → Ready → In progress → In review → Done, plus Blocked.
- **Roadmap** (Project, grouped by Milestone): Level 1 through 5.

Level 1 issues sit in **Ready**, ordered by *how often real apps use it*
over *how far it is from conformant*. Everything else starts in **Backlog**.

## Website (later, not a compatibility level)

The public website follows the design-first rule: design in
[Kite](https://kite.new/p/megabase-identity), LLM committee review,
revisions, then implementation. It is a `type:feature` epic with no
level milestone. Every page header shows the workspace release beside
the wordmark: `v` plus `[workspace.package].version` from the root
`Cargo.toml`, read when the site is generated, linking to that GitHub
release tag. The Status page treemap is generated at build from the
same unit states as the README graphic. Published builds use the
latest Judge results from `main`.

## Regenerating the board

```bash
just backlog-dry    # print GitHub writes without applying them
just backlog        # cargo run -p megabase-backlog -- sync
```

The command is idempotent: it matches existing issues by the
`<!-- megabase-id: … -->` body marker and never recreates them. It sets
Project **Status** / Level / Component / Size / Priority via GraphQL
single-select option ids (not `--text`), links sub-issues and blocked-by,
and leaves live Status values (`In progress`, `In review`, `Blocked`,
`Done`) alone. GitHub write access is required (issues, project).

Board **Status** is kept in sync with branches, PRs and the `blocked` label
by `.github/workflows/board-sync.yml` (secret `PROJECT_TOKEN`). Agents still
claim the issue themselves before starting work; see `AGENTS.md`.

## Versioning and releases

This is the only copy of the policy. `CONTRIBUTING.md` and `AGENTS.md` point
here.

**1.0.0** is Supabase drop-in parity: Level 5 reached and judge-verified.
Do not mint 1.0.0 any other way (no `BREAKING CHANGE` / `type!` for that).

Pre-1.0 versions:

| Event | Version |
|---|---|
| Phase 0 (bootstrap) | `0.1.0` |
| Level 1 reached | `0.2.0` |
| Level 2 reached | `0.3.0` |
| Level 3 reached | `0.4.0` |
| Level 4 reached | `0.5.0` |
| Level 5 reached (parity) | `1.0.0` |

At each level gate the orchestrator lands a commit whose body includes a
`Release-As: 0.x.0` footer (or `Release-As: 1.0.0` at Level 5) so
release-please cuts that exact version. Between gates, `feat` / `fix` /
`conformance` only bump **patch**.

release-please (`release-please-config.json`):
`bump-minor-pre-major` is **false**, `bump-patch-for-minor-pre-major` is
**true**. Use `release-type: simple` (not `rust`): the rust strategy
walks workspace members and fails on `version.workspace = true`
([googleapis/release-please#2478](https://github.com/googleapis/release-please/issues/2478)).
A TOML extra-file updater bumps `[workspace.package].version` on the
file release-please read; member crates inherit. That read is cached by
branch name (`main`), not by SHA
([0030](decisions/0030-release-version-only.md)). A run can cache an
older `main` tree and then commit onto a newer SHA, so the cached
`Cargo.toml` overwrites dependencies that landed in between. The
lockfile job restores `Cargo.toml` from the merge-base with the default
branch, sets only `[workspace.package].version` from
`.release-please-manifest.json`, runs `cargo update -w`, and commits
`Cargo.toml` and `Cargo.lock` when those version fields changed.
When the release-please branch is already gone, that job checks out
the default branch and pushes the lockfile commit there. The 0.1.5
merge did not: Sync Cargo.lock exited 0 on a missing branch, and
`cargo --locked` failed because workspace packages in `Cargo.lock`
were still `0.1.4`.
`megabase-guard` rejects any other edit in those files, including a
registry package version in `Cargo.lock`. `--locked` CI
(Build, Codecov, Bencher, Protected paths, Judge) stays green. That
commit is pushed with the token the tree probe selected.
`RELEASE_PLEASE_CLASSIC_TOKEN` (classic PAT, scopes `repo` and
`workflow`) is used when it is set and its tree probe succeeds, so the
push to the release branch starts pull_request workflows. If that
secret is absent, or its probe returns HTTP 403 or 404 that is not a
rate limit, the job tries `RELEASE_PLEASE_TOKEN`. `GITHUB_TOKEN` is
selected only when both of those candidates are absent or denied the
same way. A fine-grained PAT
owned by `megabase-agent` cannot write this public repository.
[GitHub's token docs](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens)
say only a classic PAT has write access to a public repository owned by
another personal account, including for a collaborator. A denied push
is separate from that probe: if the selected token's `git push` returns
HTTP 403, the job pushes again with `GITHUB_TOKEN`. That retry does
not trigger `push`/`pull_request` workflows. The lockfile
job `workflow_dispatch`es CI, Bencher, and Judge on the release branch
unless that push used the classic or release token and the branch is the
default branch. Those three workflows run on `push` only for `main`.
A lockfile commit on `release-please--branches--*` still needs that
dispatch. A lockfile commit on the default branch does not: a classic
or release token push there starts the `push` workflows. The same dispatch runs when the probe selected `GITHUB_TOKEN`,
the push was retried with `GITHUB_TOKEN`, or the commit was not pushed. If that ref's workflow files lack
`workflow_dispatch`, it dispatches **Lockfile required checks** on the
default branch, which checks out the lockfile SHA and reports Build,
Codecov, Bencher, Protected paths, and Judge via the Checks API.
`GITHUB_TOKEN` is allowed to create `workflow_dispatch` runs. The
lockfile job then polls until Build, Codecov, Bencher, and Protected
paths have check runs on that SHA, and until Judge has started. A check
named `Judge` or `Judge (<service>)` counts as Judge. The aggregate job
named `Judge` starts only after the matrix jobs finish, so the wait
does not require that name. `Judge build` and `Judge services` do not
count. The window is 180 attempts of 6s by default (about eighteen
minutes) and is configurable via `LOCKFILE_CHECK_ATTEMPTS` and
`LOCKFILE_CHECK_SLEEP_SECONDS`
([0026](decisions/0026-lockfile-judge-matrix-checks.md)). PR titles use `chore: release ${version}` (no `main` scope);
`semantic-pr.yml` also allows scope `main` as a fallback.
Squash merges must use the pull request title as the commit subject
(`squash_merge_commit_title=PR_TITLE`). `COMMIT_OR_PR_TITLE` keeps a
single commit's subject, so a `[component] unit:` squash is not a
conventional commit and release-please skips it. Do not pass a
different `--subject` to `gh pr merge --squash`. A merged pull request
whose subject is already wrong can still be included by putting
`BEGIN_COMMIT_OVERRIDE` / `END_COMMIT_OVERRIDE` around one conventional
commit in the pull request body.
`bootstrap-sha` is the Phase 0 merge (`7aa41e8`, exclusive): commits
before it (`Day 0`, `[phase0]`, `[brand]`) are not conventional and
must not fail the Release job. `release-as` is `0.1.0` for that
bootstrap only. The v0.1.0 **release PR itself** must delete
`"release-as"` from `release-please-config.json` before it merges
(PROGRESS.md). The Release workflow runs again on that merge; leaving
the key would propose another `0.1.0`. Do not delete it before that PR
exists: without it, Phase 0 would cut `0.0.1` and the `rust` strategy
on `main` today fails with `package.version is not tagged`.
release-please ignores `bootstrap-sha` after the first release PR
merges. Do not add a `version.txt`.

Cadence:

- **Weekly:** merge the release PR on Monday if there are unpublished
  changes.
- **Immediately** when a Level gate is reached (`Release-As` footer).
- **Hotfix** for a severe regression, without waiting for Monday.

Only the **orchestrator** merges release PRs, and only after reviewer
approval of the current head SHA and green CI. The release PR's
`Cargo.toml` and `Cargo.lock` are version bumps only. The Release
workflow restores `Cargo.toml` from the merge-base and runs
`cargo update -w` on the release branch. If that commit is still
missing, do the same on that branch. Each GitHub Release body
is annotated with the coverage / conformance delta versus the previous
tag. The same Release workflow attaches musl-static linux `x86_64`
and `aarch64` binaries, `SHA256SUMS`, Sigstore signatures, and SLSA
provenance, and publishes `ghcr.io/zouhairmaj/megabase:<tag>` (cosign),
only when the run SHA is that release tag (the push that tags this
commit, or Release dispatched from the tag).
Install and verify those artifacts in [Install](install.md).
