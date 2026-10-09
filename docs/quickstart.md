---
title: Quickstart
description: Build the binary, start it and confirm it answers. No hosted account needed.
section: get-started
order: 2
card: Start the binary, call /_megabase/health, then hit a 501 on an unimplemented route.
---

# Quickstart

> [!NOTE]
> Unimplemented HTTP endpoints answer 501 `MEGABASE_NOT_IMPLEMENTED`. Auth
> SQL objects listed in [Configuration](configuration.md) are installed when
> `DATABASE_URL` is set. This is not production software.

## Build from source

You need Rust **1.89** or newer. PostgreSQL 15+ is the intended external
database; the 501 gateway starts without it. The judge also needs Docker.

```shell
git clone --recurse-submodules https://github.com/Zouhairmaj/megabase.git
cd megabase
cargo run --locked -p megabase
```

Or `cargo build --release --locked -p megabase` and run
`./target/release/megabase`. Environment variables are in
[Configuration](configuration.md). Set `JWT_SECRET` to the same HS256
secret that signed your `ANON_KEY` when you start wiring Auth or REST;
the 501 gateway starts without it.

## Check it runs

Keep the binary running, then in another terminal:

```shell
curl -s localhost:8000/_megabase/health
# {"name":"megabase","version":"0.0.0","status":"ok"}
curl -sS -w '\nHTTP %{http_code}\n' localhost:8000/rest/v1/todos
# {"code":"MEGABASE_NOT_IMPLEMENTED",...}
# HTTP 501
```

A 501 on `/rest/v1/todos` means the gateway is up and that unit is not
implemented yet.

## Container image

```shell
docker build -t megabase .
docker run --rm -p 8000:8000 megabase
```

Builder and runtime base images are pinned by digest; see [Install](install.md).

## Use with supabase-js

Point supabase-js at your local base URL. Every request to an unimplemented
unit returns 501 today; Level 1 targets REST and Auth email/password first
(see [Roadmap](ROADMAP.md)).

```javascript
import { createClient } from '@supabase/supabase-js'
const supabase = createClient('http://localhost:8000', ANON_KEY)
const { data, error } = await supabase.from('todos').select()
// today: error.code === 'MEGABASE_NOT_IMPLEMENTED'
```

## Run the judge

The judge starts the official Supabase reference stack with Docker Compose,
creates a dedicated `megabase` database on that cluster, and compares HTTP
plus database side-effects with Megabase. From the repository root:

```shell
just judge-up
just judge
just judge-down
```

Recipes and ports are in `judge/README.md`.
