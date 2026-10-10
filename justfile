# Megabase commands. Only recipes that actually work are listed.
# Canonical names AGENTS.md will reference: build, test, lint, judge, coverage.

default:
    @just --list

build:
    cargo build --release --locked -p megabase

run:
    cargo run -p megabase

test:
    cargo test --workspace --locked

lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

# Extract units, render treemaps/badges, splice README / COMPATIBILITY / PROGRESS.
coverage:
    cargo run --locked -p megabase-coverage -- update

coverage-check:
    cargo run --locked -p megabase-coverage -- check

# Reject protected-path changes (see docs/adr/0003-protected-paths.md).
guard:
    cargo run --locked -p megabase-guard -- --base origin/main --head HEAD --head-ref "$(git branch --show-current)"

# Official stack + Megabase. Writes nothing into vendor/.
compose := "docker compose -p megabase-judge -f vendor/supabase/docker/docker-compose.yml -f judge/compose.override.yml --env-file vendor/supabase/docker/.env.example"

judge-up:
    {{compose}} up -d --wait --wait-timeout 300
    cargo run --locked -p megabase-judge -- prepare
    {{compose}} --profile with-megabase up -d --build --wait --wait-timeout 300
    @git -C vendor/supabase status --short || true

judge-down:
    {{compose}} --profile with-megabase down

# Compare both stacks. Requires judge-up, or a host Megabase on :8100 and the reference on :8000.
judge:
    cargo run --locked -p megabase-judge -- wait
    cargo run --locked -p megabase-judge -- run --cases judge/cases --out /tmp/judge-results.json --baseline coverage/judge-results.json --summary /tmp/judge-summary.md
    @cat /tmp/judge-summary.md

# Held-out suite. Needs MEGABASE_JUDGE_HIDDEN_SEED. Prints pass/fail counts only.
judge-hidden:
    cargo run --locked -p megabase-judge -- hidden --out /tmp/judge-hidden.json --summary /tmp/judge-hidden.md

# Upsert GitHub milestones, labels, Project fields (needs issues+project write).
# Matches existing issues by <!-- megabase-id -->; never recreates them.
backlog:
    cargo run --locked -p megabase-backlog -- sync

backlog-dry:
    cargo run --locked -p megabase-backlog -- sync --dry-run

# Gateway smoke benches (Criterion). CI tracks them with Bencher.
bench:
    cargo bench --locked --bench health

# Advisories on every Cargo.lock we own (not vendor/).
audit:
    cargo audit --file Cargo.lock
    cargo audit --file site/Cargo.lock

# cargo-fuzz (nightly). CI: 60s on PRs, 600s on the schedule.
# Force the host triple: a musl-built cargo-fuzz otherwise picks
# x86_64-unknown-linux-musl, which AddressSanitizer cannot link.
fuzz target duration="60":
    cargo +nightly fuzz run {{target}} --target "$(rustc +nightly -vV | awk '/^host:/{print $2}')" -- -max_total_time={{duration}}

fuzz-list:
    cargo +nightly fuzz list

ci: fmt-check lint test coverage-check

clean:
    cargo clean
