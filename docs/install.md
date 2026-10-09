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

The public site generator is separate:

```shell
cargo run --manifest-path site/Cargo.toml -- --repo-root . --out _site
```

## Container image

The root `Dockerfile` builds the release image. The Rust builder
(`rust:1.89-slim-bookworm`) and Debian runtime (`debian:bookworm-slim`)
are pinned by digest (OpenSSF Scorecard Pinned-Dependencies). Renovate
updates tag and digest together (`docker` datasource, `pinDigests`).

```shell
docker build -t megabase .
docker run --rm -p 8000:8000 megabase
```

The image includes `megabase-healthcheck`, which probes
`/_megabase/health`.

## Requirements

| Tool | Why |
| --- | --- |
| Rust 1.89+ | Workspace MSRV; CI also checks 1.89 |
| PostgreSQL 15+ | External database (not required just to start the 501 gateway) |
| Docker | Release image; Compose for the judge |
