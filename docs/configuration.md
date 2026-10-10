---
title: Configuration
description: Listen address, HTTP limits, DATABASE_URL, JWT_SECRET, and the GoTrue flags Auth honors.
section: get-started
order: 3
card: Listen address, HTTP limits, JWT_SECRET, and the GoTrue env flags Auth honors.
---

# Configuration

The `megabase` binary reads the process environment at startup. Do not invent
other names. There is no `.env.example` in this tree; the judge uses
`vendor/supabase/docker/.env.example` only.

| Variable | Description |
| --- | --- |
| MEGABASE_HOST | Bind address. Default `0.0.0.0`. |
| MEGABASE_PORT | HTTP port. Default `8000`. |
| DATABASE_URL | PostgreSQL connection string. PostgreSQL stays external; Megabase does not bundle it. When set, startup installs the Auth SQL objects listed in [`specs/auth/database.md`](../specs/auth/database.md) (idempotent DDL, 30-second deadline) and opens two sqlx pools (10 connections each, 30-second acquire): one for Auth and one for REST horizontal filters. Signup, logout, verify, the password and refresh-token grants on `POST /auth/v1/token`, the user routes, and served admin routes share the Auth pool. `GET /rest/v1/{relation}` filters in [`specs/rest/filtering.md`](../specs/rest/filtering.md) use the REST pool. SIGINT and SIGTERM close both pools after the drain described under HTTP limits. Omit it to skip schema install (the HTTP server still starts; admin, signup, logout, verify, token, and user calls that need the database then return 500, and a served REST filter returns 503). The judge sets this to the dedicated `megabase` database (`just judge-up` / CI), never to the official stack's `postgres` database. |
| MEGABASE_HTTP_TIMEOUT_MS | Whole-request deadline in milliseconds. Default `150000`, the functions `read_timeout` in `vendor/supabase/docker/volumes/api/kong.yml`. `0` and non-integers abort startup. A request that exceeds it returns 504. |
| MEGABASE_REQUEST_BODY_LIMIT_BYTES | Maximum request body in bytes. Default `52428800`, `FILE_SIZE_LIMIT` in `vendor/supabase/docker/docker-compose.yml`. A non-integer aborts startup. A larger body returns 413. |
| JWT_SECRET | HS256 secret for verifying and signing JWTs (same name as the self-hosted demo stack). Raw UTF-8 bytes, not base64, and not a default. Omit it and the process still starts so `/_megabase/health` and `GET /auth/v1/health` work; verification and signing then fail with `Server lacks JWT secret`. Email signup then returns 500 `Server lacks JWT secret` and does not insert a user. A present value shorter than 32 bytes, including empty, aborts startup before listen: `JWT_SECRET is N bytes; HMAC-SHA-256 keys shorter than 32 bytes are disabled`. Length is bytes, not an entropy check. Required for `/auth/v1/admin` (a missing Bearer token is still 401). |
| GOTRUE_JWT_ADMIN_ROLES | Comma-separated JWT `role` values that may call `/auth/v1/admin`. When unset, `service_role,supabase_admin`, same as GoTrue. An empty value falls back to that pair. Signup settings do not use this allow-list. On `PUT /auth/v1/user`, a role in this list ignores the token audience when the request audience is chosen. |
| GOTRUE_JWT_ADMIN_GROUP_NAME | GoTrue `JWT.AdminGroupName`. Default `admin`. `PUT /auth/v1/user` accepts `app_metadata` only when the user's `role` equals this value and the user's `aud` equals `GOTRUE_JWT_AUD`. This is not the admin-route allow-list. |
| GOTRUE_OAUTH_SERVER_ENABLED | GoTrue flag. Default `false`. Accepted values are `true`, `1`, `false`, and `0`. Any other value falls back to the default and does not abort startup. When false, admin OAuth client routes and `GET`/`DELETE /auth/v1/user/oauth/grants` return 404 `feature_disabled`. |
| GOTRUE_CUSTOM_OAUTH_ENABLED | GoTrue flag. Default `true`. Accepted values are `true`, `1`, `false`, and `0`. Any other value falls back to the default and does not abort startup. When false, admin custom-provider routes return 404 `feature_disabled`, and `GET /auth/v1/user/identities/authorize?provider=custom:…` returns 400 `validation_failed`. |
| GOTRUE_SECURITY_MANUAL_LINKING_ENABLED | GoTrue flag. Default `false`. Accepted values match Go's bool parser (`1`, `t`, `T`, `true`, `0`, `f`, `F`, `false`, any case). Any other value aborts startup. When false, `GET /auth/v1/user/identities/authorize` and `DELETE /auth/v1/user/identities/{identity_id}` return 404 `manual_linking_disabled` after the bearer check. |
| GOTRUE_SECURITY_UPDATE_PASSWORD_REQUIRE_REAUTHENTICATION | GoTrue flag. Default `false`. Accepted values match Go's bool parser (`1`, `t`, `T`, `true`, `0`, `f`, `F`, `false`, any case). Any other value aborts startup. When true, `PUT /auth/v1/user` with `password` returns 501. |
| GOTRUE_SECURITY_UPDATE_PASSWORD_REQUIRE_CURRENT_PASSWORD | GoTrue flag. Default `false`. Accepted values match Go's bool parser (`1`, `t`, `T`, `true`, `0`, `f`, `F`, `false`, any case). Any other value aborts startup. When true, `PUT /auth/v1/user` with `password` returns 501. |
| GOTRUE_URI_ALLOW_LIST | Comma-separated redirect globs for `POST /auth/v1/admin/generate_link` and `GET /auth/v1/verify`. `*` and `?` do not cross `.` or `/`; `**` does. When this is unset or blank, `ADDITIONAL_REDIRECT_URLS` is used instead. A pattern with an unescaped `[` `]` `{` or `}` is not implemented: a redirect that must be checked against it returns 501. |

