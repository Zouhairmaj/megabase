//! Topic subscriptions and presence shared by every socket of the process.

// Ported from supabase/realtime lib/realtime_web/channels/realtime_channel/message_dispatcher.ex
// and lib/realtime_web/channels/presence.ex (Apache-2.0), pin v2.143.3. Presence
// state and diffs follow Phoenix.Presence and Phoenix.Tracker 1.7, which Realtime
// uses unchanged and which are not vendored.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::{Map, Value};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::Notify;

use crate::protocol::{Frame, Serializer};
use crate::term::phx_ref;

/// One socket connection.
pub(crate) type ConnId = u64;

/// Pushes a socket may have queued before it counts as a slow consumer.
pub(crate) const OUTBOX_CAPACITY: usize = 1024;

/// The bounded queue of pushes from other sockets to one socket. A full queue
/// evicts the socket from the topic and wakes its serve loop to close it, so
/// a client that stops reading cannot make the process buffer without limit.
#[derive(Clone, Debug)]
pub(crate) struct Outbox {
    tx: Sender<String>,
    evicted: Arc<Notify>,
}

impl Outbox {
    /// A queue holding up to `capacity` pushes, and its receiving end.
    pub(crate) fn new(capacity: usize) -> (Self, Receiver<String>) {
        let (tx, rx) = channel(capacity);
        let outbox = Self {
            tx,
            evicted: Arc::new(Notify::new()),
        };
        (outbox, rx)
    }

    /// Notified once the hub evicts this socket for not draining its queue.
    pub(crate) fn evicted(&self) -> Arc<Notify> {
        Arc::clone(&self.evicted)
    }
}

/// Subscribers and presences per joined topic (`realtime:<name>`).
#[derive(Debug, Default)]
pub(crate) struct Hub {
    next_conn: AtomicU64,
    topics: Mutex<HashMap<String, Topic>>,
}

#[derive(Debug, Default)]
struct Topic {
    members: Vec<Member>,
    presences: Vec<Presence>,
}

#[derive(Debug)]
struct Member {
    conn: ConnId,
    serializer: Serializer,
    outbox: Outbox,
}

/// One tracked presence: the channel of `conn` under `key`.
#[derive(Debug)]
struct Presence {
    conn: ConnId,
    key: String,
    meta: Map<String, Value>,
}

impl Topic {
    /// `MessageDispatcher.dispatch/3`: a fastlane push to every subscriber but
    /// `except`, encoded with each subscriber's serializer.
    fn dispatch(&mut self, name: &str, except: Option<ConnId>, event: &str, payload: &Value) {
        // One encoding per serializer: `[V1, V2]`, built on first use.
        let mut cache: [Option<Option<String>>; 2] = [None, None];
        let mut slow = Vec::new();
        for member in self.members.iter().filter(|m| Some(m.conn) != except) {
            let slot = match member.serializer {
                Serializer::V1 => 0,
                Serializer::V2 => 1,
            };
            let text = cache[slot].get_or_insert_with(|| {
                let frame = Frame::push(Value::Null, name, event, payload);
                member.serializer.fastlane(frame)
            });
            if let Some(text) = text {
                // A closed outbox is a socket that is going away.
                if let Err(TrySendError::Full(_)) = member.outbox.tx.try_send(text.clone()) {
                    slow.push(member.conn);
                }
            }
        }
        for conn in slow {
            self.evict(name, conn);
        }
    }

    /// Drops a socket that stopped draining its outbox: it leaves the topic,
    /// its presence is untracked, and its serve loop is told to close.
    fn evict(&mut self, name: &str, conn: ConnId) {
        if let Some(index) = self.members.iter().position(|m| m.conn == conn) {
            self.members.remove(index).outbox.evicted.notify_one();
        }
        self.untrack(name, conn);
    }

    /// Removes the presence of `conn` and pushes its `leaves` diff.
    fn untrack(&mut self, name: &str, conn: ConnId) {
        let Some(index) = self.presences.iter().position(|p| p.conn == conn) else {
            return;
        };
        let left = self.presences.remove(index);
        let diff = diff(&[], &[&left]);
        self.dispatch(name, None, "presence_diff", &diff);
    }
}

