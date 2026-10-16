# Realtime: WebSocket endpoint (issue 70)

Status: partial (connect, heartbeat, public join and leave; public
broadcast; presence)
Unit ids (`coverage/units.json`): `realtime:route:GET /realtime/v1/websocket`,
`realtime:client-event:broadcast`, `realtime:client-event:presence`,
`realtime:server-event:presence_state`, `realtime:server-event:presence_diff`
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
| `handle_in("broadcast" …)` on a public channel | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:505` |
| Public broadcast: `self` (`pubsub_broadcast` vs `_from`), `ack` replies, payload size | `vendor/realtime/lib/realtime_web/channels/realtime_channel/broadcast_handler.ex:110`–`:171` |
| Fan-out to subscribers, sender skipped, per-serializer encoding | `vendor/realtime/lib/realtime_web/channels/realtime_channel/message_dispatcher.ex:101`–`:200` |
| V2 broadcast push `[null, null, topic, event, payload]`; non-map payload raises | `vendor/realtime/lib/realtime_web/socket/v2_serializer.ex:63`–`:70` |
| `validate_payload_size/2`: `max_payload_size_in_kb * 1000 + 500` of `:erlang.external_size/1` | `vendor/realtime/lib/realtime/tenants.ex:539`–`:544` |
| `handle_in("presence" …)` replies and shutdowns | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:509`–`:537` |
| `PresenceHandler.handle/2`: track, untrack, resync, unchanged payload, rate limit | `vendor/realtime/lib/realtime_web/channels/realtime_channel/presence_handler.ex:155`–`:356` |
| `presence_key/1`: non-empty string, number or boolean, else `UUID.uuid1()` | `vendor/realtime/lib/realtime_web/channels/payloads/join.ex:41`–`:48` |
| `CLIENT_PRESENCE_MAX_CALLS` 5, `CLIENT_PRESENCE_WINDOW_MS` 30000 | `vendor/realtime/config/runtime.exs:13`–`:14` |
| `shutdown_response/2`: `system` push then stop | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:859`–`:888` |

Heartbeat, unmatched-topic, duplicate-join and leave/close handling come
from Phoenix 1.7 `Phoenix.Socket` (`__in__`, `encode_ignore`,
`encode_close`, `encode_on_exit`), which Realtime uses unchanged and which
is not vendored. Presence grouping (`{key: {"metas": [...]}}`), the
`{"joins","leaves"}` diff, `phx_ref` (base64 of system time, a hash and a
unique integer) and `phx_ref_prev` on updates come from Phoenix.Presence
and Phoenix.Tracker 1.7, also not vendored.

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
| `realtime:<t>` / `phx_join` (public, no postgres changes) | `phx_reply` ok `{"postgres_changes":[]}`; with `presence.enabled`, then `presence_state` |
| `phx_join` on an already joined topic | `phx_close` for the old join, then the new join's reply |
| joined topic / `phx_leave` | `phx_reply` ok `{}`, then `phx_close` `{}` with `ref` = `join_ref` |
| joined topic / `broadcast` | the payload as a `broadcast` push to every other socket on the topic (and this one with `broadcast.self`), then `phx_reply` ok `{}` only with `broadcast.ack` |
| joined topic / `presence` `track` | `phx_reply` ok `{}`; `presence_state` if presence was off; `presence_diff` to every socket on the topic |
| joined topic / `presence` `untrack` | `phx_reply` ok `{}`; a `leaves` `presence_diff` if something was tracked |

Pushes use `ref` null. `presence_state` and `system` carry the channel's
`join_ref`; broadcasts and `presence_diff` (fastlane) carry `join_ref`
null. Fastlane pushes go to every subscriber of the topic, whatever its
own presence or broadcast config.

Broadcast:

- The payload goes out as received. A V2 socket skips a payload that is
  not a JSON object (`fastlane!` raises); a V1 socket still gets it.
- `self` and `ack` are `FlexibleBoolean`s.
- A payload whose Erlang external size passes 3000500 bytes is not sent;
  with `ack` the reply is `phx_reply` error `{"error":"payload_size_exceeded"}`.

Presence:

- The event is ASCII-lowercased. A payload without `event` replies ok and
  does nothing.
- Every event counts against the channel's client limit: 5 per 30 s
  window, the window starting at the first event after the previous one
  ends.
- `track` stores `payload.payload` (default `{}`). The same payload as the
  last track replies ok with no diff. A different one updates: the diff
  joins the new meta (with `phx_ref_prev`) and leaves the old one.
- Presence turns on for the channel at the first track even if the join
  did not enable it; that track also pushes `presence_state`.
- A channel's presence is untracked when it leaves, is rejoined, shuts
  down, crashes or its socket closes; the other sockets get the `leaves`
  diff.
- The diff to the tracking socket is pushed after its reply (and after
  the resync `presence_state`): Phoenix.Presence dispatches diffs
  asynchronously.
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

Presence (`phx_reply` unless noted):

| When | Answer |
|---|---|
| `track` with a `payload` that is not an object | error `{"reason":"Presence track payload must be a map"}` |
| Unknown presence event | error `{}` |
| `event` not a string | the channel crashes: `phx_error` `{}` with `ref` = `join_ref` |
| Over the client limit | `system` push `{"channel":<t>,"extension":"system","message":"Client presence rate limit exceeded","status":"error"}`, then `phx_close` |
| `track` payload over 3000500 bytes | the same with `"Track message size exceeded"` |

## Edge cases

- The self-hosted stack has one tenant (`realtime-dev`, not suspended,
  `private_only` and `presence_enabled` false), so tenant lookup, rate
  limits and the connect/join error backoff are not modelled.
- Elixir rounds a float `exp`/`iat` after validation; a float `exp` in the
  past fails validation as a non-integer and is a 500 / unknown error.
- Boolean config fields accept `true`/`false` and the strings `"true"` /
  `"false"` in any case (`FlexibleBoolean`).
- Topics, subscribers and presences live in the process; there is one
  node, so Phoenix.Tracker replication does not apply.
- Payload sizes follow the external term format of the Jason-decoded
  value (maps, binaries, UTF-8 atoms, `NEW_FLOAT_EXT`). An integer past
  `u64` decodes as a float here and a bignum upstream, a few bytes apart.
- Presence keys are JSON object keys, so the number `5` and the string
  `"5"` share a group. A key from `UUID.uuid1()` uses a random multicast
  node id.
- The tenant-wide limits (`max_events_per_second`,
  `max_presence_events_per_second`) are averages over time and are not
  modelled.

## Not implemented (501 or a not-implemented reason)

- No `JWT_SECRET` configured, `HS384`/`HS512` tokens, or a GET that is not
  a WebSocket upgrade: HTTP 501 `MEGABASE_NOT_IMPLEMENTED`.
- Joins that ask for private channels (RLS on `realtime.messages`),
  Postgres changes (including the old topic-only API), broadcast replay or
  `replication_ready`; and client `access_token` events. These reply
  `phx_reply` error with a `MEGABASE_NOT_IMPLEMENTED` response (`reason`,
  `code`, `component`, `unit`).
- Binary (V2 user broadcast) frames are ignored.
- Channel token expiry timers and server `disconnect` pushes; `system`
  pushes other than the presence shutdowns.
- Broadcast via the REST API (`POST /api/broadcast`).
- `/realtime/v1/longpoll`.

## Judge cases

None yet. WebSocket cases need harness support; they go in a separate
`review/issue-70-cases` PR. The handshake errors above are plain HTTP and
can be judged today.
