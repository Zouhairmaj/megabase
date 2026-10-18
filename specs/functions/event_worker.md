# Functions: event worker (issue 73)

Status: draft
Level: 4
Unit ids (`coverage/units.json`): `functions:op:op_user_worker_log`,
`functions:op:op_event_accept`

Pin edge-runtime `v1.77.4`. Megabase runs a function as a plain `deno run`
child (`specs/functions/invoke.md`). These ops live in edge-runtime's own
`deno_core`, which that process does not have. A unit is marked only when a
function can observe the same behavior.

## Upstream

| Unit | Behavior | File:line |
|---|---|---|
| `op_user_worker_log` | `msg: &str`, `level: i32`. `LogLevel::try_from(level as u8)` (`0` Debug, `1` Info, `2` Warning, `3` Error, anything else Debug). If op state holds an `UnboundedSender<WorkerEventWithMetadata>`, send `WorkerEvents::Log { msg, level }` with a clone of `EventMetadata` (or the default). If that send fails, return the error. If there is no sender and the `tracing` feature is off, `log::error!("[{:?}] {}", level, msg)`. Always `Ok(())` on the no-sender path | `ext/event_worker/js_interceptors.rs:11`–`:54`, `ext/event_worker/events.rs:56`–`:78` |
| `op_event_accept` | Take the `UnboundedReceiver<WorkerEventWithMetadata>` out of op state. If it is missing, `bail!("events worker receiver not available")`. `recv`: `Some` → `RawEvent::Event`, `None` → put the receiver back, wake the op waker, `RawEvent::Done` | `ext/event_worker/lib.rs:14`–`:39` |
| Listener | `EventManager` (`SupabaseEventListener`) calls `op_event_accept`. `"Done"` ends the iterator. Otherwise `event_type` is the first key of `event`, and the value is `{ timestamp, event_type, event, metadata }` | `ext/event_worker/event_worker.js:6`–`:24` |
| Who calls the log op | A user worker that is not being inspected replaces `console` with `new Console((msg, level) => op_user_worker_log(msg, level))` | `ext/runtime/js/bootstrap.js:595`–`:604` |
| Who calls accept | `globalThis.EventManager` is installed only when the worker kind is `"event"` | `ext/runtime/js/bootstrap.js:779`–`:782` |
| Sender installed | A user worker gets the sender only when `events_msg_tx` is `Some` | `crates/base/src/runtime/mod.rs:1171`–`:1176` |
| Sender absent | The pool copies `worker_event_sender`, which is `None` unless `--event-worker` was passed | `crates/base/src/worker/pool.rs:398`, `cli/src/main.rs:149`–`:150`, `crates/base/src/server.rs:308` |
| Reference stack | `functions` starts `edge-runtime start --main-service …/main` with no `--event-worker` | `vendor/supabase/docker/docker-compose.yml:475`–`:480` |
| Logger line | Without the `tracing` feature (not a default of `cli`), non-debug records are `writeln` of the message only, so the bytes are `[{level:?}] {msg}` plus a newline. `msg` from `Console` already ends in `\n` | `cli/src/logger.rs:25`–`:26`, `cli/Cargo.toml` `[features]` |

`WorkerEventWithMetadata` serializes as
`{ "event": { "<Variant>": … }, "metadata": { "service_path", "execution_id", "otel_attributes" } }`
(externally tagged `WorkerEvents`, snake_case metadata, `LogLevel` as its
variant name). `RawEvent::Done` is the JSON string `"Done"`;
`RawEvent::Event` is `{ "Event": <WorkerEventWithMetadata> }`.

## op_user_worker_log

Served and marked (`crates/megabase-functions/src/event_worker.rs`).

A user function's `console.debug` / `log` / `info` / `warn` / `error` builds
the message the op would receive (`args` joined with a space, strings left
as-is, anything else `Deno.inspect`, then `\n`) and passes the method's
level. The reference stack has no event worker, so there is no sender: the
worker writes the no-tracing fallback to its stderr, including the logger's
extra newline. `console.log("hello")` therefore writes `[Info] hello\n\n`.

The channel path is the same function: a present sender queues
`{ event: { "Log": { msg, level } }, metadata }` and writes nothing. A
dropped receiver returns an error (`channel closed`, the `SendError`
display) and does not fall back to stderr.

`level as u8` keeps the low eight bits (`256` → Debug, `257` → Info,
`-1` → Debug).

## op_event_accept

Implemented and tested, not marked, not installed on a user worker.

The reference stack never starts an event worker, and a user worker has no
`EventManager`. Installing one would add a global the reference does not
have. `event_accept` still follows the op: no receiver → the bail string;
otherwise take the receiver, wait, put it back; an event is
`RawEvent::Event`; a closed channel is `RawEvent::Done` and reports the
wake. The listener's `event_type` / `event` / `metadata` projection is
tested against that JSON. `timestamp` is `new Date().toISOString()` in
`event_worker.js`, not part of the op.

## Gaps

- `console.dir`, `trace`, `assert`, `table` and the rest stay Deno's
  console. Upstream replaces the whole `Console`.
- `console.log("%s", x)` does not apply the format specifier. The op
  receives whatever `Console` already formatted; Megabase joins the
  arguments.
- No event worker is started, so a function cannot iterate `EventManager`.

## Judge cases

None. The vendored `hello` function does not call `console` or
`EventManager`, and a fixture function would be TypeScript outside
`vendor/`. No protected judge file is needed. Tests call `user_worker_log`
and `event_accept`, and run the console shim and the listener projection
under `node`.
