# Server: gateway key-auth (issue 349)

Status: complete
Unit ids (`coverage/units.json`): none. Kong's `key-auth` plugin is not a
unit in the denominator; it guards the Auth, REST, GraphQL, Realtime and
Postgres Meta units.
Level: 1

In the self-hosted stack every request on a `key-auth` route must carry an
API key. Without one, Kong answers 401 before the upstream service runs, so
`GET /auth/v1/health` without `apikey` is a gateway error, not GoTrue's
health document.

## Upstream

Kong's source is not vendored. The behavior is the `key-auth` plugin of the
Kong 3.9.3 image (`vendor/supabase/docker/docker-compose.yml`) as
`kong.yml` configures it (`hide_credentials: false`, all other options at
their defaults).

| Behavior | File:line (pin) |
|---|---|
| Open Auth routes without `key-auth`: `/auth/v1/verify`, `/callback`, `/authorize`, `/.well-known/jwks.json`, `/sso/saml/acs`, `/sso/saml/metadata` | `vendor/supabase/docker/volumes/api/kong.yml:40`–`:99` |
| `key-auth` on `/auth/v1/` | `vendor/supabase/docker/volumes/api/kong.yml:112` |
| `key-auth` on the REST root and `/rest/v1/` | `vendor/supabase/docker/volumes/api/kong.yml:140`, `:168` |
| `key-auth` on `/graphql/v1` | `vendor/supabase/docker/volumes/api/kong.yml:197` |
| `key-auth` on `/realtime/v1/` and `/realtime/v1/api` | `vendor/supabase/docker/volumes/api/kong.yml:228`, `:289` |
| `request-termination` (no `key-auth`) on `/realtime/v1/api/openapi` and `/realtime/v1/api/tenants` | `vendor/supabase/docker/volumes/api/kong.yml:257`, `:273` |
| No `key-auth` on Storage (S3 requests carry no `apikey`) | `vendor/supabase/docker/volumes/api/kong.yml:308` |
| `key-auth` and `acl` (`admin` only) on `/pg/` | `vendor/supabase/docker/volumes/api/kong.yml:394`–`:410` |
| Consumers `anon` and `service_role` with `$SUPABASE_*_KEY` credentials | `vendor/supabase/docker/volumes/api/kong.yml:8`–`:16` |

Pins: supabase `v1.26.08` (`vendor.toml`). Kong 3.9.3 `key-auth` defaults:
`key_names = ["apikey"]`, `key_in_header = true`, `key_in_query = true`,
`key_in_body = false`. Plugin priority 1250 is below `cors` (2000), so a
CORS preflight is answered first (`specs/server/cors.md`).

## Inputs

The `apikey` request header, else the `apikey` query parameter. An empty
value counts as missing.

## Outputs

Requests with a key continue to the component unchanged.

## Errors

| When | Status | Body |
|---|---|---|
| `key-auth` route, no key | 401, `WWW-Authenticate: Key realm="kong"`, `Content-Type: application/json; charset=utf-8` | `{"message":"No API key found in request","request_id":"<32 hex>"}` |
| `/pg/` with a key that is not the service-role key (`acl`) | 403 | `{"message":"You cannot consume this service","request_id":"<32 hex>"}` |

## Edge cases

- Kong picks the longest matching route path, so the open Auth routes and
  the terminated Realtime admin routes win over `/auth/v1/` and
  `/realtime/v1/`. Matching is a plain prefix test.
- `/auth/v1` and `/rest/v1` without the trailing slash belong to Studio
  (`docs/adr/0002-gateway-layout.md`) and have no `key-auth`.

## Out of scope

- Validating the key against the consumer credentials. Megabase is not
  given `SUPABASE_ANON_KEY` / `SUPABASE_SERVICE_KEY`, so a present but
  unknown key still reaches the component (Kong would answer 401
  `Unauthorized`). `/pg/` recognizes the service-role key by verifying it as
  an HS256 JWT with `role: service_role` against `JWT_SECRET`.
- The `request-transformer` `Authorization` rewrite for opaque keys.

## Judge cases

`judge/cases/auth.toml`: `auth.health.no-key`, `auth.settings.no-key`.
