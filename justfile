# Megabase commands. Only recipes that actually work are listed.

default:
    @just --list

build:
    cargo build --release --locked -p megabase

run:
    cargo run -p megabase

test:
    cargo test --workspace --locked

lint:
    cargo clippy --workspace --all-targets -- -D warnings

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
    {{compose}} --profile with-megabase up -d --build --wait
    @git -C vendor/supabase status --short || true

judge-down:
    {{compose}} --profile with-megabase down

# Compare both stacks. Requires judge-up, or a host Megabase on :8100 and the reference on :8000.
judge:
    cargo run --locked -p megabase-judge -- wait
    cargo run --locked -p megabase-judge -- run --cases judge/cases --out coverage/judge-results.json --baseline coverage/judge-results.json --summary /tmp/judge-summary.md
    @cat /tmp/judge-summary.md

# Upsert GitHub milestones, labels, Project, epics (needs issues+project write).
backlog:
    cargo run --locked -p megabase-backlog -- sync

ci: fmt-check lint test coverage-check

clean:
    cargo clean
