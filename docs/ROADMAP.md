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

Level 1 is the critical path. REST `GET /{relation}` with `eq` / `select` /
`order` and Auth email/password plus `auth.uid()` are what most apps need
before anything else. Admin APIs, media types and exotic operators are
Level 1 scope but not on the critical path.

## Levels

| Level | What ships | Threshold | Status |
|---|---|---|---|
| 1 | `/rest/v1` + `/auth/v1` email/password, JWT, `auth.users`, `auth.uid()`, `auth.jwt()` | ≥95% conformance, no P0 security issues | In progress (HTTP 501; Auth SQL install started) |
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
level milestone. The Status page embeds the generated
`coverage/treemap.svg` / `coverage/treemap-light.svg` (same files, same
style as the README).

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
A TOML extra-file updater bumps `[workspace.package].version`; member
crates inherit. The Release workflow runs `cargo update -w` on
`release-please--branches--*` and commits `Cargo.lock` if workspace
member versions drifted, so `--locked` CI (Build, Codecov, Bencher,
Protected paths, Judge) stays green. release-please writes that PR with
`POST /git/trees` and `base_tree` set to `main`. A fine-grained PAT or
GitHub App token needs Contents, Pull requests, and Workflows (each
Read and write); a classic PAT needs `repo` and `workflow`. Without
Workflows write, GitHub returns 403 because the base tree contains
`.github/workflows`, and release-please prints only `Error adding to
tree`. The workflow probes that call. When `RELEASE_PLEASE_TOKEN`
fails it, the release PR update uses `GITHUB_TOKEN` (`contents: write`
on that job) and the log names the missing scopes (`HUMAN_LOG.md`).
The lockfile commit is still pushed with `RELEASE_PLEASE_TOKEN` when
the secret is set: a user or GitHub App push starts workflows, and a
`GITHUB_TOKEN` push does not. When the secret is unset, or that
lockfile commit was not pushed, the lockfile job `workflow_dispatch`es
CI, Bencher, and Judge on the release branch
(native check runs on that SHA). If that ref's workflow files lack
`workflow_dispatch`, it dispatches **Lockfile required checks** on the
default branch, which checks out the lockfile SHA and reports Build,
Codecov, Bencher, Protected paths, and Judge via the Checks API.
`GITHUB_TOKEN` is allowed to create `workflow_dispatch` runs. The
lockfile job then polls until those five names have started on that
SHA, not merely until any check run exists. The window is 60 attempts
of 6s by default (about six minutes) and is configurable via
`LOCKFILE_CHECK_ATTEMPTS` and `LOCKFILE_CHECK_SLEEP_SECONDS`. PR titles use `chore: release ${version}` (no `main` scope);
`semantic-pr.yml` also allows scope `main` as a fallback.
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
approval of the current head SHA and green CI. The lockfile is part of
the release PR (`cargo update -w` on the release branch). If it is still
stale, run `cargo update -w` on that branch. Each GitHub Release body
is annotated with the coverage / conformance delta versus the previous
tag. The same Release job attaches musl-static linux `x86_64` and
`aarch64` binaries, `SHA256SUMS`, Sigstore signatures, and SLSA
provenance, and publishes `ghcr.io/zouhairmaj/megabase:<tag>` (cosign).
Install and verify those artifacts in [Install](install.md).
