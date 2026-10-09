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
| DATABASE_URL | PostgreSQL connection string. PostgreSQL stays external; Megabase does not bundle it. Read today, unused until a unit needs it. |
| JWT_SECRET | HS256 secret used to verify JWTs (same name as the self-hosted demo stack). Raw UTF-8, not base64. There is no default: a missing or empty value is an error when verifying, so Megabase never mints a key. The process still starts without it so `/_megabase/health` works. |

## JWT verification

`megabase-core` verifies compact HS256 tokens the way GoTrue and PostgREST
do for the demo secret. Spec: [`specs/core/jwt.md`](../specs/core/jwt.md).

Accepted: tokens signed with this `JWT_SECRET`, including the demo
`ANON_KEY` and `SERVICE_ROLE_KEY` in `vendor/supabase/docker/.env.example`.
Claims exposed to Auth and REST: `role`, `sub` (optional on the demo keys),
`exp` (30-second skew, as PostgREST).

Rejected: forged signatures, expired `exp`, and `alg=none` (or any
algorithm other than `HS256`). Asymmetric keys and JWKS rotation are not
implemented.

Auth `/auth/v1` and REST `/rest/v1` still answer 501 until those units are
ported; they will call this verifier instead of copying JWT logic.

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