## JWT verification

`megabase-core` verifies compact HS256 tokens the way GoTrue and PostgREST
do for the demo secret, and signs signup access tokens with the same key.
Spec: [`specs/core/jwt.md`](../specs/core/jwt.md).

Accepted: tokens signed with this `JWT_SECRET`, including the demo
`ANON_KEY` and `SERVICE_ROLE_KEY` in `vendor/supabase/docker/.env.example`.
Claims exposed to Auth and REST: `role`, `sub` (optional on the demo keys),
`exp` (30-second skew, as PostgREST).

Rejected: forged signatures, expired `exp`, `alg=none` (or any algorithm
other than `HS256`), and HMAC keys shorter than 32 bytes. Asymmetric keys
and JWKS rotation are not implemented.

The first `/auth/v1/admin` GET/DELETE batch (issue #6, spec
[`specs/auth/admin.md`](../specs/auth/admin.md)) verifies the Bearer token
with this secret. Logout and the user routes verify it the same way.

## Auth HTTP

Spec: [`specs/auth/endpoints.md`](../specs/auth/endpoints.md).

Served on `/auth/v1`: `GET /health`, `GET /settings`, autoconfirm email
`POST /signup`, `POST /logout`, `POST /token` (password and refresh-token
grants), and `GET`/`POST /verify` for the signup, invite, recovery, and
email-change types. `GET /user` and `PUT /user` are served. Identity
authorize (`GET /user/identities/authorize`) and unlink
(`DELETE /user/identities/{identity_id}`), and the OAuth grant list and
revoke (`GET` and `DELETE /user/oauth/grants`), are served when their flags
in the table above are on. Spec:
[`specs/auth/user.md`](../specs/auth/user.md),
[`specs/auth/token.md`](../specs/auth/token.md), and
[`specs/auth/verify.md`](../specs/auth/verify.md). Invite, recover, resend,
and reauthenticate return 501 with their unit id. Magic link, SMS, phone
change, the `email` OTP type, and PKCE on verify do too. `PUT /user`
returns 501 for an email change, a phone change while SMS autoconfirm is
off, and a password change while reauthentication or the current password
is required. `GET /user/identities/authorize` returns 501 once the provider
is enabled. The admin routes in
[`specs/auth/admin.md`](../specs/auth/admin.md) return GoTrue statuses.
Every other `/auth/v1` path returns 501 `MEGABASE_NOT_IMPLEMENTED`.
`GET /rest/v1/{relation}` serves the horizontal filters in
[`specs/rest/filtering.md`](../specs/rest/filtering.md). Every other REST
route returns 501 `MEGABASE_NOT_IMPLEMENTED`.

Signup, logout, verify, token, and the user routes need `DATABASE_URL` (the
Auth SQL from [`specs/auth/database.md`](../specs/auth/database.md)) and
`JWT_SECRET`. `GET /auth/v1/verify` checks that secret before the token
lookup. A missing secret is rejected before the one-time token is read.
The secret signs a session; the first half of a secure email change does
not issue one.
When `GOTRUE_*` is unset, signup and settings flags match the judge
reference stack, not GoTrue's zero values: email and phone providers on,
anonymous off, `disable_signup` false, mailer autoconfirm on (the judge
overlay has no mail server), phone autoconfirm on, SMS provider empty,
SAML and passkeys off, audience and role `authenticated`, expiry 3600
seconds, issuer `http://localhost:8000/auth/v1`, password minimum 6,
`GOTRUE_SITE_URL` or `SITE_URL` `http://localhost:3000`, mailer OTP expiry 86400 seconds,
secure email change on. Bool
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

## HTTP limits

Every response passes through the gateway layers in `megabase-server`: a
request id (`x-request-id`, echoed when the caller sends one), a layer that
marks `Authorization`, `apikey`, and `Cookie` as sensitive so those values
stay out of traces, a 150-second timeout (504), and a 50 MiB body limit (413)
unless the variables above override them. `Bytes` and `Json` use that same cap. A panic catch
returns JSON `{"code":"internal_error","message":"internal error"}` without
the panic text. SIGINT and SIGTERM stop the listener. In-flight requests may
finish for 10 seconds, then the Auth and REST pools close.

## Checked SQL

Auth reads and writes use `sqlx::query!` and `sqlx::query_as!`. The workspace
compiles them from the committed `.sqlx` cache (`SQLX_OFFLINE=true` in
`.cargo/config.toml`), so CI does not need a database. The container image
copies `.sqlx` and `.cargo` and sets `SQLX_OFFLINE` for the same reason.
Regenerating the cache needs Postgres with the Auth schema installed and a
matching `DATABASE_URL`:

```shell
SQLX_OFFLINE=false DATABASE_URL=postgres://… cargo sqlx prepare --workspace -- --all-targets
```

`cargo sqlx prepare --check` in CI needs a workflow edit. That file is a
protected path, so it belongs on a `review/*` branch.

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
settings, autoconfirm email signup, logout, the password and refresh-token
grants on `POST /auth/v1/token`, `GET`/`POST /auth/v1/verify` for the
signup, invite, recovery, and email-change types, and the user routes are
the exception.

How to build or run the container image is in [Install](install.md).
