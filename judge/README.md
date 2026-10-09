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
server), and can start Megabase on host port 8100.

## Commands

From the repository root:

```bash
just judge-up     # reference stack + Megabase; wait until healthy
just judge        # wait, run cases, write coverage/judge-results.json
just judge-down
```

Equivalent:

```bash
docker compose -p megabase-judge \
  -f vendor/supabase/docker/docker-compose.yml \
  -f judge/compose.override.yml \
  --env-file vendor/supabase/docker/.env.example \
  --profile with-megabase up -d --build --wait --wait-timeout 300

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
baseline now fails, or the passing count dropped. New failures of cases
that never passed do not fail the job; they stay grey on the treemap.

CI starts only the reference stack in Docker and runs Megabase on the
runner (no image build). Local `just judge-up` builds the Megabase image.

## Cases

A case is a TOML table with an id, the `coverage/units.json` ids it
exercises, and one or more steps (`method`, `path`, optional `json` /
`headers` / `key` / `capture` / `ignore`). See `judge/cases/`.

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

**Database side-effects.** After a mutating case, snapshot `auth.users`,
`storage.objects` and the public fixture tables on both databases and
compare. HTTP equality is not enough for sign-up or uploads.

**Concurrency.** Same email signed up ten times, parallel refresh, parallel
upload to one path. The reference's winner/loser pattern is the spec.

**Fault injection.** Kill the Postgres connection, delay queries, fill the
storage volume. Compare error codes, not just happy paths.

**Adversarial.** Forged JWTs, `alg=none`, RLS filter injection, pooler
tenant isolation. Level 1 Auth and REST carry the first of these.

Studio scenarios, when they exist, compare every network call Studio makes,
visible UI errors, and the final database state. A `MEGABASE_NOT_IMPLEMENTED`
from those calls is appended to the priority list in `PROGRESS.md`.
