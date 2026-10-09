---
title: Quickstart
description: Build the binary, start it and confirm it answers. No Supabase account needed.
section: get-started
order: 2
card: Start the binary, call /health and get the 501 that proves the server is up.
---

# Quickstart

> [!NOTE]
> Phase 0: nothing passes yet. The `megabase` binary is not in this repository. The commands below are the contract once Phase 0 adds it — every endpoint answers 501 MEGABASE_NOT_IMPLEMENTED. Use these docs to follow along, not to run production.

## Build from source

You need Rust 1.75 or newer and PostgreSQL 15 or newer. Running the judge also needs Docker Compose and Python 3.

```shell
git clone https://github.com/Zouhairmaj/megabase
cd megabase
cargo build --release
```

Today there is no workspace at the repository root, so `cargo build --release` will not produce `megabase`. The site generator is `cargo run --manifest-path site/Cargo.toml -- --repo-root . --out _site`.

## Configuration

| Variable | Description |
| --- | --- |
| DATABASE_URL | PostgreSQL connection string |
| JWT_SECRET | Secret for signing JWTs. Read at startup; auth is not implemented yet |
| MEGABASE_PORT | HTTP port, default 8000 |

None of these are read by a binary in this tree yet. They are the names GOAL.md and the site already use.

## Check it runs

Once `./target/release/megabase` exists:

```shell
export DATABASE_URL=postgres://localhost:5432/megabase
export JWT_SECRET=your-secret
./target/release/megabase
# keep running; leave this terminal open
```

```shell
curl -i http://localhost:8000/health
# today: HTTP 501
# {"code":"MEGABASE_NOT_IMPLEMENTED"}
# a 501 means the server is up; nothing is implemented yet
```

There is no `/health` handler in this repository today. When it exists, a 501 with that code is success for Day 0.

> [!PLANNED]
> A Docker image is planned. Until it ships, build from source as shown above.

## Use with supabase-js

Point supabase-js at your local base URL. ANON_KEY is a JWT signed with your JWT_SECRET. Every request returns 501 today; Level 1 targets REST and Auth email/password first (see Roadmap).

```javascript
import { createClient } from '@supabase/supabase-js'
const supabase = createClient('http://localhost:8000', ANON_KEY)
const { data, error } = await supabase.from('todos').select()
// today: error.code === 'MEGABASE_NOT_IMPLEMENTED'
```

## Run the judge

> [!PLANNED]
> The judge starts the official Supabase reference stack with Docker Compose and compares it with Megabase. Keep Megabase running on :8000 (terminal 1) and run this from the repository root. There is no `judge/` directory and no `docker-compose` file in this repository yet.

```shell
docker compose up -d
python judge/compare.py
```
