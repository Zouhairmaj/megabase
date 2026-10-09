---
title: Configuration
description: MEGABASE_HOST, MEGABASE_PORT, DATABASE_URL, JWT_SECRET — what each variable does.
section: get-started
order: 3
card: Listen address and JWT_SECRET for HS256 verification. Auth and REST routes are still 501.
---

# Configuration

The `megabase` binary reads the process environment at startup. Do not invent
other names. There is no `.env.example` in this tree; the judge uses
`vendor/supabase/docker/.env.example` only.

| Variable | Description |
| --- | --- |
| MEGABASE_HOST | Bind address. Default `0.0.0.0`. |
| MEGABASE_PORT | HTTP port. Default `8000`. |
| DATABASE_URL | PostgreSQL connection string. PostgreSQL stays external; Megabase does not bundle it. When set, startup installs the Auth SQL objects listed in [`specs/auth/database.md`](../specs/auth/database.md). The statements are idempotent. Connect plus SQL must finish within 30 seconds or the process exits. If install fails, the process exits. Omit it to skip schema install (the HTTP server still starts). The judge sets this to the dedicated `megabase` database (`just judge-up` / CI), never to the official stack's `postgres` database. |
| JWT_SECRET | HS256 secret used to verify JWTs (same name as the self-hosted demo stack). Raw UTF-8 bytes, not base64, and not a default. Omit it and the process still starts so `/_megabase/health` works; verification then fails with `Server lacks JWT secret`. A present value shorter than 32 bytes, including empty, aborts startup before listen: `JWT_SECRET is N bytes; HMAC-SHA-256 keys shorter than 32 bytes are disabled`. Length is bytes, not an entropy check. |

## JWT verification

`megabase-core` verifies compact HS256 tokens the way GoTrue and PostgREST
do for the demo secret. Spec: [`specs/core/jwt.md`](../specs/core/jwt.md).

Accepted: tokens signed with this `JWT_SECRET`, including the demo
`ANON_KEY` and `SERVICE_ROLE_KEY` in `vendor/supabase/docker/.env.example`.
Claims exposed to Auth and REST: `role`, `sub` (optional on the demo keys),
`exp` (30-second skew, as PostgREST).

Rejected: forged signatures, expired `exp`, `alg=none` (or any algorithm
other than `HS256`), and HMAC keys shorter than 32 bytes. Asymmetric keys
and JWKS rotation are not implemented.

Auth `/auth/v1` and REST `/rest/v1` still answer 501 until those units are
ported; they will call this verifier instead of copying JWT logic.

## PostgreSQL TLS

The installer connects **without TLS** (`NoTls`). That is the local judge
database. `sslmode=require` aborts startup instead of sending the password
in the clear. `verify-ca` and `verify-full` are not accepted by this client
and also abort. `sslmode=disable`, and the default `prefer` when this client
cannot offer TLS, still send credentials in the clear. Use a Unix socket or
loopback. A remote `DATABASE_URL` needs a separate encrypted transport until
Megabase speaks TLS to PostgreSQL.

## Listen address

Liveness: `GET /_megabase/health` (not a Supabase path).

Same URL layout as the Supabase gateway ([ADR 0002](adr/0002-gateway-layout.md)):

- `/rest/v1`
- `/auth/v1`
- `/storage/v1`
- `/realtime/v1`
- `/functions/v1`
- `/pg` (Postgres Meta)

Until a unit passes the judge, each of those answers HTTP 501 with
`code: MEGABASE_NOT_IMPLEMENTED`.

How to build or run the container image is in [Install](install.md).
