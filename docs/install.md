---
title: Install
description: Clone the workspace, build the megabase binary, or build the pinned container image.
section: get-started
order: 1
card: Build from source with Rust 1.89+ or from the root Dockerfile. PostgreSQL is external.
---

# Install

Megabase is one Rust binary next to PostgreSQL. Unimplemented routes return
HTTP 501 `{"code":"MEGABASE_NOT_IMPLEMENTED",...}`. Nothing here is
production software.

## Clone

```shell
git clone --recurse-submodules https://github.com/Zouhairmaj/megabase.git
cd megabase
```

Vendor pins live in `vendor/` (`vendor.toml`). Coverage and the judge need the submodules.

## Build from source

MSRV is **1.89**. From the repository root:

```shell
cargo build --release --locked -p megabase
./target/release/megabase
```

The binary listens on `0.0.0.0:8000` (`MEGABASE_HOST` / `MEGABASE_PORT`).
See [Configuration](configuration.md). `just` lists every recipe that
works today.

When `DATABASE_URL` is set, startup creates the Auth schema objects Megabase
currently implements. The install is idempotent. Without `DATABASE_URL` the
process still serves HTTP.

The public site generator is separate:

```shell
cargo run --manifest-path site/Cargo.toml -- --repo-root . --out _site
```

## Container image

The root `Dockerfile` builds the release image. Both `FROM` lines are
pinned by digest so Scorecard Pinned-Dependencies does not flag a
floating tag (alerts #6 and #7 on `main`):

| Stage | Image |
| --- | --- |
| Builder | `rust:1.89-slim-bookworm@sha256:d7fc7de78bb8c1469933aeecbf801314d30d7d6e9f0578bba4cfa285bfa37fe6` |
| Runtime | `debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587` |

The Cloud Agent image in `.cursor/Dockerfile` is pinned the same way:
`ubuntu:24.04@sha256:534baea6a22c03a63003dbc8dbe78fe34bc0d7e595d9a9dc9834884ff530eb55`
(index digest for that tag, verified 2026-10-09).

Renovate's `docker` datasource has `pinDigests: true`, so a tag bump and
its digest move in the same PR. Do not un-pin these images to a bare tag.

```shell
docker build -t megabase .
docker run --rm -p 8000:8000 megabase
```

Pass `-e DATABASE_URL=...` if the process should install Auth SQL objects.
The image includes `megabase-healthcheck`, which probes
`/_megabase/health`.

## Requirements

| Tool | Why |
| --- | --- |
| Rust 1.89+ | Workspace MSRV; CI also checks 1.89 |
| PostgreSQL 15+ | External database (not required just to start the 501 gateway) |
| Docker | Release image; Compose for the judge |
