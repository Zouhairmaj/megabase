# Realtime: protocol events (issue 68)

Status: served for public channels
Unit ids (`coverage/units.json`): `realtime:protocol-event:phx_join`,
`realtime:protocol-event:phx_reply`, `realtime:protocol-event:phx_leave`,
`realtime:protocol-event:phx_close`, `realtime:protocol-event:phx_error`,
`realtime:protocol-event:access_token` (and the server side of the same
event, `realtime:client-event:access_token`)
Level: 3

The Phoenix channel events realtime-js names in `CHANNEL_EVENTS`, as the
Realtime socket answers them. Framing, the handshake and join error
reasons are in [`websocket.md`](websocket.md), broadcast and presence in
[`client-events.md`](client-events.md); this spec
covers the event lifecycle and the token refresh.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| `CHANNEL_EVENTS`: `phx_close`, `phx_error`, `phx_join`, `phx_reply`, `phx_leave`, `access_token` | `vendor/supabase-js/packages/core/realtime-js/src/lib/constants.ts:51`–`:58` |
| `setAuth` pushes `access_token` `{"access_token": token}` to every joined channel | `vendor/supabase-js/packages/core/realtime-js/src/RealtimeClient.ts:673`–`:689` |
| `join/3` with `confirm_token/1`: the `:confirm_token` timer at `min(1 h, exp - now)` | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:40`, `:101`, `:829`–`:857` |
| `handle_in("access_token", …)`: no-ops, throttle, hold | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:539`–`:589` |
| `apply_access_token/3` and its shutdown messages | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:597`–`:686` |
| `handle_info(:confirm_token, …)` and `:apply_pending_access_token` | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:399`–`:433` |
| `shutdown_response/2`: `system` push, then `{:stop, :normal, …}` | `vendor/realtime/lib/realtime_web/channels/realtime_channel.ex:859`–`:864` |
| `authorize_conn/3`: `Token has expired <n> seconds ago` | `vendor/realtime/lib/realtime_web/channels/auth/channels_authorization.ex:19`–`:37` |
| `ACCESS_TOKEN_THROTTLE_MS` default 10 s | `vendor/realtime/config/runtime.exs:15` |

`phx_reply`, `phx_close` and `phx_error` frames come from Phoenix 1.7
`Phoenix.Socket` (`reply/2`, `encode_close/3`, `encode_on_exit/4`), which
Realtime uses unchanged and which is not vendored.

Pins: realtime `v2.143.3`, supabase-js `v2.117.3`.

## Events

| Event | Direction | Frame |
|---|---|---|
| `phx_join` | client | `[join_ref, ref, "realtime:<t>", "phx_join", {config, access_token}]`; answered by `phx_reply` (see `websocket.md`) |
| `phx_reply` | server | `[join_ref, ref, topic, "phx_reply", {"status": "ok"\|"error", "response": {...}}]`, `ref` of the request; heartbeats use `join_ref` null |
| `phx_leave` | client | answered by `phx_reply` ok `{}`, then `phx_close` |
| `phx_close` | server | `[join_ref, join_ref, topic, "phx_close", {}]`: the channel stopped normally (leave, duplicate join, shutdown) |
| `phx_error` | server | `[join_ref, join_ref, topic, "phx_error", {}]`: the channel process crashed |
| `access_token` | client | `[join_ref, ref, topic, "access_token", {"access_token": token}]`; never replied to |

## `access_token`

On a joined channel, in clause order:

1. `access_token` starting with `sb_`: ignored.
2. Equal to the channel's current token: ignored.
3. `null`: ignored.
4. Equal to the held (pending) token: ignored.
5. Any other string: applied now if no refresh was applied on this
   channel yet, or the last one was at least 10 s ago. Otherwise it is
   held: the first hold in a window arms a timer for the end of the
   window; later holds replace the held token without a new timer.
6. Anything else (non-string, missing key, non-object payload): the
   `handle_in/3` catch-all, ignored.

Applying a token confirms it. On success the channel keeps the new token
and re-arms `:confirm_token`; nothing is sent. On failure the channel
sends a `system` push and stops:

```json
["1", null, "realtime:room", "system",
 {"channel": "room", "extension": "system", "status": "error", "message": "<message>"}]
["1", "1", "realtime:room", "phx_close", {}]
```

| Token | `message` |
|---|---|
| Expired | `Token has expired <n> seconds ago` |
| No `role` or `exp` | ``Fields `role` and `exp` are required in JWT`` |
| Not a JWT | `The token provided is not a valid JWT` |
| `alg` not HS* | `Failed to generate JWT signer, check your JWT secret or JWKS configuration` |
| Bad signature | `:signature_error` (`inspect/1` of the atom) |
| Rounded `exp` equal to now | `0` (`inspect/1` of `exp - now`) |

## Timers

- `:confirm_token` fires `min(1 h, exp - now)` after each successful
  confirmation (join, refresh, or the timer itself). It re-checks the
  current token; an expired one shuts the channel down with
  `Token has expired <n> seconds ago`. If a held token is waiting, it is
  applied instead.
- `:apply_pending_access_token` applies the held token, if any.

Timers use the server clock in milliseconds; `exp` is compared in whole
seconds (`Joken.current_time/0`).

## Edge cases

- Public channels have no policies, so `maybe_assign_policies/3` and the
  read-revocation check pass, and the fastlane metadata does not change.
- A refresh on a channel joined with an `sb_` key compares against the
  socket token, which the join assigned.
- `phx_error` is sent for the presence crash in `websocket.md` and when
  `:confirm_token` meets `{:error, :token_malformed, _}` (no clause
  matches it). The second is unreachable in practice: the token was
  confirmed before.

## Not implemented

- A refreshed token with `HS384`/`HS512`, a non-object header or claims,
  a non-numeric `exp`/`iat`, or a bad percent escape: upstream answers
  with an `inspect/1` of a term Megabase does not model (or crashes).
  Megabase replies `phx_reply` error with a `MEGABASE_NOT_IMPLEMENTED`
  response for `realtime:client-event:access_token` and keeps the channel.
- Private channels: the policy re-evaluation and the
  `You no longer have permission to read from this Channel topic: <t>`
  shutdown. Private joins are not served (`websocket.md`).
- Postgres changes resubscription after a refresh.

## Judge cases

None yet. They go in a separate `review/issue-68-cases` PR once the judge
harness drives WebSocket frames.
