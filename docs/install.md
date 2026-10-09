---
title: Install
description: Build the megabase binary with Rust 1.89+ and point it at PostgreSQL 15+.
section: get-started
order: 1
card: Build from source with Rust 1.89+ and PostgreSQL 15+. A Docker image is in the repository.
---

# Install

Megabase is one Rust binary next to PostgreSQL. Clone the repository, build
`megabase`, and set `DATABASE_URL` if you want it to install Auth SQL objects
at startup.

> [!NOTE]
> Nothing in this repository is production software. Unimplemented HTTP
> endpoints answer 501 with `{"code":"MEGABASE_NOT_IMPLEMENTED",...}`.

## Clone

```shell
git clone --recurse-submodules https://github.com/Zouhairmaj/megabase
cd megabase
```

`vendor/` is git submodules. Coverage extraction and the judge need them;
building the binary does not.

## Build the binary

Rust **1.89** or newer (the MSRV). From the repository root:

```shell
cargo build --release --locked -p megabase
./target/release/megabase
```

Default listen address is `0.0.0.0:8000` (`MEGABASE_HOST` / `MEGABASE_PORT`).
Liveness is `GET /_megabase/health`.

The website generator is separate: `cargo run --manifest-path site/Cargo.toml -- --repo-root . --out _site`.

## PostgreSQL

PostgreSQL 15 or newer stays external. When `DATABASE_URL` is set, startup
creates the Auth schema objects Megabase currently implements (see
[Configuration](configuration.md)). The install is idempotent. Without
`DATABASE_URL` the process still serves HTTP.

```shell
docker build -t megabase .
docker run --rm -p 8000:8000 -e DATABASE_URL=postgres://postgres@host:5432/postgres megabase
```

## Requirements

| Tool | Why |
| --- | --- |
| Rust 1.89+ | Workspace MSRV |
| PostgreSQL 15+ | External database the binary sits next to |
| Docker Compose | Judge only (`just judge-up`) |