/// `Phoenix.Presence.group/1`: `{key: {"metas": [meta, …]}}`.
fn group(presences: &[&Presence]) -> Value {
    let mut grouped = Map::new();
    for presence in presences {
        let entry = grouped.entry(presence.key.as_str()).or_insert_with(|| {
            Value::Object(Map::from_iter([(
                "metas".to_owned(),
                Value::Array(Vec::new()),
            )]))
        });
        if let Some(Value::Array(metas)) = entry.get_mut("metas") {
            metas.push(Value::Object(presence.meta.clone()));
        }
    }
    Value::Object(grouped)
}

/// The `presence_diff` payload: `{"joins": …, "leaves": …}`, both grouped.
// megabase:unit realtime:server-event:presence_diff
fn diff(joins: &[&Presence], leaves: &[&Presence]) -> Value {
    Value::Object(Map::from_iter([
        ("joins".to_owned(), group(joins)),
        ("leaves".to_owned(), group(leaves)),
    ]))
}

impl Hub {
    fn topics(&self) -> MutexGuard<'_, HashMap<String, Topic>> {
        self.topics.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A fresh id for a new socket.
    pub(crate) fn connect(&self) -> ConnId {
        self.next_conn.fetch_add(1, Ordering::Relaxed)
    }

    /// `Endpoint.subscribe/2` for the channel of `conn` on `topic`.
    pub(crate) fn subscribe(
        &self,
        topic: &str,
        conn: ConnId,
        serializer: Serializer,
        outbox: Outbox,
    ) {
        let mut topics = self.topics();
        let entry = topics.entry(topic.to_owned()).or_default();
        entry.members.retain(|m| m.conn != conn);
        entry.members.push(Member {
            conn,
            serializer,
            outbox,
        });
    }

    /// The channel of `conn` on `topic` stopped: unsubscribe it and untrack
    /// its presence, which pushes a `leaves` diff to the remaining members.
    pub(crate) fn unsubscribe(&self, topic: &str, conn: ConnId) {
        let mut topics = self.topics();
        let Some(entry) = topics.get_mut(topic) else {
            return;
        };
        entry.members.retain(|m| m.conn != conn);
        entry.untrack(topic, conn);
        if entry.members.is_empty() && entry.presences.is_empty() {
            topics.remove(topic);
        }
    }

    /// `pubsub_broadcast_from/5`: a `broadcast` push to every subscriber of
    /// `topic` except `from`.
    pub(crate) fn broadcast(&self, topic: &str, from: ConnId, payload: &Value) {
        if let Some(entry) = self.topics().get_mut(topic) {
            entry.dispatch(topic, Some(from), "broadcast", payload);
        }
    }

    /// `Presence.track/4`, or `Presence.update/4` when the channel already
    /// tracks a presence; the diff goes to every subscriber, `conn` included.
    pub(crate) fn track(&self, topic: &str, conn: ConnId, key: &str, payload: &Map<String, Value>) {
        let mut topics = self.topics();
        let Some(entry) = topics.get_mut(topic) else {
            return;
        };
        let mut meta = payload.clone();
        meta.insert("phx_ref".to_owned(), Value::String(phx_ref()));
        let diff = match entry.presences.iter().position(|p| p.conn == conn) {
            Some(index) => {
                let previous = &entry.presences[index];
                if let Some(prev_ref) = previous.meta.get("phx_ref") {
                    meta.insert("phx_ref_prev".to_owned(), prev_ref.clone());
                }
                let current = Presence {
                    conn,
                    key: key.to_owned(),
                    meta,
                };
                let diff = diff(&[&current], &[previous]);
                entry.presences[index] = current;
                diff
            }
            None => {
                let current = Presence {
                    conn,
                    key: key.to_owned(),
                    meta,
                };
                let diff = diff(&[&current], &[]);
                entry.presences.push(current);
                diff
            }
        };
        entry.dispatch(topic, None, "presence_diff", &diff);
    }

    /// `Presence.untrack/3` for the channel of `conn`.
    pub(crate) fn untrack(&self, topic: &str, conn: ConnId) {
        if let Some(entry) = self.topics().get_mut(topic) {
            entry.untrack(topic, conn);
        }
    }

