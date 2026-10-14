# REST: CORS (issue 318)

Status: complete
Unit ids (`coverage/units.json`): `rest:route:OPTIONS /rest/v1/`, `rest:route:OPTIONS /rest/v1/{relation}`, `rest:route:OPTIONS /rest/v1/rpc/{function}`; the response headers below apply to every `rest:route:*` unit
Level: 1

Browsers send a preflight (`OPTIONS` with `Origin` and
`Access-Control-Request-Method`) before a `PATCH`, a `DELETE`, or any request
with `apikey`, `Authorization`, `Prefer`, or another non-simple header, and
only read response headers named in `Access-Control-Expose-Headers`
(`supabase-js` reads `Content-Range` for counts). In the pinned stack two
layers produce these headers: Kong's `cors` plugin at the gateway and
PostgREST's `wai-cors` middleware behind it. Megabase keeps the same split:
`megabase-server` serves the Kong part for every route that lists the plugin
(Auth, REST, GraphQL, Realtime, Storage, Functions, OAuth discovery) and
`megabase-rest` serves the PostgREST part.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| `cors` plugin with no `config` on `rest-v1-openapi`, `rest-v1`, `graphql-v1` | [`vendor/supabase/docker/volumes/api/kong.yml:139`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/volumes/api/kong.yml#L139), [`:167`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/volumes/api/kong.yml#L167), [`:196`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/volumes/api/kong.yml#L196) |
| Same plugin on the Auth, Realtime, Storage, Functions, and OAuth discovery services; absent on `meta` (`/pg/`) and on the 403 `request-termination` routes `/realtime/v1/api/openapi`, `/realtime/v1/api/tenants`, `/api/mcp`, `/mcp`. The `dashboard` catch-all lists it too | [`kong.yml:49`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/volumes/api/kong.yml#L49)–[`:366`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/volumes/api/kong.yml#L366), [`:394`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/volumes/api/kong.yml#L394), [`:455`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/volumes/api/kong.yml#L455) |
| Gateway image `kong/kong:3.9.3`; the plugin defaults are that version's `kong/plugins/cors/schema.lua` and `handler.lua` (Kong is not vendored) | [`vendor/supabase/docker/docker-compose.yml:70`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/docker-compose.yml#L70) |
| PostgREST policy: only when `Origin` is present; all origins; exposed headers; `corsVaryOrigin = False` | [`vendor/postgrest/src/library/PostgREST/Cors.hs:25`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Cors.hs#L25) |
| Expected header values | [`vendor/postgrest/test/spec/Feature/CorsSpec.hs:14`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/test/spec/Feature/CorsSpec.hs#L14) |

Pins: `supabase v1.26.08`, `postgrest v16.4` (`vendor.toml`). The PostgREST
layer carries the credit header required by AGENTS.md. The Kong layer is
written from the plugin's documented default configuration, not ported.

## Inputs

- `Origin` request header.
- `Access-Control-Request-Method` and `Access-Control-Request-Headers` on an
  `OPTIONS` request.
- Any method and path under a gateway prefix whose Kong service lists the
  `cors` plugin.

## Outputs

### Preflight (gateway, Kong `cors` plugin)

An `OPTIONS` request with both `Origin` and `Access-Control-Request-Method`
is answered in Kong's access phase. The plugin runs before `key-auth`,
`acl`, and `request-transformer`, so no `apikey` is needed and the admin-only
root `/rest/v1/` still answers 200. The upstream is never called, so there
is no `Allow` and no `Access-Control-Expose-Headers`.

| Header | Value |
|---|---|
| status | 200, empty body |
| `Access-Control-Allow-Origin` | `*` (`origins` unset) |
| `Access-Control-Allow-Methods` | `GET,HEAD,PUT,PATCH,POST,DELETE,OPTIONS,TRACE,CONNECT` (the `methods` default, joined with `,`) |
| `Access-Control-Allow-Headers` | the request's `Access-Control-Request-Headers` verbatim (`headers` unset); absent when the request has none |
| `Access-Control-Allow-Credentials` | absent (`credentials` default `false`) |
| `Access-Control-Max-Age` | absent (`max_age` unset) |

### Every other response (gateway)

Kong's header filter sets `Access-Control-Allow-Origin: *` on every response
of a route with the plugin, with or without an `Origin` header. It is a set,
not an append, so PostgREST's own `*` is replaced and the header appears
once. `exposed_headers` is unset, so the gateway adds no
`Access-Control-Expose-Headers`.

### Request with `Origin` (PostgREST)

| Header | Value |
|---|---|
| `Access-Control-Allow-Origin` | `*` (`PGRST_SERVER_CORS_ALLOWED_ORIGINS` unset) |
| `Access-Control-Expose-Headers` | `Content-Encoding, Content-Location, Content-Range, Content-Type, Date, Location, Server, Transfer-Encoding, Range-Unit` |
| `Vary` | not added |

Applies to every status, including errors, and to `HEAD` and `OPTIONS`. A
request without `Origin` gets nothing from PostgREST.

## Errors

None. The layers add headers to whatever the route returns.

## Edge cases

- `OPTIONS` with `Origin` but no `Access-Control-Request-Method` is not a
  preflight: Kong forwards it, PostgREST answers `Allow` and
  `Access-Control-Allow-Origin: *`.
- `OPTIONS` with `Access-Control-Request-Method` but no `Origin` is forwarded
  too.
- A non-`OPTIONS` request with both preflight headers is a normal request.
- `/rest/v1` without the trailing slash and other unmatched paths fall to
  Kong's `dashboard` catch-all, which lists the plugin, so they get the same
  preflight and `Access-Control-Allow-Origin: *` as any CORS route.
- `/pg/…`, the blocked routes above, and `/_megabase/health` are outside the
  plugin and carry no CORS header.
- `Access-Control-Allow-Headers` echoes the request value byte for byte,
  including case and spacing.

## Out of scope

- PostgREST's own preflight branch (`Access-Control-Allow-Methods: GET, POST,
  PATCH, PUT, DELETE, OPTIONS, HEAD`, `Access-Control-Max-Age: 86400`) is
  never reached behind Kong and is not served.
- `PGRST_SERVER_CORS_ALLOWED_ORIGINS` (origin allow-list with
  `Access-Control-Allow-Credentials`) is unset in the pinned stack.
- GoTrue's own CORS headers on `/auth/v1/` (`rs/cors`: exposed `X-Total-Count`,
  `Link`) belong to the Auth CORS issue; the gateway part above already
  covers Auth preflights.

## Judge cases

`rest.root.options-cors-preflight` and `rest.options.relation-cors-preflight`
(`judge/cases/rest.toml`) compare `allow` and `access-control-allow-origin`.
Cases that compare `access-control-allow-methods`,
`access-control-allow-headers` on a preflight with
`Access-Control-Request-Headers`, and `access-control-expose-headers` on a
`GET` with `Origin` go in a separate `review/issue-318-cases` PR.
