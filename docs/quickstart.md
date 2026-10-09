---
title: Quickstart
description: Build the binary, start it and confirm it answers. No hosted account needed.
section: get-started
order: 2
card: Start the binary, call /_megabase/health, and get 200. Other routes are 501 until they pass the judge.
---

# Quickstart

> [!NOTE]
> Unimplemented HTTP endpoints answer 501 `MEGABASE_NOT_IMPLEMENTED`. Auth
> SQL objects listed in [Configuration](configuration.md) are installed when
> `DATABASE_URL` is set. This is not production software.

## Build from source

Rust 1.89 or newer. PostgreSQL 15 or newer if you set `DATABASE_URL`. The
judge also needs Docker Compose.

```shell
git clone --recurse-submodules https://github.com/Zouhairmaj/megabase
cd megabase
cargo build --release --locked -p megabase
```

## Configuration

| Variable | Description |
| --- | --- |
| DATABASE_URL | PostgreSQL connection string. Installs implemented Auth SQL objects at startup when set |
| JWT_SECRET | Secret for signing JWTs. Read at startup; Auth HTTP is not implemented yet |
| MEGABASE_PORT | HTTP port, default 8000 |

## Check it runs

```shell
export DATABASE_URL=postgres://postgres@localhost:5432/postgres
export JWT_SECRET=your-secret
./target/release/megabase
# keep running; leave this terminal open
```

```shell
curl -i http://localhost:8000/_megabase/health
# HTTP 200 {"name":"megabase","status":"ok",...}

curl -i http://localhost:8000/auth/v1/health
# HTTP 501 {"code":"MEGABASE_NOT_IMPLEMENTED",...}
```

A 501 on `/auth/v1` means the gateway is up and that HTTP unit is not
ported yet. A 200 on `/_megabase/health` means the process is alive.

```shell
docker build -t megabase .
docker run --rm -p 8000:8000 megabase
```

## Use with supabase-js

Point supabase-js at your local base URL. `ANON_KEY` is a JWT signed with
your `JWT_SECRET`. Every Auth/REST request returns 501 today; Level 1
targets REST and Auth email/password first (see Roadmap).

```javascript
import { createClient } from '@supabase/supabase-js'
const supabase = createClient('http://localhost:8000', ANON_KEY)
const { data, error } = await supabase.from('todos').select()
// today: error.code === 'MEGABASE_NOT_IMPLEMENTED'
```

## Run the judge

Keep Megabase reachable on `:8100` (compose profile) or `:8000` (host),
and the reference stack on `:8000`. From the repository root:

```shell
just judge-up
just judge
just judge-down
```
