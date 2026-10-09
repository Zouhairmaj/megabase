# Judge

The judge is the external arbiter of correctness (MANIFESTO.md rule 4).
It runs the official self-hosted Supabase stack from the pins in
`vendor.toml` next to Megabase, sends the same requests to both, and
compares status, selected headers and normalized bodies.

Agents do not grade themselves. Changes to this directory happen only on a
`review/*` branch, reviewed by the reviewer agent
(`docs/adr/0003-protected-paths.md`). If you think the judge is wrong,
write it up under "Judge disputes" in `PROGRESS.md` and move on.

## Layout

```
judge/
  compose.override.yml   overlay on vendor/supabase/docker/docker-compose.yml
  fixtures/schema.sql    extra schema loaded into the reference database
  cases/*.toml           visible cases (unit ids from coverage/units.json)
  harness/               megabase-judge (Rust, ureq)
  NORMALIZATION.md       rules applied to both responses
```

The overlay retags images to the pins, uses named volumes so nothing is
written into `vendor/`, turns on Auth autoconfirm (the stack has no mail
server), and can start Megabase on host port 8100 against a dedicated
`megabase` database.

## Commands

From the repository root:

```bash
just judge-up     # reference stack, prepare megabase DB, Megabase
just judge        # wait, run cases, write coverage/judge-results.json
just judge-down
```

Equivalent:

```bash
docker compose -p megabase-judge \
  -f vendor/supabase/docker/docker-compose.yml \
  -f judge/compose.override.yml \
  --env-file vendor/supabase/docker/.env.example \
  up -d --wait --wait-timeout 300

cargo run -p megabase-judge -- prepare
# then start Megabase with DATABASE_URL=.../megabase (compose profile
# with-megabase, or the host binary in CI)

cargo run -p megabase-judge -- wait
cargo run -p megabase-judge -- run \
  --cases judge/cases \
  --out coverage/judge-results.json \
  --baseline coverage/judge-results.json \
  --summary "$GITHUB_STEP_SUMMARY"
```

`--env-file` is the upstream demo file. Never point the judge at production
data. After a run, `git -C vendor/supabase status` must stay clean.

The harness exits non-zero on a **regression**: a case that passed in the
baseline now fails. A drop in the total passing count does not fail the
job by itself. New failures of cases that never passed stay grey on the
treemap.

CI starts only the reference stack in Docker and runs Megabase on the
runner (no image build). Local `just judge-up` builds the Megabase image.

## Cases

A case is a TOML table with an id, the `coverage/units.json` ids it
exercises, and at least one HTTP `[[case.step]]` (`method`, `path`,
optional `json` / `headers` / `key` / `capture` / `ignore`) and/or one
`[[case.db]]` check (`table` or `function`, optional `rows = true`).
See `judge/cases/`.

## Database side-effects

The harness opens two PostgreSQL connections (it does not import Megabase
crates):

| Side | Default URL |
|---|---|
| Reference | `postgres://postgres:…@127.0.0.1:54322/postgres` |
| Megabase | `postgres://postgres:…@127.0.0.1:54322/megabase` |

Host `5432` is Supavisor. The overlay publishes Postgres itself on
`54322` so `prepare` can `CREATE DATABASE` and so snapshots do not go
through the pooler.

`prepare` creates the `megabase` database on the official cluster and
loads `judge/fixtures/schema.sql` into it. Megabase then installs its
Auth SQL there (`DATABASE_URL`). Sharing the official `postgres`
database would make row and catalog comparisons vacuous.

After every mutating HTTP case (POST / PUT / PATCH / DELETE), the
harness snapshots `auth.users` and `public.todos` on both databases
before and after the HTTP steps and compares the per-case row delta
(added/removed). Leftover rows from an earlier case or a reused volume
do not fail a later case. Set `snapshot = []` to skip, or
`snapshot = ["schema.table", …]` to choose other relations.
`storage.objects` is Level 2.

`[[case.db]]` compares a table catalog (columns, nullability, generated
expressions, RLS flag, `pg_get_indexdef`, `pg_get_constraintdef`) or a
function (arguments, result type, language, volatility, normalized body)
so `auth:sql-table:*` and `auth:sql-function:*` units can become
conformant. A required object missing on both databases fails the case
(a typo must not look like a pass). Set `absent = true` when the pin
dropped the object (`auth.sso_sessions`): both databases must lack it,
or the case fails. A missing fixture snapshot table (`auth.users`,
`public.todos`) on the reference stack still aborts the run.

Studio browser sessions (`judge/studio/`, a Rust harness such as
fantoccini or chromiumoxide driving Chromium against official Studio
twice) are **planned**: they land with Level 4, on a `review/*` branch,
and are the Studio test in GOAL.md section 9. The runner is Rust-only.

## Accepted design (not all built)

Approved by the agent coordinator with proposal 0001. Built incrementally;
none of these weaken existing cases.

**Hidden tests.** A second, private suite the agents cannot read. Run
weekly; only pass/fail counts are published. Stops overfitting to the
visible TOML.

**Database side-effects.** Built for Level 1: `auth.users` and
`public.todos` after mutating cases, plus `[[case.db]]` catalog checks.
`storage.objects` waits for Level 2.

**Concurrency.** Same email signed up ten times, parallel refresh, parallel
upload to one path. The reference's winner/loser pattern is the spec.

**Fault injection.** Kill the Postgres connection, delay queries, fill the
storage volume. Compare error codes, not just happy paths.

**Adversarial.** Forged JWTs, `alg=none`, RLS filter injection, pooler
tenant isolation. Level 1 Auth and REST carry the first of these.

Studio scenarios, when they exist, compare every network call Studio makes,
visible UI errors, and the final database state. A `MEGABASE_NOT_IMPLEMENTED`
from those calls is appended to the priority list in `PROGRESS.md`.
