# Server: gateway CORS (issue 317)

Status: complete
Unit ids (`coverage/units.json`): none. The gateway's `cors` plugin is not
a unit in the denominator; it is what lets browsers reach every Auth,
REST, Storage, Realtime, and Functions unit.
Level: 1

A cross-origin browser call to `/auth/v1/*` (and to the other Supabase
prefixes) that needs a CORS preflight (a non-simple method or headers such
as `apikey` or `Authorization`) is preceded by one; simple requests go
straight to the normal response path. In the self-hosted stack Kong answers
that preflight itself with its `cors` plugin and stamps
`Access-Control-Allow-Origin` on every other response. Without this,
sign-up and sign-in from any web app fail before GoTrue sees a request.

## Upstream

Kong's source is not vendored. The behavior below is the `cors` plugin of
the Kong image the pinned `docker-compose.yml` runs, with the plugin's
default configuration, because `kong.yml` attaches it without a `config`
block.

| Behavior | File:line (pin) |
|---|---|
| `cors` on every Auth service, including the `/auth/v1/` catch-all, with no `config` | [`vendor/supabase/docker/volumes/api/kong.yml:111`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/volumes/api/kong.yml#L111) (also `:49`, `:59`, `:69`, `:79`, `:89`, `:99`) |
| `cors` on REST (`:139`, `:167`), GraphQL (`:196`), Realtime websocket (`:226`) and API (`:282`), Storage (`:323`), Functions (`:348`), OAuth metadata (`:360`), Studio catch-all (`:436`) | [`vendor/supabase/docker/volumes/api/kong.yml:139`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/volumes/api/kong.yml#L139) |
| No `cors` on Postgres Meta `/pg/` | [`vendor/supabase/docker/volumes/api/kong.yml:386`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/volumes/api/kong.yml#L386) |
| No `cors` on the blocked `/realtime/v1/api/openapi`, `/realtime/v1/api/tenants`, `/api/mcp`, and `/mcp` routes (`request-termination`) | [`vendor/supabase/docker/volumes/api/kong.yml:244`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/volumes/api/kong.yml#L244), `:259`, `:404`, `:418` |
| Kong image `kong/kong:3.9.3`; `cors` enabled in `KONG_PLUGINS` | [`vendor/supabase/docker/docker-compose.yml:70`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/docker-compose.yml#L70), `:100` |

Pins: supabase `v1.26.08` (`vendor.toml`). The judge runs the same Kong
3.9.3 (`judge/compose.override.yml`). Kong 3.9.3 `cors` plugin defaults:
`origins`, `headers`, `exposed_headers`, `max_age` unset; `methods` =
`GET, HEAD, PUT, PATCH, POST, DELETE, OPTIONS, TRACE, CONNECT`;
`credentials = false`; `preflight_continue = false`;
`private_network = false`; `allow_origin_absent = true`. The plugin's
`access` phase has priority 2000, above `key-auth` (1250), `acl` (950),
`basic-auth` (1100), and `request-transformer` (801), so a preflight is
answered before authentication and never reaches the upstream service.
Nothing in `kong.yml` or `.env.example` overrides these, so Megabase has
no CORS configuration either.

Implementation: `crates/megabase-server/src/cors.rs`, applied in
`layers.rs` outside the body limit and the deadline and inside the request
id. No code is copied from Kong, so there is no credit header and no
`NOTICE` entry.

## Inputs

- Any request whose path is not under one of the prefixes without the
  plugin: `/pg/`, `/realtime/v1/api/openapi`, `/realtime/v1/api/tenants`,
  `/api/mcp`, `/mcp`. Kong picks the longest matching route path, so these
  are plain prefix tests, like the gateway table in
  [ADR 0002](../../docs/adr/0002-gateway-layout.md).
- A **preflight** is `OPTIONS` with both `Origin` and
  `Access-Control-Request-Method` present (any value, including empty).
  `Access-Control-Request-Headers` is optional.
- Every other request is a **simple request** for this spec, including
  `OPTIONS` without one of those two headers, which goes to the component
  (REST answers its `Allow`; unported routes answer 501).

## Outputs

Preflight (Kong `access` phase, `kong.response.exit(200)`):

| Header | Value |
|---|---|
| status | `200`, empty body, no `Content-Type` |
| `Access-Control-Allow-Origin` | `*` (no `origins` configured; the request `Origin` is not echoed) |
| `Access-Control-Allow-Headers` | the first `Access-Control-Request-Headers` value, verbatim; absent when the request sent none |
| `Access-Control-Allow-Methods` | `GET,HEAD,PUT,PATCH,POST,DELETE,OPTIONS,TRACE,CONNECT` |
| `Access-Control-Allow-Credentials`, `Access-Control-Max-Age`, `Access-Control-Expose-Headers`, `Vary`, `Allow` | not set (`credentials = false`, no `max_age`, no `exposed_headers`, upstream not called) |

No `apikey` is needed: `key-auth` runs after `cors` has already exited.

Simple request (Kong `header_filter` phase): the upstream response passes
through with `Access-Control-Allow-Origin: *` set, replacing any value the
service set. Kong does this whether or not the request carried `Origin`,
and on its own error pages (timeout, oversized body, 501 from Megabase).
No other CORS header is added by the gateway.

## Errors

None of its own. A preflight cannot fail at the gateway; the component's
errors are unchanged apart from the added header.

## Edge cases

- `OPTIONS` with `Origin` but no `Access-Control-Request-Method` (or the
  reverse) is not a preflight and reaches the component.
- Repeated `Access-Control-Request-Headers` lines: only the first is
  echoed, as `kong.request.get_header` returns the first value.
- A preflight to a path without the plugin (`/pg/tables`) gets the
  component's answer and no CORS header. On the reference stack that is
  `key-auth`'s 401 or `request-termination`'s 403; Megabase answers 501
  until those routes are ported.
- `OPTIONS /rest/v1/{relation}` without `Origin` is PostgREST's own
  `OPTIONS` (`specs/rest/resources.md`), with `Allow`, plus the stamped
  `Access-Control-Allow-Origin`.
- Megabase's `/_megabase/health` falls under Kong's Studio catch-all, so
  it gets the same treatment.

## Out of scope

- GoTrue's own `rs/cors` middleware
  (`vendor/auth/internal/api/api.go:474`): with a request `Origin` it adds
  `Access-Control-Allow-Credentials: true`,
  `Access-Control-Expose-Headers: X-Total-Count, Link, X-Supabase-Api-Version`
  and `Vary: Origin` to simple responses. Those belong to the Auth crate,
  not the gateway, and are not served yet.
- PostgREST's and Storage's own CORS headers on simple responses, for the
  same reason.
- Any `cors` configuration (`origins`, `credentials`, `max_age`): the
  pinned stack has none, so Megabase exposes no knob.

## Judge cases

Judge cases live in `judge/`, which this feature branch cannot edit
([ADR 0003](../../docs/adr/0003-protected-paths.md)). A
`review/issue-317-cases` pull request should add, with
`compare_headers = ["access-control-allow-origin", "access-control-allow-methods", "access-control-allow-headers"]`:

- `auth.cors.preflight`: `OPTIONS /auth/v1/signup` with `Origin`,
  `Access-Control-Request-Method: POST` and the supabase-js
  `Access-Control-Request-Headers`, expecting `200` and the headers above.
- `auth.cors.simple`: `GET /auth/v1/health` with `Origin`, expecting
  `Access-Control-Allow-Origin: *`.

Until then the behavior is covered by `crates/megabase-server/tests/cors.rs`
and the unit tests in `cors.rs` and `layers.rs`.
