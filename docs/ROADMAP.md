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
| 1 | `/rest/v1` + `/auth/v1` email/password, JWT, `auth.users`, `auth.uid()`, `auth.jwt()` | ≥95% conformance, no P0 security issues | In progress (501 everywhere) |
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
crates inherit. `bootstrap-sha` is the Phase 0 merge on `main` so
bootstrap history is not dumped into a 0.0.1 changelog; release-please
ignores it after the first release PR merges. Phase 0's `0.1.0` still
needs a `Release-As: 0.1.0` footer from the orchestrator. Do not add a
`version.txt`.

Cadence:

- **Weekly:** merge the release PR on Monday if there are unpublished
  changes.
- **Immediately** when a Level gate is reached (`Release-As` footer).
- **Hotfix** for a severe regression, without waiting for Monday.

Only the **orchestrator** merges release PRs, and only after reviewer
approval of the current head SHA and green CI. If the release PR's
lockfile is stale, regenerate it with `cargo generate-lockfile` on that
PR. Each GitHub Release body is annotated with the coverage /
conformance delta versus the previous tag.
