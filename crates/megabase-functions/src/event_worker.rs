// Ported from supabase/edge-runtime ext/event_worker/js_interceptors.rs,
// ext/event_worker/lib.rs, and ext/event_worker/event_worker.js (MIT), pin v1.77.4.

//! User-worker `console` logs and the events-worker accept loop.
//!
//! The self-hosted stack does not pass `--event-worker`, so a user worker
//! has no events channel: `console.debug` / `log` / `info` / `warn` /
//! `error` write the no-tracing fallback line. `EventManager` belongs to
//! the events worker and is not installed. Spec: `specs/functions/event_worker.md`.

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use thiserror::Error;
use tokio::sync::mpsc;

/// JavaScript that routes the five console methods through the no-sender
/// path of `op_user_worker_log`. It runs before the function is imported.
// megabase:unit functions:op:op_user_worker_log
pub const SOURCE: &str = r#"
// op_user_worker_log, no events channel (crates/megabase-functions/src/event_worker.rs).
{
  const levels = { debug: 0, log: 1, info: 1, warn: 2, error: 3 };
  const names = ["Debug", "Info", "Warning", "Error"];
  const encoder = new TextEncoder();
  const format = (args) => args.map((arg) =>
    typeof arg === "string" ? arg : Deno.inspect(arg)
  ).join(" ") + "\n";
  for (const method of Object.keys(levels)) {
    console[method] = (...args) => {
      const msg = format(args);
      const name = names[levels[method]] ?? "Debug";
      // logger.rs writeln adds a newline after the op's `[{level:?}] {msg}`.
      Deno.stderr.writeSync(encoder.encode(`[${name}] ${msg}\n`));
    };
  }
}
"#;

/// Log severity. Unknown bytes are [`LogLevel::Debug`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevel {
    #[default]
    Debug,
    Info,
    Warning,
    Error,
}

impl LogLevel {
    /// `LogLevel::try_from(level as u8)`.
    ///
    /// The cast keeps the low eight bits, as `level as u8` does upstream.
    #[must_use]
    pub fn from_smi(level: i32) -> Self {
        match level as u8 {
            0 => Self::Debug,
            1 => Self::Info,
            2 => Self::Warning,
            3 => Self::Error,
            _ => Self::Debug,
        }
    }

    /// Variant name, which is also the JSON string and the `{:?}` text.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "Debug",
            Self::Info => "Info",
            Self::Warning => "Warning",
            Self::Error => "Error",
        }
    }
}

/// `EventMetadata`. `execution_id` is the `Uuid` rendered as a string.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EventMetadata {
    /// Service path of the worker, when the pool set one.
    pub service_path: Option<String>,
    /// Worker key. Upstream stores a `Uuid`.
    pub execution_id: Option<String>,
    /// OpenTelemetry attributes copied onto the event.
    pub otel_attributes: Option<BTreeMap<String, String>>,
}

impl EventMetadata {
    fn to_json(&self) -> Value {
        let mut object = Map::new();
        object.insert("service_path".to_owned(), json_string(&self.service_path));
        object.insert("execution_id".to_owned(), json_string(&self.execution_id));
        object.insert(
            "otel_attributes".to_owned(),
            match &self.otel_attributes {
                Some(map) => Value::Object(
                    map.iter()
                        .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                        .collect(),
                ),
                None => Value::Null,
            },
        );
        Value::Object(object)
    }
}

fn json_string(value: &Option<String>) -> Value {
    match value {
        Some(text) => Value::String(text.clone()),
        None => Value::Null,
    }
}

/// One events-channel message: an externally tagged worker event plus metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkerEventWithMetadata {
    /// Externally tagged event, for example `{"Log": {"msg", "level"}}`.
    pub event: Value,
    /// Metadata cloned from the worker at the log call.
    pub metadata: EventMetadata,
}

/// `RawEvent`. `Done` is the JSON string `"Done"`.
#[derive(Debug, Clone, PartialEq)]
pub enum RawEvent {
    /// The next worker event.
    Event(WorkerEventWithMetadata),
    /// The channel closed.
    Done,
}

impl RawEvent {
    /// JSON the events-worker listener reads.
    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            Self::Done => Value::String("Done".to_owned()),
            Self::Event(event) => serde_json::json!({
                "Event": {
                    "event": event.event,
                    "metadata": event.metadata.to_json(),
                }
            }),
        }
    }
}

/// `log::error!("[{:?}] {}", level, msg)` arguments, before the logger's newline.
#[must_use]
pub fn fallback_line(level: LogLevel, msg: &str) -> String {
    format!("[{}] {msg}", level.as_str())
}

