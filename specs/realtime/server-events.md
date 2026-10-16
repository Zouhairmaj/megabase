# Realtime: server events (issue 69)

Status: partial (`broadcast`, `disconnect`, `presence_diff`,
`presence_state`, `system` served; `postgres_changes` not implemented)
Unit ids (`coverage/units.json`): `realtime:server-event:broadcast`,
`realtime:server-event:disconnect`, `realtime:server-event:postgres_changes`,
`realtime:server-event:presence_diff`, `realtime:server-event:presence_state`,
`realtime:server-event:system`
Level: 3

The pushes the server sends to a socket without being asked: what a
subscriber receives, plus the socket's forced close. The client events
that cause most of them are in [`client-events.md`](client-events.md).
The handshake, join and frame encodings are in
[`websocket.md`](websocket.md), and token expiry in
[`protocol.md`](protocol.md). This spec does not repeat them.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| `handle_info(%{event: type, payload: payload}, …)`: relay a broadcast as a push | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:347`–`:352` |
| Fastlane `broadcast` push per serializer | `vendor/realtime/lib/realtime_web/channels/realtime_channel/message_dispatcher.ex:101`–`:200` |
| Replay pushes (`meta.replayed`), private channels only | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:292`–`:299`, `:1206`–`:1219` |
| `maybe_replay_messages/5`: `replay` on a public channel is `:invalid_replay_channel` | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:1176`–`:1178`, `:263`–`:264` |
| `limit_joins/2` and the `:too_many_joins` join error with `disconnect` | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:99`, `:193`–`:196`, `:705`–`:722` |
| `joins_per_second_rate/2`: tick 5 s, 12 buckets, `avg` limit | `vendor/realtime/lib/realtime/tenants.ex:191`–`:214` |
| `RateCounter`: first tick at once, `avg = sum / len / tick_s`, trigger at `>=`, release at `<`, 5 min idle shutdown | `vendor/realtime/lib/realtime/rate_counter/rate_counter.ex:25`–`:290` |
| `TENANT_MAX_JOINS_PER_SECOND` default 100 | `vendor/realtime/config/runtime.exs:150` |
| `check_tenant/1` on connect; `:too_many_joins` is 429 | `vendor/realtime/lib/realtime_web/channels/tenant_rate_limiters.ex:11`–`:42`, `vendor/realtime/lib/realtime_web/channels/user_socket.ex:78`, `:122`–`:125`, `:160` |
| `presence_diff` push | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:451`–`:468` |
| `presence_state` push | `vendor/realtime/lib/realtime_web/channels/realtime_channel/presence_handler.ex:92`–`:117` |
| `push_system_message/5` and `shutdown_response/2` | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:859`–`:897` |
| `postgres_changes` dispatch (`{"ids","data"}`; old API: the change type as event) | `vendor/realtime/lib/extensions/postgres_cdc_rls/message_dispatcher.ex:10`–`:44` |

`Phoenix.Socket` 1.7 (`__info__/2` on `%Broadcast{event: "disconnect"}`
returns `{:stop, {:shutdown, :disconnected}, state}`) is not vendored.

Pin: realtime `v2.143.3`.

## broadcast

A subscriber of `realtime:<t>` receives every message broadcast to the
topic: from another socket's `broadcast` event (and its own with
`self`), or from `POST /api/broadcast` ([`http-api.md`](http-api.md)).

| Serializer | Push |
|---|---|
| V2, JSON payload | `[null, null, topic, "broadcast", payload]`; a non-map payload is skipped |
| V1, JSON payload | `{"topic","event":"broadcast","payload","ref":null}` |
| V2, binary user broadcast | kind 4 frame (`client-events.md`) |
| V1, binary user broadcast | JSON payload only, else skipped |

A join with a `broadcast.replay` key (any value, `null` included) on a
public channel replies
`{"reason":"UnableToReplayMessages: Replay is not allowed for public channels"}`.
This check runs after the token checks and before Postgres changes.

## disconnect

The self-hosted tenant has `max_joins_per_second` 100. One counter per
process counts joins. Its first tick runs when the first connect or join
starts it, then every 5 s. Each tick pushes the joins since the last tick
onto a bucket of at most 12 counts. The limit is triggered while
`sum(bucket) / len(bucket) / 5 >= 100` as of the last tick. A counter
whose last 12 counts are all zero at a 5-minute idle check stops; the
next connect or join starts a new one.

| When | Result |
|---|---|
| `phx_join` on `realtime:<t>` (not `realtime:`), limit not triggered | the join is counted, then the usual join checks |
| `phx_join`, limit triggered | not counted; `phx_reply` error `{"reason":"ClientJoinRateLimitReached: Too many joins per second"}`, then the socket closes (every channel on it stops) |
| Connect, limit triggered | 429 `{"error":"Too many joins per second"}` |

A join counts even if a later step (token, options) rejects it. The close
frame Megabase sends has no status code; the code Cowboy sends through
WebSockAdapter is not vendored. The `tenant_not_found` disconnect cannot
happen on the single self-hosted tenant.

## presence_diff and presence_state

Served as in [`websocket.md`](websocket.md) (grouping, `phx_ref`) and
[`client-events.md`](client-events.md) (track, untrack, resync).

## system

`["<join_ref>", null, topic, "system", {"channel": <t without "realtime:">, "extension": "system", "status": "error", "message": <message>}]`,
then `phx_close`. Served for the presence shutdowns (`client-events.md`)
and token shutdowns (`protocol.md`).

## Not implemented

- `postgres_changes`: CDC from the tenant database (`postgres_cdc_rls`).
  Joins with `postgres_changes` entries, or without `config` (the old
  API), reply a `MEGABASE_NOT_IMPLEMENTED` response. So do the `system`
  pushes with extension `postgres_changes` ("Subscribed to PostgreSQL",
  subscribe errors).
- `replication_ready`: the "Replication connection established" `system`
  push (`MEGABASE_NOT_IMPLEMENTED` join reply).
- Replay pushes, which need private channels.
- `Too many messages per second` (`max_events_per_second`) and the other
  tenant limits (see `websocket.md`).

## Judge cases

None yet: the judge harness only sends HTTP steps, so these WebSocket
cases need harness support first. They go in a separate
`review/issue-69-cases` PR. Triggering the join-rate limit takes about
1000 joins within one 5-second tick, which may be too heavy for a judge
case.
