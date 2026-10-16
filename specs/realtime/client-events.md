# Realtime: client events (issue 66)

Status: served for public channels
Unit ids (`coverage/units.json`): `realtime:client-event:access_token`,
`realtime:client-event:broadcast`, `realtime:client-event:presence`
Level: 3

The events a client sends on a joined `realtime:<t>` channel. Routing
(unmatched topics, stale join refs, duplicate joins), the handshake and
text framing are in [`websocket.md`](websocket.md). `access_token`
refreshes are in [`protocol.md`](protocol.md#access_token); this spec
does not repeat them.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| `handle_in("broadcast" …)` on a public channel | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:505` |
| Public broadcast: `self` (`pubsub_broadcast` vs `_from`), `ack` replies, payload size | `vendor/realtime/lib/realtime_web/channels/realtime_channel/broadcast_handler.ex:110`–`:171` |
| `build_broadcast/2`: a user broadcast push becomes `%UserBroadcast{}` without its metadata | `vendor/realtime/lib/realtime_web/channels/realtime_channel/broadcast_handler.ex:174`–`:189` |
| Fan-out to subscribers, sender skipped, per-serializer encoding; V1 gets `convert_to_json_broadcast/1` | `vendor/realtime/lib/realtime_web/channels/realtime_channel/message_dispatcher.ex:101`–`:200` |
| `convert_to_json_broadcast/1`: JSON payload that decodes, emitted verbatim | `vendor/realtime/lib/realtime_web/socket/user_broadcast.ex:21`–`:47` |
| V2 binary frames: `@push` 0, `@broadcast` 2, `@user_broadcast_push` 3, `@user_broadcast` 4; `fastlane!/1`, `decode!/2`, 255-byte fields | `vendor/realtime/lib/realtime_web/socket/v2_serializer.ex:9`–`:241` |
| V2 broadcast push `[null, null, topic, event, payload]`; non-map payload raises | `vendor/realtime/lib/realtime_web/socket/v2_serializer.ex:63`–`:70` |
| `validate_payload_size/2`: `max_payload_size_in_kb * 1000 + 500` of `:erlang.external_size/1` | `vendor/realtime/lib/realtime/tenants.ex:539`–`:544` |
| `handle_in("presence" …)` replies and shutdowns | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:509`–`:537` |
| `PresenceHandler.handle/2`: track, untrack, resync, unchanged payload, rate limit | `vendor/realtime/lib/realtime_web/channels/realtime_channel/presence_handler.ex:155`–`:356` |
| `CLIENT_PRESENCE_MAX_CALLS` 5, `CLIENT_PRESENCE_WINDOW_MS` 30000 | `vendor/realtime/config/runtime.exs:13`–`:14` |
| `handle_in("access_token" …)` | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:539`–`:569` |
| realtime-js sends `broadcast` with a string `event` as a binary `userBroadcastPush` | `vendor/supabase-js/packages/core/realtime-js/src/lib/serializer.ts:25`–`:135` |

Presence grouping, the `{"joins","leaves"}` diff and `phx_ref` come from
Phoenix.Presence and Phoenix.Tracker 1.7 (see `websocket.md`).

Pins: realtime `v2.143.3`, supabase-js `v2.117.3`.

## Inputs

On a joined channel:

| Event | Text frame payload | V2 binary frame |
|---|---|---|
| `broadcast` | any JSON; realtime-js sends `{"type":"broadcast","event":e,"payload":p}` | kind 3 `userBroadcastPush` (below), or kind 0 `push` with event `broadcast` |
| `presence` | `{"type":"presence","event":"track"\|"untrack","payload":meta}` | kind 0 `push`: no `event` key |
| `access_token` | `{"access_token": token}` | kind 0 `push`: the catch-all, ignored |

realtime-js 2.x connects with `vsn=2.0.0`, so `channel.send({type:
"broadcast", event, payload})` arrives as a binary frame:

```
kind=3 | join_ref_size | ref_size | topic_size | event_size | metadata_size | encoding
join_ref | ref | topic | user_event | metadata | user_payload
```

Sizes are one byte each. `encoding` 0 is binary and anything else JSON.
`metadata` is empty or JSON (only the keys the client allow-lists).
The kind 0 `push` frame is
`0 | join_ref_size | ref_size | topic_size | event_size | join_ref | ref | topic | event | data`.
A V1 socket decodes a binary frame as JSON text.

## Outputs

Broadcast:

- The payload goes to every other socket on the topic, and to this one
  with `broadcast.self`. The echo comes before the `ack` reply.
- Without `broadcast.ack` there is no reply; with it, `phx_reply` ok `{}`.
- `self` and `ack` are `FlexibleBoolean`s.
- Per subscriber serializer:

| Sent as | V2 subscriber | V1 subscriber |
|---|---|---|
| JSON object payload | text `[null, null, topic, "broadcast", payload]` | text `{"event":"broadcast","payload":…,"ref":null,"topic":…}` |
| JSON non-object payload | skipped (`fastlane!` raises) | text, as above |
| kind 3, JSON encoding | binary `4 \| topic_size \| event_size \| 0 \| 1 \| topic \| user_event \| user_payload` | text with payload `{"event":e,"payload":<user_payload bytes>,"type":"broadcast"}`, only if the bytes are valid JSON |
| kind 3, binary encoding | binary as above with encoding 0 | skipped |
| kind 0 | binary `2 \| topic_size \| 9 \| topic \| "broadcast" \| data` | skipped |

The relayed user broadcast never carries the client's metadata
(`metadata_size` 0, no `meta` key).

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
- `untrack` replies ok `{}`; a `leaves` `presence_diff` follows if
  something was tracked.
- A channel's presence is untracked when it leaves, is rejoined, shuts
  down, crashes or its socket closes; the other sockets get the `leaves`
  diff.
- The diff to the tracking socket is pushed after its reply (and after
  the resync `presence_state`): Phoenix.Presence dispatches diffs
  asynchronously.

## Errors

Broadcast: a payload whose Erlang external size passes 3000500 bytes is
not sent; with `ack` the reply is `phx_reply` error
`{"error":"payload_size_exceeded"}`. The size of a kind 3 frame is that of
the tuple `{user_event, :json | :binary, user_payload, metadata}`; of a
kind 0 frame, `{:binary, data}`.

Presence (`phx_reply` unless noted):

| When | Answer |
|---|---|
| `track` with a `payload` that is not an object | error `{"reason":"Presence track payload must be a map"}` |
| Unknown presence event | error `{}` |
| `event` not a string | the channel crashes: `phx_error` `{}` with `ref` = `join_ref` |
| Over the client limit | `system` push `{"channel":<t>,"extension":"system","message":"Client presence rate limit exceeded","status":"error"}`, then `phx_close` |
| `track` payload over 3000500 bytes | the same with `"Track message size exceeded"` |

## Edge cases

- A binary frame whose kind is not 0 or 3, that is truncated, whose
  metadata is not JSON, or (Megabase) whose `join_ref`, `ref`, topic or
  event is not UTF-8 is ignored like a malformed text frame.
- Binary-frame `join_ref` and `ref` are strings; realtime-js sends `""`
  for a missing one, so the `join_ref` check drops a frame sent before the
  join.
- A user broadcast's topic and event are at most 255 bytes (one length
  byte); an oversized field cannot be framed and the subscriber is
  skipped.
- Tenant-wide limits and payload-size rounding follow `websocket.md`.

## Not implemented

- Private channels: their joins already reply `MEGABASE_NOT_IMPLEMENTED`
  (see `websocket.md`), so the RLS checks and message persistence of
  private broadcasts and presence never run.
- A kind 0 `phx_join` (`{:binary, data}` join payload): `phx_reply` error
  with a `MEGABASE_NOT_IMPLEMENTED` response for
  `realtime:protocol-event:phx_join`, after the `phx_close` of a channel it
  replaces.

## Judge cases

None yet. WebSocket cases need harness support; they go in a separate
`review/issue-66-cases` PR.
