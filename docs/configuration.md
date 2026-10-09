---
title: Configuration
description: DATABASE_URL, JWT_SECRET, MEGABASE_PORT — what each variable does and its default.
section: get-started
order: 3
card: DATABASE_URL, JWT_SECRET, MEGABASE_PORT: what each variable does and its default.
---

# Configuration

Megabase reads the process environment at startup. There is no `.env.example`
in the tree.

| Variable | Description |
| --- | --- |
| DATABASE_URL | PostgreSQL connection string. PostgreSQL stays external; Megabase does not bundle it. When set, startup installs the Auth SQL objects this build implements: `auth.uid()`, `auth.role()`, `auth.email()`, `auth.jwt()`, and tables `auth.instances`, `auth.audit_log_entries`, `auth.identities`, `auth.flow_state`, `auth.mfa_amr_claims`, `auth.custom_oauth_providers`. The statements are idempotent. If install fails, the process exits. |
| JWT_SECRET | Secret used to sign and verify JWTs. Auth HTTP is not implemented; the value is still read so a missing secret can fail loudly later instead of minting a default. |
| MEGABASE_HOST | Bind address. Default `0.0.0.0`. |
| MEGABASE_PORT | HTTP port. Default 8000. |

Omit `DATABASE_URL` to skip schema install (the HTTP server still starts).

## PostgreSQL TLS

The installer connects **without TLS** (`NoTls`). That is the local judge
database. `sslmode=require` aborts startup instead of sending the password
in the clear. `verify-ca` and `verify-full` are not accepted by this client
and also abort. Use a Unix socket, an internal network, or `sslmode=disable`.

## Listen address

Same URL layout as the Supabase gateway, from GOAL.md:

- `/rest/v1`
- `/auth/v1`
- `/storage/v1`
- `/realtime/v1`
- `/functions/v1`
- `/pg` (Postgres Meta)

Until a unit passes the judge, each of those answers HTTP 501 with
`code: MEGABASE_NOT_IMPLEMENTED`. Health is `GET /_megabase/health`.
