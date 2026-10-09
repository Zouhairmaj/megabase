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
cargo run -p megabase-backlog -- sync
```

The command is idempotent: it upserts milestones, labels, Project fields,
epics and task issues from `coverage/units.json` plus the infra / judge /
spec / website epics. GitHub write access is required (issues, project).

Board **Status** is kept in sync with branches, PRs and the `blocked` label
by `.github/workflows/board-sync.yml` (secret `PROJECT_TOKEN`). Agents still
claim the issue themselves before starting work; see `CONTRIBUTING.md`.
