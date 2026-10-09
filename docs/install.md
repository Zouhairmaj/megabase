---
title: Install
description: Clone the repository and see what is here today. The megabase binary is Phase 0 work.
section: get-started
order: 1
card: Build from source with Rust 1.75+ and PostgreSQL 15+. A Docker image is planned, not shipped.
---

# Install

Megabase is one Rust binary next to PostgreSQL. This repository is still in Phase 0: the Cargo workspace that produces that binary is not in the tree yet. What is here today is the experiment record (GOAL.md, MANIFESTO.md) and the public site generator in `site/`.

> [!NOTE]
> Nothing in this repository is production software. When the binary exists, every endpoint answers HTTP 501 with `{"code":"MEGABASE_NOT_IMPLEMENTED",...}` until a unit passes the judge.

## Clone

```shell
git clone https://github.com/Zouhairmaj/megabase
cd megabase
```

## What you can build today

The only crate in this repository is the website generator:

```shell
cargo run --manifest-path site/Cargo.toml -- --repo-root . --out _site
```

Rust 1.75 or newer is enough for that crate. PostgreSQL is not required to generate the site.

## The megabase binary

Phase 0 adds a Cargo workspace (`megabase`, plus one crate per Supabase service) whose release build is `./target/release/megabase`. That binary is not here yet, so this command is not runnable:

```shell
cargo build --release
```

When it lands, you will also need PostgreSQL 15 or newer. The intended default listen port is 8000 (`MEGABASE_PORT`).

> [!PLANNED]
> A Docker image is planned, not shipped. There is no Dockerfile or compose file in this repository.

## Requirements (once the binary exists)

| Tool | Why |
| --- | --- |
| Rust 1.75+ | Workspace and site generator |
| PostgreSQL 15+ | External database the binary sits next to |
| Docker Compose + Python 3 | Judge only, when `judge/` exists |
