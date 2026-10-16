# Realtime: server events (issue 69)

Status: served (`broadcast`, `disconnect`, `postgres_changes`,
`presence_diff`, `presence_state`, `system`)
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
| `pg_change_params/5`, `postgres_cdc_subscribe/2`, `add_id_to_postgres_changes/1` | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:116`–`:133`, `:899`–`:964`, `:1047`–`:1052` |
| `:postgres_subscribe`, "Subscribed to PostgreSQL", retry 5–10 s, fatal reasons | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:44`, `:359`–`:397`, `:696`–`:703`, `:966`–`:1045` |
| Poll loop, `generate_record/1` | `vendor/realtime/lib/extensions/postgres_cdc_rls/replication_poller.ex` |
| Slot SQL and `list_changes` query | `vendor/realtime/lib/extensions/postgres_cdc_rls/replications.ex` |
| Defaults (`supabase_realtime`, `supabase_realtime_replication_slot`, 100 changes); self-hosted `poll_interval_ms` 100, `poll_max_record_bytes` 1048576 | `vendor/realtime/lib/extensions/postgres_cdc_rls/db_settings.ex:9`–`:12`, `vendor/realtime/priv/repo/seeds.exs:40`–`:41` |
| Change JSON (sorted keys of the derived struct) | `vendor/realtime/lib/realtime/adapters/changes.ex:15`–`:59` |
| Tenant CDC schema (`realtime.subscription`, `apply_rls`, `list_changes`, …) | `vendor/realtime/priv/repo/tenant_schema/realtime/` |

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

## postgres_changes

Needs `DATABASE_URL`. At startup Megabase installs the CDC objects of the
tenant schema (types, `realtime.subscription`, `apply_rls`,
`list_changes`, …) and the `supabase_realtime` publication when missing.
Without a database, or when that install fails, joins that would subscribe
reply `MEGABASE_NOT_IMPLEMENTED`.

| Step | Behavior |
|---|---|
| Join, new API (`config` present) | the non-null entries of a non-empty `config.postgres_changes` list |
| Join, old API (no `config`) | one entry from the topic: `schema[:table[:filter]]` |
| Join reply | `{"postgres_changes": [entry + {"id": n}, …]}` |
| After the join | insert the subscriptions with the token's claims, then `system` `{"extension":"postgres_changes","status":"ok","message":"Subscribed to PostgreSQL","channel":<t>}` |
| Subscribe error | `system` with `status` `"error"` and the message; malformed params and insert failures are final, other errors retry 5–10 s later |
| Each change | for every channel with a matching subscription id: new API `postgres_changes` `{"data": change, "ids": [ids…]}` (ids in reverse join order), old API the change with its type (`INSERT`, `UPDATE`, `DELETE`) as the event |
| Change JSON | `columns`, `commit_timestamp`, `errors` (null when empty), `old_record` (UPDATE, DELETE), `record` (INSERT, UPDATE), `schema`, `table`, `type`, in that order |
| Leave or close | the channel's subscriptions are deleted |

The poller holds one connection with the temporary wal2json slot
`supabase_realtime_replication_slot` (`_<SLOT_NAME_SUFFIX>` when set). It
polls again at once after changes, after 100 ms (+50–100 ms jitter) when
the slot had changes for no subscriber, and after 500 ms when idle. An
empty publication creates no slot; the publication is checked every 60 s.
Upstream stops it after 6 consecutive failures and its supervisor restarts it. Megabase never stops it: after 6 failures it keeps retrying at the longest backoff and logs each failure. At startup it also deletes the rows left in `realtime.subscription` (skipped when `SLOT_NAME_SUFFIX` is set). After a token refresh, Megabase stores the channel's subscriptions again with the new claims, which upstream does not do. The schema install fails when an existing type or table lacks the attributes the subscription SQL uses.

Differences from upstream:

- `id` is `:erlang.phash2(params)` upstream. Megabase hashes the entry's
  JSON (FNV-1a) into the same `0..2^27` range, so ids differ from
  upstream's but match between the reply and the pushes. The reply entry's
  keys are sorted (`id` among them); upstream puts `id` first.
- The `ReplicationSlotBeingUsed` recovery (`pg_terminate_backend` on the
  slot holder) is not ported: on a cluster shared with another Realtime it
  would end that server's slot. Use `SLOT_NAME_SUFFIX` there.
- The `db_events_per_second` and subscription-error rate counters are not
  modeled; nor is `postgres_changes_wait_timeout` (the join never waits).

## Not implemented

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
