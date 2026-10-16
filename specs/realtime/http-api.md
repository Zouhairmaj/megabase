# Realtime: HTTP API (issue 67)

Status: partial (public broadcasts; see Out of scope)
Unit ids (`coverage/units.json`):
`realtime:route:GET /realtime/v1/api/ping`,
`realtime:route:POST /realtime/v1/api/broadcast`,
`realtime:route:POST /realtime/v1/api/broadcast/{topic}/events/{event}`
Level: 3

The tenant HTTP API: a liveness ping and two ways to broadcast to channel
subscribers without a socket. Ping is specified in [`ping.md`](ping.md);
this file covers the two broadcast routes.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Routes and pipelines (`:tenant_api`, `:broadcast_single`, `:secure_tenant_api`) | `vendor/realtime/lib/realtime_web/router.ex:27`–`:45`, `:106`–`:116` |
| Body parsers (`:json`, `OctetStream`, `pass: ["*/*"]`) | `vendor/realtime/lib/realtime_web/endpoint.ex:90`–`:93`, `lib/realtime_web/plugs/parsers/octet_stream.ex` |
| Content-type gate (415) | `vendor/realtime/lib/realtime_web/plugs/validate_broadcast_content_type.ex` |
| Token lookup and 401 | `vendor/realtime/lib/realtime_web/plugs/auth_tenant.ex` |
| `role` / `exp` required | `vendor/realtime/lib/realtime_web/channels/auth/channels_authorization.ex:19` |
| Batch controller and changeset | `vendor/realtime/lib/realtime_web/controllers/broadcast_controller.ex:35`, `lib/realtime/tenants/batch_broadcast.ex` |
| Single controller and validation | `vendor/realtime/lib/realtime_web/controllers/broadcast_single_controller.ex:87`–`:134`, `lib/realtime/tenants/single_broadcast.ex` |
| Error bodies | `vendor/realtime/lib/realtime_web/controllers/fallback_controller.ex`, `lib/realtime_web/views/changeset_view.ex` |
| Tenant defaults (payload 3000 kB, 100 events/s) | `vendor/realtime/lib/realtime/api/tenant.ex:23`, `config/runtime.exs:149`, `lib/realtime/tenants.ex:539` |
| Gateway `key-auth` and `Authorization` rewrite | `vendor/supabase/docker/volumes/api/kong.yml:279`, `volumes/api/kong-entrypoint.sh:24` |

Pins: realtime `v2.143.3`, supabase `v1.26.08` (`vendor.toml`).

## Inputs

- `POST /realtime/v1/api/broadcast` with a JSON body
  `{"messages": [{"topic", "event", "payload", "private"?, "id"?}, …]}`.
- `POST /realtime/v1/api/broadcast/{topic}/events/{event}` with a JSON or
  `application/octet-stream` body (the payload), and the optional
  `private` / `persist` flags (query string or JSON body keys; only `"true"`
  or `true` is true).
- A token in `Authorization: Bearer …` (scheme case-insensitive), else in
  `apikey`. Kong replaces `Bearer sb_…` with the raw `apikey`.

Request params are the query string merged with the parsed body (body wins),
then the path (path wins), as in Phoenix.

## Outputs

202 with an empty body and no content type.

## Errors

In pipeline order:

| When | Status | Body |
|---|---|---|
| JSON body that does not parse (`Plug.Parsers.ParseError`) | 400 | `Bad Request` (`text/html`), or `"Bad Request"` when `Accept` prefers JSON |
| Single: `Content-Type` present and not `application/json` / `application/octet-stream` | 415 | `{"error":"Unsupported Media Type. Use application/json or application/octet-stream"}` |
| No token, bad signature, malformed, expired, or no `role`/`exp` | 401 | `{"message":"Unauthorized"}` |
| Changeset invalid | 422 | `{"errors": …}` (below) |
| Batch longer than 100 messages | 429 | `{"message":"Too many messages to broadcast, please reduce the batch size"}` |

Batch changeset (`cast_embed(:messages, required: true)`):

- `messages` missing or `[]`: `{"errors":{"messages":["can't be blank"]}}`.
- `messages` a string, number or boolean, or an element that is not an
  object: `{"errors":{"messages":["is invalid"]}}`.
- Otherwise one object per message, `{}` for a valid one, present only when
  some message is invalid. Per field: a blank (`null`, whitespace) `topic`,
  `event` or `payload` is `can't be blank`; a non-string `topic`, `event`,
  `id`, a non-object `payload` or a non-boolean `private` is `is invalid`; a
  `payload` whose `:erlang.external_size/1` exceeds 3,000,500 bytes is
  `Payload size exceeds tenant limit`.

Single validation (`SingleBroadcast.validate_message/7`): whitespace-only
`topic` / `event` is `can't be blank`; `persist` without `private` is
`can only be used on private channels`; a payload over 3,000,500 bytes
(`external_size`: binary length + 6, or the size of the decoded JSON body)
is `Payload size exceeds tenant limit`; a JSON body `{"_binary": null}`
takes the binary clause and is `payload: can't be blank`.

## Edge cases

- A JSON body that is not an object is `{"_json": value}`; an empty JSON
  body is `{}`. Both are valid single payloads.
- With no `Content-Type` the body is not read: the single route accepts it
  with payload `{}`, the batch route reports `messages` blank.
- `AssignTenant` resolves the tenant from `Host`; the self-hosted stack has
  one tenant (`realtime-dev`), so Megabase does not model the lookup.

## Out of scope (501)

- Private broadcasts (`private` true on either route): authorization runs the
  tenant's RLS policies on `realtime.messages`.
- `HS384` / `HS512` tokens, as on the socket.
- `application/x-www-form-urlencoded` and `multipart` bodies, nested
  (`a[b]=…`) or badly escaped query strings, bodies over 8,000,000 bytes
  (the 413 reason phrase comes from the unvendored Plug status table), and
  the Ecto index-map / `null` forms of `messages`.
- Without `JWT_SECRET` both routes answer 501.

Not modeled, with the same response as a fresh reference stack:

- The rolling event rate (`RateCounter`, `RateLimiter` plug and its
  `x-rate-*` headers). Megabase treats the rolling average as 0, so only the
  batch-length bound applies.
- `accepts/2` (406 for an `Accept` header that excludes JSON).
- Delivery to socket subscribers. On `main` the socket does not yet relay
  broadcasts; the fan-out to the shared topic hub lands with public
  broadcast on the socket (issue 70).

## Judge cases

To add on `review/issue-67-cases`: 202 for a valid batch and single JSON /
binary broadcast, 401 without a valid token, 415 for `text/plain`, and the
422 bodies above.
