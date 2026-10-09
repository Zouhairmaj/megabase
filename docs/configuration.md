---
title: Configuration
description: MEGABASE_HOST, MEGABASE_PORT, DATABASE_URL, JWT_SECRET — what each variable does.
section: get-started
order: 3
card: Listen address, JWT_SECRET, and the GoTrue env flags Auth admin and signup honor.
---

# Configuration

The `megabase` binary reads the process environment at startup. Do not invent
other names. There is no `.env.example` in this tree; the judge uses
`vendor/supabase/docker/.env.example` only.

| Variable | Description |
| --- | --- |
| MEGABASE_HOST | Bind address. Default `0.0.0.0`. |
| MEGABASE_PORT | HTTP port. Default `8000`. |
| DATABASE_URL | PostgreSQL connection string. PostgreSQL stays external; Megabase does not bundle it. When set, startup installs the Auth SQL objects listed in [`specs/auth/database.md`](../specs/auth/database.md). The statements are idempotent. Connect plus SQL must finish within 30 seconds or the process exits. If install fails, the process exits. Served admin routes that read or write Auth tables use this URL per request and abort the connect after 30 seconds with 500. Signup and logout use one connection opened at startup. Omit it to skip schema install (the HTTP server still starts; admin, signup, and logout calls that need the database then return 500). The judge sets this to the dedicated `megabase` database (`just judge-up` / CI), never to the official stack's `postgres` database. |
| JWT_SECRET | HS256 secret for verifying and signing JWTs (same name as the self-hosted demo stack). Raw UTF-8, not base64. There is no default: a missing or empty value is an error when verifying or signing, so Megabase never mints a key. The process still starts without it so `/_megabase/health` and `GET /auth/v1/health` work. Required for `/auth/v1/admin` (a missing Bearer token is still 401). Email signup then returns 500 `Server lacks JWT secret` and does not insert a user. |
| GOTRUE_JWT_ADMIN_ROLES | Comma-separated JWT `role` values that may call `/auth/v1/admin`. When unset, `service_role,supabase_admin`, same as GoTrue. An empty value falls back to that pair. Signup settings do not use this allow-list. |
| GOTRUE_OAUTH_SERVER_ENABLED | GoTrue flag. Default `false`. When false, admin OAuth client routes return 404 `feature_disabled`. |
| GOTRUE_CUSTOM_OAUTH_ENABLED | GoTrue flag. Default `true`. When false, admin custom-provider routes return 404 `feature_disabled`. |

## JWT verification

`megabase-core` verifies compact HS256 tokens the way GoTrue and PostgREST
do for the demo secret, and signs signup access tokens with the same key.
Spec: [`specs/core/jwt.md`](../specs/core/jwt.md).

Accepted: tokens signed with this `JWT_SECRET`, including the demo
`ANON_KEY` and `SERVICE_ROLE_KEY` in `vendor/supabase/docker/.env.example`.
Claims exposed to Auth and REST: `role`, `sub` (optional on the demo keys),
`exp` (30-second skew, as PostgREST).

Rejected: forged signatures, expired `exp`, and `alg=none` (or any
algorithm other than `HS256`). Asymmetric keys and JWKS rotation are not
implemented.

The first `/auth/v1/admin` GET/DELETE batch (issue #6, spec
[`specs/auth/admin.md`](../specs/auth/admin.md)) verifies the Bearer token
with this secret. Logout verifies it the same way.

## Auth HTTP

Spec: [`specs/auth/endpoints.md`](../specs/auth/endpoints.md).

Served on `/auth/v1`: `GET /health`, `GET /settings`, autoconfirm email
`POST /signup`, and `POST /logout`. Invite, recover, resend, and
reauthenticate return 501 with their unit id. The admin routes in
[`specs/auth/admin.md`](../specs/auth/admin.md) return GoTrue statuses.
Every other `/auth/v1` path, and all of REST, returns 501
`MEGABASE_NOT_IMPLEMENTED`.

Signup and logout need `DATABASE_URL` (the Auth SQL from
[`specs/auth/database.md`](../specs/auth/database.md)) and `JWT_SECRET`.
When `GOTRUE_*` is unset, signup and settings flags match the judge
reference stack, not GoTrue's zero values: email and phone providers on,
anonymous off, `disable_signup` false, mailer autoconfirm on (the judge
overlay has no mail server), phone autoconfirm on, SMS provider empty,
SAML and passkeys off, audience and role `authenticated`, expiry 3600
seconds, issuer `http://localhost:8000/auth/v1`, password minimum 6. Bool
overrides accept Go's `1`/`t`/`true`/`0`/`f`/`false`.
`GOTRUE_PASSWORD_MIN_LENGTH` below 6 is raised to 6. External OAuth
provider flags (`GOTRUE_EXTERNAL_GITHUB_ENABLED` and the rest of the
settings object) default off. The admin allow-list stays the GoTrue unset
default in the table above.

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

Routes that are not served yet answer HTTP 501 with
`code: MEGABASE_NOT_IMPLEMENTED`. Served Auth admin routes and Auth health,
settings, autoconfirm email signup, and logout are the exception.

How to build or run the container image is in [Install](install.md).
