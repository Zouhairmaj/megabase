# Realtime: WebSocket endpoint (issue 70)

Status: partial (first slice: connect, heartbeat, public join, leave)
Unit ids (`coverage/units.json`): `realtime:route:GET /realtime/v1/websocket`
Level: 3

The Phoenix socket supabase-js (`@supabase/realtime-js`) opens for
channels. Kong strips `/realtime/v1/` and forwards to Realtime's
`/socket/websocket`.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| `socket "/socket", RealtimeWeb.UserSocket`, V1 and V2 serializers, `max_frame_size: 5_000_000` | `vendor/realtime/lib/realtime_web/endpoint.ex:15`–`:40` |
| `connect/3`: token from `x-api-key` header, else `apikey` param; tenant JWT check | `vendor/realtime/lib/realtime_web/channels/user_socket.ex:52`–`:125` |
| `handle_error/2`: connect errors become JSON HTTP answers | `vendor/realtime/lib/realtime_web/channels/user_socket.ex:132`–`:162` |
| `authorize_conn/3`: `role` and `exp` required; expired-token message | `vendor/realtime/lib/realtime_web/channels/auth/channels_authorization.ex:19`–`:38` |
| JWT shape, signer choice, `exp`/`iat` validators | `vendor/realtime/lib/realtime_web/channels/auth/jwt_verification.ex` |
| `join("realtime:" <> sub_topic, …)` and its error reasons | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:48`–`:288` |
| `assign_access_token/2`, `confirm_token/1` | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:803`–`:857` |
| `log_error/3`, `build_msg/2` (`"<Code>: <message>"` reasons) | `vendor/realtime/lib/realtime_web/channels/realtime_channel/logging.ex:21`, `:64` |
| Malformed frames are logged and ignored | `vendor/realtime/lib/realtime_web/socket.ex:48`–`:67` |
| V2 text frames `[join_ref, ref, topic, event, payload]` | `vendor/realtime/lib/realtime_web/socket/v2_serializer.ex` |
| Kong route: `key-auth`, `x-api-key` and `apikey` from `query_params.apikey` | `vendor/supabase/docker/volumes/api/kong.yml:217`–`:244` |
| Client constants (`vsn` 2.0.0 default, `phx_*` events) | `vendor/supabase-js/packages/core/realtime-js/src/lib/constants.ts:14`–`:57` |

Heartbeat, unmatched-topic, duplicate-join and leave/close handling come
from Phoenix 1.7 `Phoenix.Socket` (`__in__`, `encode_ignore`,
`encode_close`), which Realtime uses unchanged and which is not vendored.

Pins: realtime `v2.143.3`, supabase `v1.26.08`, supabase-js `v2.117.3`.

## Inputs

`GET /realtime/v1/websocket?apikey=<jwt>&vsn=<1.0.0|2.0.0>` with a
WebSocket upgrade. `x-api-key` overrides the `apikey` parameter. The
token is percent-decoded once more and stripped of whitespace
(`clean_token`).

## Outputs

101 Switching Protocols, then text frames. V2 encodes
`[join_ref, ref, topic, event, payload]`; V1 encodes
`{"topic","event","payload","ref"}`.

| Client frame | Server frames |
|---|---|
| `phoenix` / `heartbeat` | `phx_reply` `{"status":"ok","response":{}}`, `join_ref` null |
| `realtime:<t>` / `phx_join` (public, no presence, no postgres changes) | `phx_reply` ok `{"postgres_changes":[]}` |
| `phx_join` on an already joined topic | `phx_close` for the old join, then the new join's reply |
| joined topic / `phx_leave` | `phx_reply` ok `{}`, then `phx_close` `{}` with `ref` = `join_ref` |
| non-`realtime:` topic, or a non-join event on an unjoined topic | `phx_reply` error `{"reason":"unmatched topic"}` |
| joined topic, `join_ref` differs from the current join | nothing |
| joined topic, unknown event | nothing |
| invalid JSON / wrong frame shape | nothing; the socket stays open |

## Errors

Handshake (before the upgrade):

| When | Status | Body |
|---|---|---|
| No `apikey` at all | 401 | Kong `No API key found in request` (gateway) |
| `vsn` not `~> 1.0.0` / `~> 2.0.0` | 403 | empty |
| `apikey` only as a header (Kong forwards no token) | 401 | `{"error":"API key is missing"}` |
| Not a JWT | 401 | `{"error":"The token provided is not a valid JWT"}` |
| Bad signature | 401 | `{"error":"Failed to validate JWT signature"}` |
| `alg` not HS* | 401 | `{"error":"Failed to generate JWT signer, check your JWT secret or JWKS configuration"}` |
| `exp` integer and not in the future | 401 | `{"error":"Token has expired"}` |
| No `role` or no `exp` | 401 | ``{"error":"Fields `role` and `exp` are required in JWT"}`` |
| Claims not an object, header not an object, non-numeric `exp`/`iat`, bad percent escape | 500 | `{"error":"Error connecting to Realtime"}` |

Error bodies are `application/json; charset=utf-8`.

Join (`phx_reply` `{"status":"error","response":{"reason":…}}`), token
from `user_token`, else `access_token` (an `sb_` key means the socket
token), else the socket token:

| When | `reason` |
|---|---|
| Topic `realtime:` | `TopicNameRequired: You must provide a topic name` |
| Expired | `InvalidJWTToken: Token has expired <n> seconds ago` |
| Missing `role`/`exp` | ``InvalidJWTToken: Fields `role` and `exp` are required in JWT`` |
| Not a JWT | `MalformedJWT: The token provided is not a valid JWT` |
| Bad signature | `JwtSignatureError: Failed to validate JWT signature` |
| `alg` not HS* | `JwtSignerError: Failed to generate JWT signer, check your JWT secret or JWKS configuration` |
| Rounded `exp` not after now | `InvalidJWTToken: Token expiration time is invalid` |
| Other token failures | `Unknown Error on Channel` |

## Edge cases

- The self-hosted stack has one tenant (`realtime-dev`, not suspended,
  `private_only` and `presence_enabled` false), so tenant lookup, rate
  limits and the connect/join error backoff are not modelled.
- Elixir rounds a float `exp`/`iat` after validation; a float `exp` in the
  past fails validation as a non-integer and is a 500 / unknown error.
- Boolean config fields accept `true`/`false` and the strings `"true"` /
  `"false"` in any case (`FlexibleBoolean`).

## Not implemented (501 or a not-implemented reason)

- No `JWT_SECRET` configured, `HS384`/`HS512` tokens, or a GET that is not
  a WebSocket upgrade: HTTP 501 `MEGABASE_NOT_IMPLEMENTED`.
- Joins that ask for private channels (RLS on `realtime.messages`),
  presence, Postgres changes (including the old topic-only API), broadcast
  replay or `replication_ready`; and client `broadcast`, `presence` and
  `access_token` events. These reply `phx_reply` error with a
  `MEGABASE_NOT_IMPLEMENTED` response (`reason`, `code`, `component`,
  `unit`).
- Binary (V2 user broadcast) frames are ignored.
- Channel token expiry timers and server `system`/`disconnect` pushes.
- `/realtime/v1/longpoll`.

## Judge cases

None yet. WebSocket cases need harness support; they go in a separate
`review/issue-70-cases` PR. The handshake errors above are plain HTTP and
can be judged today.