    /// `presence_dirty_list/1`: the grouped presences of `topic`.
    pub(crate) fn presence_state(&self, topic: &str) -> Value {
        let topics = self.topics();
        let presences: Vec<&Presence> = topics
            .get(topic)
            .map(|entry| entry.presences.iter().collect())
            .unwrap_or_default();
        group(&presences)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn member(hub: &Hub, serializer: Serializer) -> (ConnId, Receiver<String>) {
        let conn = hub.connect();
        let (tx, rx) = Outbox::new(OUTBOX_CAPACITY);
        hub.subscribe("realtime:room", conn, serializer, tx);
        (conn, rx)
    }

    #[tokio::test]
    async fn a_full_outbox_evicts_the_socket_and_untracks_it() {
        let hub = Hub::default();
        let slow = hub.connect();
        let (slow_outbox, _slow_rx) = Outbox::new(1);
        let evicted = slow_outbox.evicted();
        hub.subscribe("realtime:room", slow, Serializer::V2, slow_outbox);
        let (sender, mut rx) = member(&hub, Serializer::V2);
        let payload = json!({"user": 1});
        hub.track("realtime:room", slow, "k", payload.as_object().unwrap());
        // The track diff fills the slow queue; the next broadcast overflows it.
        hub.broadcast("realtime:room", sender, &json!({"n": 1}));
        evicted.notified().await;
        assert_eq!(hub.presence_state("realtime:room"), json!({}));
        // The survivor saw the join and then the leave of the slow socket.
        let diffs = drain(&mut rx);
        assert_eq!(diffs.len(), 2);
        assert_eq!(diffs[1][4]["leaves"]["k"]["metas"][0]["user"], 1);
    }

    fn drain(rx: &mut Receiver<String>) -> Vec<Value> {
        let mut out = Vec::new();
        while let Ok(text) = rx.try_recv() {
            out.push(serde_json::from_str(&text).unwrap());
        }
        out
    }

    #[test]
    fn broadcast_skips_the_sender_and_non_map_payloads_on_v2() {
        let hub = Hub::default();
        let (a, mut a_rx) = member(&hub, Serializer::V2);
        let (_, mut b_rx) = member(&hub, Serializer::V2);
        let (_, mut c_rx) = member(&hub, Serializer::V1);
        hub.broadcast("realtime:room", a, &json!({"event": "e"}));
        hub.broadcast("realtime:room", a, &json!("text"));
        assert!(drain(&mut a_rx).is_empty());
        assert_eq!(
            drain(&mut b_rx),
            [json!([null, null, "realtime:room", "broadcast", {"event": "e"}])]
        );
        assert_eq!(
            drain(&mut c_rx),
            [
                json!({"topic": "realtime:room", "event": "broadcast", "payload": {"event": "e"}, "ref": null}),
                json!({"topic": "realtime:room", "event": "broadcast", "payload": "text", "ref": null}),
            ]
        );
    }

    #[test]
    fn track_update_and_leave_diffs() {
        let hub = Hub::default();
        let (a, mut a_rx) = member(&hub, Serializer::V2);
        let (_, mut b_rx) = member(&hub, Serializer::V2);
        let payload = json!({"user": 1});
        hub.track("realtime:room", a, "k", payload.as_object().unwrap());
        let joined = drain(&mut a_rx);
        assert_eq!(joined, drain(&mut b_rx));
        let ref1 = joined[0][4]["joins"]["k"]["metas"][0]["phx_ref"].clone();
        assert!(ref1.is_string());
        assert_eq!(joined[0][4]["leaves"], json!({}));
        assert_eq!(
            hub.presence_state("realtime:room")["k"]["metas"][0]["user"],
            1
        );

        let payload = json!({"user": 2});
        hub.track("realtime:room", a, "k", payload.as_object().unwrap());
        let updated = &drain(&mut b_rx)[0][4];
        assert_eq!(updated["joins"]["k"]["metas"][0]["phx_ref_prev"], ref1);
        assert_eq!(updated["leaves"]["k"]["metas"][0]["phx_ref"], ref1);
        assert_eq!(updated["leaves"]["k"]["metas"][0]["user"], 1);

        drain(&mut a_rx);
        hub.unsubscribe("realtime:room", a);
        let left = &drain(&mut b_rx)[0][4];
        assert_eq!(left["joins"], json!({}));
        assert_eq!(left["leaves"]["k"]["metas"][0]["user"], 2);
        assert!(drain(&mut a_rx).is_empty());
        assert_eq!(hub.presence_state("realtime:room"), json!({}));
    }
}