/// What `op_user_worker_log` did with one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogDisposition {
    /// Queued on the events channel. Nothing is written.
    Queued,
    /// No channel: the fallback line, without the logger newline.
    Fallback(String),
}

/// Failure of the channel path. The no-sender path does not fail.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum UserWorkerLogError {
    /// `UnboundedSender::send` failed because the receiver is gone.
    #[error("channel closed")]
    ChannelClosed,
}

/// `op_event_accept` when the events worker has no receiver.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EventAcceptError {
    /// Op state has no `UnboundedReceiver<WorkerEventWithMetadata>`.
    #[error("events worker receiver not available")]
    ReceiverUnavailable,
}

/// Run `op_user_worker_log`.
///
/// # Errors
///
/// [`UserWorkerLogError::ChannelClosed`] when `tx` is set and its receiver
/// has been dropped. A missing `tx` writes nothing and returns the fallback
/// line.
pub fn user_worker_log(
    tx: Option<&mpsc::UnboundedSender<WorkerEventWithMetadata>>,
    metadata: &EventMetadata,
    msg: &str,
    level: i32,
) -> Result<LogDisposition, UserWorkerLogError> {
    let level = LogLevel::from_smi(level);
    let Some(tx) = tx else {
        return Ok(LogDisposition::Fallback(fallback_line(level, msg)));
    };
    let event = WorkerEventWithMetadata {
        event: serde_json::json!({
            "Log": { "msg": msg, "level": level.as_str() }
        }),
        metadata: metadata.clone(),
    };
    tx.send(event)
        .map(|()| LogDisposition::Queued)
        .map_err(|_| UserWorkerLogError::ChannelClosed)
}

/// Puts the taken receiver back into `slot` when dropped, including when the
/// wait is cancelled.
struct ReceiverGuard<'a> {
    slot: &'a mut Option<mpsc::UnboundedReceiver<WorkerEventWithMetadata>>,
    receiver: Option<mpsc::UnboundedReceiver<WorkerEventWithMetadata>>,
}

impl Drop for ReceiverGuard<'_> {
    fn drop(&mut self) {
        if let Some(receiver) = self.receiver.take() {
            *self.slot = Some(receiver);
        }
    }
}

