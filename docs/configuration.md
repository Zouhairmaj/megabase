---
title: Configuration
description: MEGABASE_HOST, MEGABASE_PORT, DATABASE_URL, JWT_SECRET — what each variable does.
section: get-started
order: 3
card: Listen address and secrets the binary reads at startup. Auth does not use JWT_SECRET yet.
---

# Configuration

The `megabase` binary reads the process environment at startup. Do not invent
other names. There is no `.env.example` in this tree; the judge uses
`vendor/supabase/docker/.env.example` only.

| Variable | Description |
| --- | --- |
| MEGABASE_HOST | Bind address. Default `0.0.0.0`. |
| MEGABASE_PORT | HTTP port. Default `8000`. |
| DATABASE_URL | PostgreSQL connection string. PostgreSQL stays external; Megabase does not bundle it. When set, startup installs the Auth SQL objects this build implements: `auth.uid()`, `auth.role()`, `auth.email()`, `auth.jwt()`, and tables `auth.instances`, `auth.audit_log_entries`, `auth.identities`, `auth.flow_state`, `auth.mfa_amr_claims`, `auth.custom_oauth_providers`. The statements are idempotent. If install fails, the process exits. |
| JWT_SECRET | Secret used to sign and verify JWTs. Auth HTTP is not implemented; the value is still read so a missing secret can fail loudly later instead of minting a default. |

Omit `DATABASE_URL` to skip schema install (the HTTP server still starts).

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