/// Run `op_event_accept` against `slot`.
///
/// The receiver is taken out before the wait and put back afterwards, so a
/// second accept that observes the empty slot fails with
/// [`EventAcceptError::ReceiverUnavailable`]. Dropping the future during the
/// wait puts the receiver back too. The `bool` is the wake flag: `true` only
/// for [`RawEvent::Done`].
///
/// # Errors
///
/// [`EventAcceptError::ReceiverUnavailable`] when `slot` is empty.
pub async fn event_accept(
    slot: &mut Option<mpsc::UnboundedReceiver<WorkerEventWithMetadata>>,
) -> Result<(RawEvent, bool), EventAcceptError> {
    let receiver = slot.take().ok_or(EventAcceptError::ReceiverUnavailable)?;
    let mut guard = ReceiverGuard {
        slot,
        receiver: Some(receiver),
    };
    let data = match guard.receiver.as_mut() {
        Some(receiver) => receiver.recv().await,
        None => return Err(EventAcceptError::ReceiverUnavailable),
    };
    let wake = data.is_none();
    let event = match data {
        Some(event) => RawEvent::Event(event),
        None => RawEvent::Done,
    };
    Ok((event, wake))
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use tokio::sync::mpsc;

    use super::*;

    fn metadata() -> EventMetadata {
        let mut otel = BTreeMap::new();
        otel.insert("k".to_owned(), "v".to_owned());
        EventMetadata {
            service_path: Some("hello".to_owned()),
            execution_id: Some("11111111-1111-4111-8111-111111111111".to_owned()),
            otel_attributes: Some(otel),
        }
    }

    fn run_node(script: &str) -> String {
        static CALLS: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "megabase-event-worker-{}-{nanos}-{}",
            std::process::id(),
            CALLS.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("run.mjs");
        std::fs::write(&path, script).unwrap();
        let output = Command::new("node")
            .arg(&path)
            .output()
            .unwrap_or_else(|error| {
                panic!("node failed to start ({error}); the console shim test needs node on PATH")
            });
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            output.status.success(),
            "node exited {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status.code()
        );
        stdout
    }

    #[test]
    fn levels_keep_the_low_eight_bits_and_unknown_bytes_are_debug() {
        assert_eq!(LogLevel::from_smi(0), LogLevel::Debug);
        assert_eq!(LogLevel::from_smi(1), LogLevel::Info);
        assert_eq!(LogLevel::from_smi(2), LogLevel::Warning);
        assert_eq!(LogLevel::from_smi(3), LogLevel::Error);
        assert_eq!(LogLevel::from_smi(4), LogLevel::Debug);
        assert_eq!(LogLevel::from_smi(256), LogLevel::Debug);
        assert_eq!(LogLevel::from_smi(257), LogLevel::Info);
        assert_eq!(LogLevel::from_smi(-1), LogLevel::Debug);
        assert_eq!(LogLevel::from_smi(i32::MIN), LogLevel::Debug);
        assert_eq!(LogLevel::from_smi(i32::MAX), LogLevel::Debug);
        assert_eq!(LogLevel::Info.as_str(), "Info");
        assert_eq!(
            fallback_line(LogLevel::Warning, "careful\n"),
            "[Warning] careful\n"
        );
    }

    #[test]
    fn without_a_channel_the_fallback_line_is_returned_and_metadata_is_unused() {
        let disposition = user_worker_log(None, &metadata(), "hello\n", 1).unwrap();
        assert_eq!(
            disposition,
            LogDisposition::Fallback("[Info] hello\n".to_owned())
        );
        let err = user_worker_log(None, &EventMetadata::default(), "x", 3).unwrap();
        assert_eq!(err, LogDisposition::Fallback("[Error] x".to_owned()));
    }

    #[test]
    fn a_channel_queues_a_log_event_and_a_dropped_receiver_is_an_error() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let meta = metadata();
        let disposition = user_worker_log(Some(&tx), &meta, "hello\n", 1).unwrap();
        assert_eq!(disposition, LogDisposition::Queued);
        let event = rx.try_recv().unwrap();
        assert_eq!(
            event.event,
            serde_json::json!({"Log": {"msg": "hello\n", "level": "Info"}})
        );
        assert_eq!(event.metadata, meta);
        drop(rx);
        let error = user_worker_log(Some(&tx), &meta, "later", 0).unwrap_err();
        assert_eq!(error, UserWorkerLogError::ChannelClosed);
        assert_eq!(error.to_string(), "channel closed");
    }

    #[test]
    fn raw_event_json_is_what_the_listener_switches_on() {
        let event = WorkerEventWithMetadata {
            event: serde_json::json!({"Log": {"msg": "m", "level": "Debug"}}),
            metadata: EventMetadata::default(),
        };
        assert_eq!(
            RawEvent::Event(event).to_json(),
            serde_json::json!({
                "Event": {
                    "event": {"Log": {"msg": "m", "level": "Debug"}},
                    "metadata": {
                        "service_path": null,
                        "execution_id": null,
                        "otel_attributes": null
                    }
                }
            })
        );
        assert_eq!(RawEvent::Done.to_json(), serde_json::json!("Done"));
    }

    #[tokio::test]
    async fn accept_waits_for_one_event_then_done_wakes_and_keeps_the_receiver() {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut slot = Some(rx);
        let (raw, wake) = {
            let pending = event_accept(&mut slot);
            tokio::pin!(pending);
            let early = tokio::time::timeout(Duration::from_millis(20), &mut pending).await;
            assert!(early.is_err(), "accept returned before the event was sent");
            let logged = user_worker_log(Some(&tx), &EventMetadata::default(), "hi", 2).unwrap();
            assert_eq!(logged, LogDisposition::Queued);
            pending.await.unwrap()
        };
        assert!(!wake);
        assert!(slot.is_some());
        match raw {
            RawEvent::Event(event) => {
                assert_eq!(event.event["Log"]["msg"], "hi");
                assert_eq!(event.event["Log"]["level"], "Warning");
            }
            RawEvent::Done => panic!("expected the queued log"),
        }

        drop(tx);
        let (raw, wake) = event_accept(&mut slot).await.unwrap();
        assert!(wake);
        assert_eq!(raw, RawEvent::Done);
        assert!(slot.is_some());
        let (again, wake) = event_accept(&mut slot).await.unwrap();
        assert!(wake);
        assert_eq!(again, RawEvent::Done);
    }

    #[tokio::test]
    async fn accept_with_no_receiver_is_the_upstream_bail_string() {
        let mut slot = None;
        let error = event_accept(&mut slot).await.unwrap_err();
        assert_eq!(error, EventAcceptError::ReceiverUnavailable);
        assert_eq!(error.to_string(), "events worker receiver not available");

        let (tx, rx) = mpsc::unbounded_channel::<WorkerEventWithMetadata>();
        let mut slot = Some(rx);
        let taken = slot.take();
        let busy = event_accept(&mut slot).await.unwrap_err();
        assert_eq!(busy, EventAcceptError::ReceiverUnavailable);
        slot = taken;
        drop(tx);
        let (raw, wake) = event_accept(&mut slot).await.unwrap();
        assert!(wake);
        assert_eq!(raw, RawEvent::Done);
    }

    #[tokio::test]
    async fn dropping_accept_during_the_wait_puts_the_receiver_back() {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut slot = Some(rx);
        {
            let pending = event_accept(&mut slot);
            tokio::pin!(pending);
            let early = tokio::time::timeout(Duration::from_millis(20), &mut pending).await;
            assert!(early.is_err(), "accept finished before it was dropped");
        }
        assert!(
            slot.is_some(),
            "receiver was lost when the wait was cancelled"
        );
        let queued = user_worker_log(Some(&tx), &EventMetadata::default(), "later", 1).unwrap();
        assert_eq!(queued, LogDisposition::Queued);
        let (raw, wake) = event_accept(&mut slot).await.unwrap();
        assert!(!wake);
        match raw {
            RawEvent::Event(event) => assert_eq!(event.event["Log"]["msg"], "later"),
            RawEvent::Done => panic!("channel closed"),
        }
    }

    #[test]
    fn the_console_shim_runs_and_matches_the_fallback_line() {
        let script = format!(
            r#"
            const chunks = [];
            const decoder = new TextDecoder();
            globalThis.Deno = {{
              inspect(value) {{
                if (value === null) return "null";
                if (value === undefined) return "undefined";
                if (typeof value === "string") return JSON.stringify(value);
                if (typeof value === "number" || typeof value === "boolean") return String(value);
                if (Array.isArray(value)) {{
                  return `[ ${{value.map((item) => Deno.inspect(item)).join(", ")}} ]`;
                }}
                if (typeof value === "object") {{
                  const body = Object.keys(value)
                    .map((key) => `${{key}}: ${{Deno.inspect(value[key])}}`)
                    .join(", ");
                  return `{{ ${{body}} }}`;
                }}
                return String(value);
              }},
              stderr: {{
                writeSync(data) {{
                  chunks.push(decoder.decode(data));
                  return data.byteLength;
                }},
              }},
            }};
            {SOURCE}
            console.debug("d");
            console.log("l");
            console.info("i");
            console.warn("w");
            console.error("e");
            console.log("a", "b");
            console.log("%s", "z");
            console.log({{ n: 1 }});
            process.stdout.write(chunks.join(""));
            "#
        );
        let stdout = run_node(&script);
        let expect = |level: LogLevel, msg: &str| format!("{}\n", fallback_line(level, msg));
        let mut wanted = String::new();
        wanted.push_str(&expect(LogLevel::Debug, "d\n"));
        wanted.push_str(&expect(LogLevel::Info, "l\n"));
        wanted.push_str(&expect(LogLevel::Info, "i\n"));
        wanted.push_str(&expect(LogLevel::Warning, "w\n"));
        wanted.push_str(&expect(LogLevel::Error, "e\n"));
        wanted.push_str(&expect(LogLevel::Info, "a b\n"));
        wanted.push_str(&expect(LogLevel::Info, "%s z\n"));
        wanted.push_str(&expect(LogLevel::Info, "{ n: 1 }\n"));
        assert_eq!(stdout, wanted);
    }

    #[test]
    fn the_listener_projection_runs_against_accept_json() {
        let event = WorkerEventWithMetadata {
            event: serde_json::json!({"Log": {"msg": "m\n", "level": "Error"}}),
            metadata: metadata(),
        };
        let cases = [RawEvent::Done.to_json(), RawEvent::Event(event).to_json()];
        let payload = serde_json::to_string(&cases).unwrap();
        let script = format!(
            r#"
            const cases = {payload};
            function project(reqEvt) {{
              const done = reqEvt === "Done";
              let value = null;
              if (!done) {{
                const rawEvent = reqEvt["Event"];
                const eventType = Object.keys(rawEvent.event)[0];
                value = {{
                  event_type: eventType,
                  event: rawEvent.event[eventType],
                  metadata: rawEvent.metadata,
                }};
              }}
              return {{ value, done }};
            }}
            process.stdout.write(JSON.stringify(cases.map(project)));
            "#
        );
        let stdout = run_node(&script);
        let projected: Vec<Value> = serde_json::from_str(&stdout).unwrap();
        assert_eq!(
            projected,
            vec![
                serde_json::json!({"value": null, "done": true}),
                serde_json::json!({
                    "value": {
                        "event_type": "Log",
                        "event": {"msg": "m\n", "level": "Error"},
                        "metadata": metadata().to_json()
                    },
                    "done": false
                }),
            ]
        );
    }

    #[test]
    fn the_worker_module_installs_the_console_shim_before_import() {
        let module = crate::bootstrap::module();
        let shim = module.find(SOURCE).unwrap();
        assert!(shim < module.find("await import(").unwrap());
    }
}
