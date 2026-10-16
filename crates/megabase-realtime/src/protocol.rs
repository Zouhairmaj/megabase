//! Phoenix channel protocol over the Realtime socket: text and V2 binary
//! frames, heartbeat, `phx_join` on `realtime:*` topics, public broadcast
//! (user broadcasts included), presence, `access_token` refreshes, token
//! expiry and `phx_leave`.

// Ported from supabase/realtime lib/realtime_web/channels/realtime_channel.ex,
// lib/realtime_web/channels/realtime_channel/broadcast_handler.ex,
// lib/realtime_web/channels/realtime_channel/presence_handler.ex,
// lib/realtime_web/socket/v2_serializer.ex, lib/realtime_web/socket/user_broadcast.ex
// and lib/realtime_web/socket.ex (Apache-2.0), pin v2.143.3. Socket-level handling
// follows Phoenix 1.7 `Phoenix.Socket`, which Realtime uses unchanged. Event names
// and the binary frame layout from supabase/supabase-js
// packages/core/realtime-js/src/lib/constants.ts and src/lib/serializer.ts (MIT),
// pin v2.117.3.

use std::collections::HashMap;
use std::sync::Arc;

use megabase_core::{Hs256, MegabaseNotImplemented};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::cdc::{params_id, Cdc, Pending, Subscriber};
use crate::hub::{ConnId, Hub, Outbox};
use crate::term::{binary_tuple_size, external_size, user_broadcast_size, uuid1, MAX_PAYLOAD_SIZE};
use crate::token::{authorize_conn, TokenError};
use crate::COMPONENT;

/// `CLIENT_PRESENCE_MAX_CALLS` default: presence events per window and channel.
const CLIENT_PRESENCE_MAX_CALLS: u32 = 5;
/// `CLIENT_PRESENCE_WINDOW_MS` default.
const CLIENT_PRESENCE_WINDOW_MS: i64 = 30_000;
/// `ACCESS_TOKEN_THROTTLE_MS` default: one applied refresh per window.
const ACCESS_TOKEN_THROTTLE_MS: i64 = 10_000;
/// `@confirm_token_ms_max_interval`: the longest re-confirmation delay.
const CONFIRM_TOKEN_MAX_INTERVAL_MS: i64 = 3_600_000;

/// Negotiated `vsn` serializer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Serializer {
    /// `Phoenix.Socket.V1.JSONSerializer` (`~> 1.0.0`): JSON objects.
    V1,
    /// `RealtimeWeb.Socket.V2Serializer` (`~> 2.0.0`): JSON arrays.
    V2,
}

impl Serializer {
    /// `negotiate_serializer/2`: `None` when no serializer matches `vsn`.
    pub(crate) fn negotiate(vsn: Option<&str>) -> Option<Self> {
        let vsn = vsn.unwrap_or("1.0.0");
        let parts: Vec<&str> = vsn.split('.').collect();
        let [major, minor, patch] = parts[..] else {
            return None;
        };
        let number = |part: &str| -> Option<u64> {
            let valid = !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_digit())
                && (part == "0" || !part.starts_with('0'));
            valid.then(|| part.parse().ok()).flatten()
        };
        match (number(major)?, number(minor)?, number(patch)?) {
            (1, 0, _) => Some(Self::V1),
            (2, 0, _) => Some(Self::V2),
            _ => None,
        }
    }
}

/// One decoded client or server message.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Frame {
    pub join_ref: Value,
    pub reference: Value,
    pub topic: String,
    pub event: String,
    pub payload: Value,
}

impl Frame {
    /// `Phoenix.Socket` `reply/2`: `{"status", "response"}` under the
    /// request's `ref`.
    // megabase:unit realtime:protocol-event:phx_reply
    fn reply(
        join_ref: Value,
        reference: Value,
        topic: &str,
        status: &str,
        response: Value,
    ) -> Self {
        Self {
            join_ref,
            reference,
            topic: topic.to_owned(),
            event: "phx_reply".to_owned(),
            payload: Value::Object(Map::from_iter([
                ("status".to_owned(), Value::from(status)),
                ("response".to_owned(), response),
            ])),
        }
    }

    /// A server push (`Phoenix.Channel.push/3`, or a fastlane broadcast when
    /// `join_ref` is null): `ref` is null.
    pub(crate) fn push(join_ref: Value, topic: &str, event: &str, payload: &Value) -> Self {
        Self {
            join_ref,
            reference: Value::Null,
            topic: topic.to_owned(),
            event: event.to_owned(),
            payload: payload.clone(),
        }
    }

    /// `Phoenix.Socket` `encode_close/3`: `ref` is the join ref.
    // megabase:unit realtime:protocol-event:phx_close
    fn close(join_ref: &Value, topic: &str) -> Self {
        Self::exit(join_ref, topic, "phx_close")
    }

    /// `Phoenix.Socket` `encode_on_exit/4` for a crashed channel.
    // megabase:unit realtime:protocol-event:phx_error
    fn error(join_ref: &Value, topic: &str) -> Self {
        Self::exit(join_ref, topic, "phx_error")
    }

    fn exit(join_ref: &Value, topic: &str, event: &str) -> Self {
        Self {
            join_ref: join_ref.clone(),
            reference: join_ref.clone(),
            topic: topic.to_owned(),
            event: event.to_owned(),
            payload: json!({}),
        }
    }
}

/// One encoded server message, as the socket sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Wire {
    Text(String),
    /// A `V2Serializer` binary frame.
    Binary(Vec<u8>),
}

/// `V2Serializer` binary frame kinds.
const KIND_PUSH: u8 = 0;
const KIND_BROADCAST: u8 = 2;
const KIND_USER_BROADCAST_PUSH: u8 = 3;
const KIND_USER_BROADCAST: u8 = 4;

/// What a `broadcast` carries to the topic's subscribers.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Broadcast {
    /// `%Phoenix.Socket.Broadcast{}` with a decoded JSON payload.
    Json(Value),
    /// `%Phoenix.Socket.Broadcast{payload: {:binary, data}}` from a `@push`
    /// frame.
    Binary(Vec<u8>),
    /// `%UserBroadcast{}` from a `@user_broadcast_push` frame. `build_broadcast/2`
    /// drops the client metadata, so none is relayed.
    User {
        event: String,
        /// `user_payload_encoding`: `:json` (1) or `:binary` (0).
        json: bool,
        payload: Vec<u8>,
    },
}

impl Broadcast {
    /// The `broadcast` push a subscriber of `topic` using `serializer`
    /// receives (`MessageDispatcher` `fastlane!/2`); `None` is a message that
    /// serializer cannot encode, which the dispatcher skips.
    // megabase:unit realtime:server-event:broadcast
    pub(crate) fn fastlane(&self, serializer: Serializer, topic: &str) -> Option<Wire> {
        match (self, serializer) {
            (Self::Json(payload), _) => serializer
                .fastlane(Frame::push(Value::Null, topic, "broadcast", payload))
                .map(Wire::Text),
            // The V1 serializer raises on a non-map payload.
            (Self::Binary(_), Serializer::V1) => None,
            (Self::Binary(data), Serializer::V2) => {
                let mut bytes = vec![KIND_BROADCAST, byte_size(topic)?, byte_size("broadcast")?];
                bytes.extend_from_slice(topic.as_bytes());
                bytes.extend_from_slice(b"broadcast");
                bytes.extend_from_slice(data);
                Some(Wire::Binary(bytes))
            }
            // `UserBroadcast.convert_to_json_broadcast/1`: only a JSON payload
            // that decodes, emitted verbatim.
            (
                Self::User {
                    event,
                    json,
                    payload,
                },
                Serializer::V1,
            ) => {
                let raw = std::str::from_utf8(payload).ok().filter(|_| *json)?;
                serde_json::from_str::<Value>(raw).ok()?;
                Some(Wire::Text(format!(
                    r#"{{"event":"broadcast","payload":{{"event":{},"payload":{raw},"type":"broadcast"}},"ref":null,"topic":{}}}"#,
                    Value::from(event.as_str()),
                    Value::from(topic),
                )))
            }
            (
                Self::User {
                    event,
                    json,
                    payload,
                },
                Serializer::V2,
            ) => {
                let mut bytes = vec![
                    KIND_USER_BROADCAST,
                    byte_size(topic)?,
                    byte_size(event)?,
                    0,
                    u8::from(*json),
                ];
                bytes.extend_from_slice(topic.as_bytes());
                bytes.extend_from_slice(event.as_bytes());
                bytes.extend_from_slice(payload);
                Some(Wire::Binary(bytes))
            }
        }
    }
}

/// `byte_size!/3`: a binary frame field is at most 255 bytes.
fn byte_size(field: &str) -> Option<u8> {
    u8::try_from(field.len()).ok()
}

/// A decoded `V2Serializer` binary frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BinaryFrame {
    pub join_ref: Value,
    pub reference: Value,
    pub topic: String,
    pub event: String,
    /// The payload, as a broadcast would relay it.
    pub body: Broadcast,
    /// `:erlang.external_size/1` of the decoded payload term.
    pub size: usize,
}

/// Splits `len` bytes off the front of `bytes`.
fn take<'b>(bytes: &mut &'b [u8], len: u8) -> Option<&'b [u8]> {
    let (head, tail) = bytes.split_at_checked(usize::from(len))?;
    *bytes = tail;
    Some(head)
}

/// A binary field as a string; Megabase treats a field that is not UTF-8
/// as a malformed frame.
fn utf8(bytes: &[u8]) -> Option<String> {
    String::from_utf8(bytes.to_vec()).ok()
}

impl Serializer {
    /// `V2Serializer.decode!/2` for a binary frame; `None` is a frame no
    /// clause matches, handled like a malformed text frame.
    pub(crate) fn decode_binary(bytes: &[u8]) -> Option<BinaryFrame> {
        let (&kind, mut rest) = bytes.split_first()?;
        match kind {
            KIND_PUSH => {
                let [join_ref, reference, topic, event] = *take(&mut rest, 4)? else {
                    return None;
                };
                let join_ref = utf8(take(&mut rest, join_ref)?)?;
                let reference = utf8(take(&mut rest, reference)?)?;
                let topic = utf8(take(&mut rest, topic)?)?;
                let event = utf8(take(&mut rest, event)?)?;
                Some(BinaryFrame {
                    join_ref: Value::String(join_ref),
                    reference: Value::String(reference),
                    topic,
                    event,
                    size: binary_tuple_size(rest.len()),
                    body: Broadcast::Binary(rest.to_vec()),
                })
            }
            KIND_USER_BROADCAST_PUSH => {
                let [join_ref, reference, topic, event, metadata, encoding] = *take(&mut rest, 6)?
                else {
                    return None;
                };
                let join_ref = utf8(take(&mut rest, join_ref)?)?;
                let reference = utf8(take(&mut rest, reference)?)?;
                let topic = utf8(take(&mut rest, topic)?)?;
                let event = utf8(take(&mut rest, event)?)?;
                let metadata = take(&mut rest, metadata)?;
                // `Jason.decode!/1` raises on bad metadata.
                let metadata = if metadata.is_empty() {
                    Value::Object(Map::new())
                } else {
                    serde_json::from_slice(metadata).ok()?
                };
                let json = encoding != 0;
                Some(BinaryFrame {
                    join_ref: Value::String(join_ref),
                    reference: Value::String(reference),
                    topic,
                    event: "broadcast".to_owned(),
                    size: user_broadcast_size(&event, json, rest.len(), &metadata),
                    body: Broadcast::User {
                        event,
                        json,
                        payload: rest.to_vec(),
                    },
                })
            }
            _ => None,
        }
    }

    /// Decodes a text frame; `None` is a malformed message, which upstream
    /// logs and ignores.
    pub(crate) fn decode(self, text: &str) -> Option<Frame> {
        let value: Value = serde_json::from_str(text).ok()?;
        let (join_ref, reference, topic, event, payload) = match (self, value) {
            (Self::V2, Value::Array(items)) if items.len() >= 5 => {
                let mut items = items.into_iter();
                (
                    items.next()?,
                    items.next()?,
                    items.next()?,
                    items.next()?,
                    items.next()?,
                )
            }
            (Self::V1, Value::Object(mut map)) => {
                if !["topic", "event", "payload", "ref"]
                    .iter()
                    .all(|key| map.contains_key(*key))
                {
                    return None;
                }
                (
                    map.remove("join_ref").unwrap_or(Value::Null),
                    map.remove("ref")?,
                    map.remove("topic")?,
                    map.remove("event")?,
                    map.remove("payload")?,
                )
            }
            _ => return None,
        };
        let (Value::String(topic), Value::String(event)) = (topic, event) else {
            return None;
        };
        Some(Frame {
            join_ref,
            reference,
            topic,
            event,
            payload,
        })
    }

    pub(crate) fn encode(self, frame: Frame) -> String {
        let Frame {
            join_ref,
            reference,
            topic,
            event,
            payload,
        } = frame;
        let value = match self {
            Self::V2 => Value::Array(vec![
                join_ref,
                reference,
                Value::String(topic),
                Value::String(event),
                payload,
            ]),
            // The V1 serializer drops `join_ref`.
            Self::V1 => Value::Object(Map::from_iter([
                ("topic".to_owned(), Value::String(topic)),
                ("event".to_owned(), Value::String(event)),
                ("payload".to_owned(), payload),
                ("ref".to_owned(), reference),
            ])),
        };
        value.to_string()
    }

    /// `fastlane!/1` for a broadcast: the V2 serializer refuses a payload
    /// that is not a map, and the dispatcher skips the message.
    pub(crate) fn fastlane(self, frame: Frame) -> Option<String> {
        (self == Self::V1 || frame.payload.is_object()).then(|| self.encode(frame))
    }
}

/// What a successful join assigns to the channel.
#[derive(Debug, PartialEq)]
struct JoinConfig {
    ack_broadcast: bool,
    self_broadcast: bool,
    presence_enabled: bool,
    presence_key: String,
    /// `access_token`: the token the channel was confirmed with.
    access_token: String,
    /// `confirm_token_ref`: when `:confirm_token` fires (Unix ms).
    confirm_token_at: i64,
    /// `new_api?/1`: the join payload has a `config`.
    is_new_api: bool,
    /// `pg_change_params/5`: the `postgres_changes` entries.
    postgres_changes: Vec<Value>,
}

/// Why `confirm_token/1` failed.
#[derive(Debug, PartialEq)]
enum Unconfirmed {
    /// `authorize_conn/3` rejected the token.
    Token(TokenError),
    /// The rounded `exp` is not after now: `exp - now` (`None` when `exp` is
    /// not an integer Megabase can represent).
    ExpDiff(Option<i64>),
}

/// One joined channel (the channel process's assigns).
struct Channel {
    join_ref: Value,
    config: JoinConfig,
    /// `access_token_verified_at`: when a refresh was last applied.
    access_token_verified_at: Option<i64>,
    /// `pending_access_token`: a throttled refresh waiting to be applied.
    pending_access_token: Option<String>,
    /// `:apply_pending_access_token` timers still to fire (Unix ms).
    apply_pending_at: Vec<i64>,
    /// `presence_track_payload`: the last tracked payload.
    presence_payload: Option<Value>,
    /// `presence_client_rate_limit`: calls in the window ending at `reset_at`.
    presence_calls: u32,
    presence_reset_at: Option<i64>,
}

impl Channel {
    /// `limit_client_presence_event/1` at monotonic time `now_ms`.
    fn allow_presence_event(&mut self, now_ms: i64) -> bool {
        match self.presence_reset_at {
            Some(reset_at) if now_ms <= reset_at => {
                if self.presence_calls >= CLIENT_PRESENCE_MAX_CALLS {
                    return false;
                }
                self.presence_calls += 1;
            }
            _ => {
                self.presence_calls = 1;
                self.presence_reset_at = Some(now_ms + CLIENT_PRESENCE_WINDOW_MS);
            }
        }
        true
    }
}

/// `Phoenix.Socket` `handle_in/4`: where a message for a topic goes.
enum Route {
    /// The joined channel, with its join ref.
    Channel(Value),
    /// The client raced a server close: dropped without a reply.
    Dropped,
    /// No channel and not a join.
    Unmatched,
    /// A join, after the `phx_close` of a channel it replaces.
    Join(Option<Frame>),
}

/// The `unmatched topic` error reply.
fn unmatched(join_ref: Value, reference: Value, topic: &str) -> Frame {
    Frame::reply(
        join_ref,
        reference,
        topic,
        "error",
        json!({ "reason": "unmatched topic" }),
    )
}

/// Per-connection channel state.
pub(crate) struct Session<'a> {
    jwt: &'a Hs256,
    hub: &'a Hub,
    conn: ConnId,
    serializer: Serializer,
    /// Where other sockets' pushes to this socket land.
    outbox: Outbox,
    /// The socket's own token (`tenant_token`).
    socket_token: String,
    /// Joined topic -> its channel.
    channels: HashMap<String, Channel>,
    /// A channel sent the transport `%Broadcast{event: "disconnect"}`.
    disconnect: bool,
    /// The tenant's `postgres_cdc_rls` extension; `None` without a database.
    cdc: Option<Arc<Cdc>>,
}

impl Drop for Session<'_> {
    /// The transport went away: every channel stops, which unsubscribes it
    /// and untracks its presence.
    fn drop(&mut self) {
        let topics: Vec<String> = self.channels.keys().cloned().collect();
        for topic in topics {
            self.stop(&topic);
        }
    }
}

impl<'a> Session<'a> {
    pub(crate) fn new(
        jwt: &'a Hs256,
        hub: &'a Hub,
        serializer: Serializer,
        socket_token: String,
        outbox: Outbox,
    ) -> Self {
        Self {
            jwt,
            hub,
            conn: hub.connect(),
            serializer,
            outbox,
            socket_token,
            channels: HashMap::new(),
            disconnect: false,
            cdc: None,
        }
    }

    /// The session with the tenant's Postgres changes extension.
    #[must_use]
    pub(crate) fn with_cdc(mut self, cdc: Option<Arc<Cdc>>) -> Self {
        self.cdc = cdc;
        self
    }

    /// Whether the socket must close once the replies so far are sent:
    /// `Phoenix.Socket` stops the transport on a `disconnect` broadcast.
    pub(crate) fn disconnected(&self) -> bool {
        self.disconnect
    }

    /// Frames to send back for one client frame, at Unix time `now_ms` in
    /// milliseconds. Pushes from other channels arrive on the outbox.
    pub(crate) fn handle(&mut self, frame: Frame, now_ms: i64) -> Vec<Wire> {
        if frame.topic == "phoenix" && frame.event == "heartbeat" {
            return self.wire(vec![Frame::reply(
                Value::Null,
                frame.reference,
                "phoenix",
                "ok",
                json!({}),
            )]);
        }
        match self.route(&frame.topic, &frame.event, &frame.join_ref) {
            Route::Channel(join_ref) => self.channel_event(join_ref, frame, now_ms),
            Route::Dropped => Vec::new(),
            Route::Unmatched => self.wire(vec![unmatched(
                frame.join_ref,
                frame.reference,
                &frame.topic,
            )]),
            Route::Join(closed) => {
                let mut out = Vec::from_iter(closed);
                out.extend(self.phx_join(frame, now_ms));
                self.wire(out)
            }
        }
    }

    /// Frames to send back for one binary frame from a V2 socket.
    pub(crate) fn handle_binary(&mut self, frame: BinaryFrame) -> Vec<Wire> {
        let BinaryFrame {
            join_ref,
            reference,
            topic,
            event,
            body,
            size,
        } = frame;
        match self.route(&topic, &event, &join_ref) {
            Route::Channel(join_ref) => match event.as_str() {
                "phx_leave" => {
                    let out = self.phx_leave(&join_ref, &topic, reference);
                    self.wire(out)
                }
                "broadcast" => self.broadcast(join_ref, &topic, reference, &body, size),
                // `PresenceHandler.handle(_, socket)`: no `event` key.
                "presence" => self.wire(vec![Frame::reply(
                    join_ref,
                    reference,
                    &topic,
                    "ok",
                    json!({}),
                )]),
                _ => Vec::new(),
            },
            Route::Dropped => Vec::new(),
            Route::Unmatched => self.wire(vec![unmatched(join_ref, reference, &topic)]),
            // `RealtimeChannel.join/3` reads a map; Megabase does not model
            // the crash a `{:binary, data}` join payload causes.
            Route::Join(closed) => {
                let mut out = Vec::from_iter(closed);
                out.push(Frame::reply(
                    join_ref,
                    reference,
                    &topic,
                    "error",
                    not_implemented("realtime:protocol-event:phx_join"),
                ));
                self.wire(out)
            }
        }
    }

    /// Encodes frames with this socket's serializer.
    fn wire(&self, frames: Vec<Frame>) -> Vec<Wire> {
        frames
            .into_iter()
            .map(|frame| Wire::Text(self.serializer.encode(frame)))
            .collect()
    }

    /// `Phoenix.Socket` `handle_in/4`: where a message for `topic` goes.
    fn route(&mut self, topic: &str, event: &str, join_ref: &Value) -> Route {
        if let Some(current) = self.channels.get(topic) {
            if event == "phx_join" {
                // Duplicate join: Phoenix shuts the existing channel down first.
                let close = Frame::close(&current.join_ref, topic);
                self.stop(topic);
                return Route::Join(Some(close));
            }
            if !current.join_ref.is_null() && current.join_ref != *join_ref {
                // The client raced a server close; Phoenix drops the message.
                return Route::Dropped;
            }
            return Route::Channel(current.join_ref.clone());
        }
        if event == "phx_join" && topic.starts_with("realtime:") {
            Route::Join(None)
        } else {
            Route::Unmatched
        }
    }

    /// `phx_join` on a topic with no running channel.
    // megabase:unit realtime:protocol-event:phx_join
    fn phx_join(&mut self, frame: Frame, now_ms: i64) -> Vec<Frame> {
        let mut out = Vec::new();
        if let Some(rejected) = self.limit_joins(&frame, now_ms) {
            out.push(rejected);
            return out;
        }
        match self.join(&frame.topic, &frame.payload, now_ms) {
            Ok(mut config) => {
                let presence_enabled = config.presence_enabled;
                self.hub.subscribe(
                    &frame.topic,
                    self.conn,
                    self.serializer,
                    self.outbox.clone(),
                );
                let postgres_changes = self.postgres_cdc_subscribe(&frame, &mut config, now_ms);
                self.channels.insert(
                    frame.topic.clone(),
                    Channel {
                        join_ref: frame.join_ref.clone(),
                        config,
                        access_token_verified_at: None,
                        pending_access_token: None,
                        apply_pending_at: Vec::new(),
                        presence_payload: None,
                        presence_calls: 0,
                        presence_reset_at: None,
                    },
                );
                out.push(Frame::reply(
                    frame.join_ref.clone(),
                    frame.reference,
                    &frame.topic,
                    "ok",
                    json!({ "postgres_changes": postgres_changes }),
                ));
                // `send(self(), :sync_presence)` right after the join.
                if presence_enabled {
                    out.push(self.presence_state(frame.join_ref, &frame.topic));
                }
            }
            Err(response) => out.push(Frame::reply(
                frame.join_ref,
                frame.reference,
                &frame.topic,
                "error",
                response,
            )),
        }
        out
    }

    /// `postgres_cdc_subscribe/2` and `start_postgres_subscribe/4`: the
    /// channel's fastlane subscription to the tenant's changes, then
    /// `:postgres_subscribe` in the background. Returns the reply's
    /// `postgres_changes` (`add_id_to_postgres_changes/1`).
    fn postgres_cdc_subscribe(
        &self,
        frame: &Frame,
        config: &mut JoinConfig,
        now_ms: i64,
    ) -> Vec<Value> {
        let params = std::mem::take(&mut config.postgres_changes);
        let Some(cdc) = self.cdc.as_ref().filter(|_| !params.is_empty()) else {
            return Vec::new();
        };
        let claims = authorize_conn(self.jwt, &config.access_token, now_ms.div_euclid(1000))
            .map(Value::Object)
            .unwrap_or_default();
        let ids: Vec<(Uuid, u32)> = params
            .iter()
            .map(|params| (Uuid::new_v4(), params_id(params)))
            .collect();
        cdc.subscribe(Subscriber {
            conn: self.conn,
            topic: frame.topic.clone(),
            serializer: self.serializer,
            outbox: self.outbox.clone(),
            is_new_api: config.is_new_api,
            ids: ids.clone(),
        });
        let reply = params
            .iter()
            .zip(&ids)
            .map(|(params, (_, id))| {
                let mut entry = params.as_object().cloned().unwrap_or_default();
                entry.insert("id".to_owned(), Value::from(*id));
                Value::Object(entry)
            })
            .collect();
        let pending = Pending {
            conn: self.conn,
            topic: frame.topic.clone(),
            join_ref: frame.join_ref.clone(),
            serializer: self.serializer,
            outbox: self.outbox.clone(),
            claims,
            ids,
            params,
        };
        tokio::spawn(Arc::clone(cdc).postgres_subscribe(pending));
        reply
    }

    /// `limit_joins/2`, the first join step that can fail on the
    /// self-hosted tenant: counts the join, or rejects it while the
    /// joins-per-second limit is triggered and disconnects the socket after
    /// the error reply. `join("realtime:", …)` rejects an empty topic before.
    // megabase:unit realtime:server-event:disconnect
    fn limit_joins(&mut self, frame: &Frame, now_ms: i64) -> Option<Frame> {
        if frame.topic == "realtime:" || self.hub.count_join(now_ms) {
            return None;
        }
        // `send(transport_pid, %Phoenix.Socket.Broadcast{event: "disconnect"})`
        // reaches the transport after it sends the join reply.
        self.disconnect = true;
        Some(Frame::reply(
            frame.join_ref.clone(),
            frame.reference.clone(),
            &frame.topic,
            "error",
            reason("ClientJoinRateLimitReached: Too many joins per second"),
        ))
    }

    /// The channel on `topic` stopped: forget it, unsubscribe it and untrack
    /// its presence.
    fn stop(&mut self, topic: &str) {
        self.channels.remove(topic);
        self.hub.unsubscribe(topic, self.conn);
        if let Some(cdc) = &self.cdc {
            cdc.forget(cdc.unsubscribe(self.conn, topic));
        }
    }

    /// `presence_state` push with the topic's grouped presences.
    // megabase:unit realtime:server-event:presence_state
    fn presence_state(&self, join_ref: Value, topic: &str) -> Frame {
        Frame::push(
            join_ref,
            topic,
            "presence_state",
            &self.hub.presence_state(topic),
        )
    }

    /// An event on the joined channel `frame.topic`.
    fn channel_event(&mut self, join_ref: Value, frame: Frame, now_ms: i64) -> Vec<Wire> {
        let out = match frame.event.as_str() {
            "phx_leave" => self.phx_leave(&join_ref, &frame.topic, frame.reference),
            "broadcast" => {
                let size = external_size(&frame.payload);
                let body = Broadcast::Json(frame.payload);
                return self.broadcast(join_ref, &frame.topic, frame.reference, &body, size);
            }
            "presence" => self.presence(&join_ref, &frame, now_ms),
            "access_token" => self.access_token(frame, now_ms),
            // `handle_in/3` catch-all: logged, no reply.
            _ => Vec::new(),
        };
        self.wire(out)
    }

    /// `Phoenix.Socket` `handle_in` for `phx_leave`: reply ok, then close.
    // megabase:unit realtime:protocol-event:phx_leave
    fn phx_leave(&mut self, join_ref: &Value, topic: &str, reference: Value) -> Vec<Frame> {
        self.stop(topic);
        vec![
            Frame::reply(join_ref.clone(), reference, topic, "ok", json!({})),
            Frame::close(join_ref, topic),
        ]
    }

    /// `handle_in("access_token", …)`: a refreshed token, never replied to.
    // megabase:unit realtime:protocol-event:access_token
    // megabase:unit realtime:client-event:access_token
    fn access_token(&mut self, frame: Frame, now_ms: i64) -> Vec<Frame> {
        // `nil`, a non-string, no `access_token` or a non-map payload: no-op.
        let Value::Object(mut payload) = frame.payload else {
            return Vec::new();
        };
        let Some(Value::String(token)) = payload.remove("access_token") else {
            return Vec::new();
        };
        let Some(channel) = self.channels.get_mut(&frame.topic) else {
            return Vec::new();
        };
        if token.starts_with("sb_")
            || token == channel.config.access_token
            || channel.pending_access_token.as_ref() == Some(&token)
        {
            return Vec::new();
        }
        match channel.access_token_verified_at {
            // `hold_access_token/3`: the first hold of a window arms the timer.
            Some(verified_at) if now_ms - verified_at < ACCESS_TOKEN_THROTTLE_MS => {
                if channel.pending_access_token.is_none() {
                    channel
                        .apply_pending_at
                        .push(verified_at + ACCESS_TOKEN_THROTTLE_MS);
                }
                channel.pending_access_token = Some(token);
                Vec::new()
            }
            _ => self.apply_access_token(&frame.topic, token, frame.reference, now_ms),
        }
    }

    /// `apply_access_token/3` on a public channel: confirm the token, or
    /// shut the channel down with the reason.
    fn apply_access_token(
        &mut self,
        topic: &str,
        token: String,
        reference: Value,
        now_ms: i64,
    ) -> Vec<Frame> {
        let confirmed = self.confirm_token(&token, now_ms);
        let Some(channel) = self.channels.get_mut(topic) else {
            return Vec::new();
        };
        let message = match confirmed {
            Ok(confirm_token_at) => {
                channel.config.access_token = token;
                channel.config.confirm_token_at = confirm_token_at;
                channel.access_token_verified_at = Some(now_ms);
                channel.pending_access_token = None;
                return Vec::new();
            }
            Err(error) => error,
        };
        let join_ref = channel.join_ref.clone();
        self.token_rejected(&join_ref, topic, &message, reference)
    }

    /// The `else` branch of `apply_access_token/3` (and of
    /// `handle_info(:confirm_token, …)`): a `system` error, then the stop.
    fn token_rejected(
        &mut self,
        join_ref: &Value,
        topic: &str,
        error: &Unconfirmed,
        reference: Value,
    ) -> Vec<Frame> {
        match refresh_error(error) {
            Some(message) => self.shutdown(join_ref, topic, &message),
            None => vec![Frame::reply(
                join_ref.clone(),
                reference,
                topic,
                "error",
                not_implemented("realtime:client-event:access_token"),
            )],
        }
    }

    /// The earliest channel timer (`:confirm_token`,
    /// `:apply_pending_access_token`), in Unix milliseconds.
    pub(crate) fn deadline(&self) -> Option<i64> {
        self.channels
            .values()
            .flat_map(|channel| {
                channel
                    .apply_pending_at
                    .iter()
                    .copied()
                    .chain([channel.config.confirm_token_at])
            })
            .min()
    }

    /// Fires the channel timers due at `now_ms`.
    pub(crate) fn tick(&mut self, now_ms: i64) -> Vec<Wire> {
        let due: Vec<String> = self
            .channels
            .iter()
            .filter(|(_, channel)| {
                channel.config.confirm_token_at <= now_ms
                    || channel.apply_pending_at.iter().any(|at| *at <= now_ms)
            })
            .map(|(topic, _)| topic.to_owned())
            .collect();
        let mut out = Vec::new();
        for topic in due {
            let Some(channel) = self.channels.get_mut(&topic) else {
                continue;
            };
            let armed = channel.apply_pending_at.len();
            channel.apply_pending_at.retain(|at| *at > now_ms);
            let applying = channel.apply_pending_at.len() < armed;
            let confirming = channel.config.confirm_token_at <= now_ms;
            // `:apply_pending_access_token`, and `:confirm_token` with a held
            // token, apply it; the new confirmation re-arms `:confirm_token`.
            if applying || confirming {
                if let Some(token) = channel.pending_access_token.take() {
                    out.extend(self.apply_access_token(&topic, token, Value::Null, now_ms));
                    continue;
                }
            }
            if confirming {
                out.extend(self.reconfirm(&topic, now_ms));
            }
        }
        self.wire(out)
    }

    /// `handle_info(:confirm_token, …)`: the current token again; expiry
    /// shuts the channel down.
    fn reconfirm(&mut self, topic: &str, now_ms: i64) -> Vec<Frame> {
        let Some(channel) = self.channels.get(topic) else {
            return Vec::new();
        };
        let confirmed = self.confirm_token(&channel.config.access_token, now_ms);
        let join_ref = channel.join_ref.clone();
        match confirmed {
            Ok(confirm_token_at) => {
                if let Some(channel) = self.channels.get_mut(topic) {
                    channel.config.confirm_token_at = confirm_token_at;
                }
                Vec::new()
            }
            // No clause matches `{:error, :token_malformed, msg}`: the channel
            // crashes. The token was confirmed before, so in practice only
            // expiry is reachable.
            Err(Unconfirmed::Token(TokenError::Malformed)) => {
                self.stop(topic);
                vec![Frame::error(&join_ref, topic)]
            }
            Err(error) => self.token_rejected(&join_ref, topic, &error, Value::Null),
        }
    }

    /// `BroadcastHandler.handle/3` on a public channel: fan out to the other
    /// subscribers (and this one with `self`), reply only with `ack`. `size`
    /// is the payload's external term size.
    // megabase:unit realtime:client-event:broadcast
    fn broadcast(
        &mut self,
        join_ref: Value,
        topic: &str,
        reference: Value,
        body: &Broadcast,
        size: usize,
    ) -> Vec<Wire> {
        let Some(channel) = self.channels.get(topic) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let (status, response) = if size > MAX_PAYLOAD_SIZE {
            ("error", json!({ "error": "payload_size_exceeded" }))
        } else {
            self.hub.broadcast(topic, self.conn, body);
            // `pubsub_broadcast/5` reaches this socket's own fastlane first.
            if channel.config.self_broadcast {
                out.extend(body.fastlane(self.serializer, topic));
            }
            ("ok", json!({}))
        };
        if channel.config.ack_broadcast {
            let reply = Frame::reply(join_ref, reference, topic, status, response);
            out.push(Wire::Text(self.serializer.encode(reply)));
        }
        out
    }

    /// `handle_in("presence", …)` with `PresenceHandler.handle/2`.
    // megabase:unit realtime:client-event:presence
    fn presence(&mut self, join_ref: &Value, frame: &Frame, now_ms: i64) -> Vec<Frame> {
        let topic = frame.topic.as_str();
        let reply = |status: &str, response: Value| {
            vec![Frame::reply(
                join_ref.clone(),
                frame.reference.clone(),
                topic,
                status,
                response,
            )]
        };
        // `handle(_, socket)`: no `event` is a no-op.
        let Some(event) = frame.payload.as_object().and_then(|p| p.get("event")) else {
            return reply("ok", json!({}));
        };
        // `String.downcase/2` raises on a non-string: the channel crashes.
        let Some(event) = event.as_str().map(str::to_ascii_lowercase) else {
            self.stop(topic);
            return vec![Frame::error(join_ref, topic)];
        };
        let Some(channel) = self.channels.get_mut(topic) else {
            return Vec::new();
        };
        if !channel.allow_presence_event(now_ms) {
            return self.shutdown(join_ref, topic, "Client presence rate limit exceeded");
        }
        match event.as_str() {
            "track" => {
                let payload = frame
                    .payload
                    .get("payload")
                    .map_or_else(|| json!({}), Value::clone);
                let Value::Object(meta) = &payload else {
                    return reply(
                        "error",
                        json!({ "reason": "Presence track payload must be a map" }),
                    );
                };
                // `check_track_payload/2`: an unchanged payload is a no-op.
                if channel.presence_payload.as_ref() == Some(&payload) {
                    return reply("ok", json!({}));
                }
                if external_size(&payload) > MAX_PAYLOAD_SIZE {
                    return self.shutdown(join_ref, topic, "Track message size exceeded");
                }
                self.hub
                    .track(topic, self.conn, &channel.config.presence_key, meta);
                let resync = !channel.config.presence_enabled;
                channel.config.presence_enabled = true;
                channel.presence_payload = Some(payload);
                let mut out = reply("ok", json!({}));
                if resync {
                    out.push(self.presence_state(join_ref.clone(), topic));
                }
                out
            }
            "untrack" => {
                self.hub.untrack(topic, self.conn);
                channel.presence_payload = None;
                reply("ok", json!({}))
            }
            // `{:error, :unknown_presence_event}` -> `{:reply, :error, socket}`.
            _ => reply("error", json!({})),
        }
    }

    /// `shutdown_response/2`: a `system` error push
    /// (`push_system_message/5` with a string message), then the channel
    /// stops.
    // megabase:unit realtime:server-event:system
    fn shutdown(&mut self, join_ref: &Value, topic: &str, message: &str) -> Vec<Frame> {
        self.stop(topic);
        let channel = topic.strip_prefix("realtime:").unwrap_or(topic);
        vec![
            Frame::push(
                join_ref.clone(),
                topic,
                "system",
                &json!({
                    "extension": "system",
                    "status": "error",
                    "message": message,
                    "channel": channel,
                }),
            ),
            Frame::close(join_ref, topic),
        ]
    }

    /// `RealtimeChannel.join/3` for a public channel without Postgres changes.
    fn join(&self, topic: &str, params: &Value, now_ms: i64) -> Result<JoinConfig, Value> {
        let sub_topic = &topic["realtime:".len()..];
        if sub_topic.is_empty() {
            return Err(reason("TopicNameRequired: You must provide a topic name"));
        }
        let empty = Map::new();
        let params = params.as_object().unwrap_or(&empty);
        let access_token = self.assign_access_token(params);
        let confirm_token_at = self
            .confirm_token(access_token, now_ms)
            .map_err(|error| join_error(&error))?;

        let config = params.get("config");
        let field = |path: &[&str]| -> Option<&Value> {
            path.iter()
                .try_fold(config?, |value, key| value.as_object()?.get(*key))
        };
        let flag = |path: &[&str]| field(path).is_some_and(flexible_true);
        // `pg_change_params/5`. The new API takes the non-null entries of a
        // non-empty `postgres_changes` list; the old API reads the topic.
        let postgres_changes: Vec<Value> = match config {
            None => {
                let mut parts = sub_topic.splitn(3, ':');
                let mut params = Map::new();
                for key in ["schema", "table", "filter"] {
                    if let Some(part) = parts.next() {
                        params.insert(key.to_owned(), Value::from(part));
                    }
                }
                vec![Value::Object(params)]
            }
            Some(_) => match field(&["postgres_changes"]) {
                Some(Value::Array(changes)) => {
                    changes.iter().filter(|c| !c.is_null()).cloned().collect()
                }
                _ => Vec::new(),
            },
        };
        let unit = if config.is_none() && self.cdc.is_none() {
            Some("realtime:server-event:postgres_changes")
        } else if config.is_none() {
            None
        } else if flag(&["private"]) {
            Some("realtime:protocol-event:phx_join")
        } else if field(&["broadcast"])
            .and_then(Value::as_object)
            .is_some_and(|broadcast| broadcast.contains_key("replay"))
        {
            // `maybe_replay_messages/5` on a public channel matches any
            // `replay` value, `null` included.
            return Err(reason(
                "UnableToReplayMessages: Replay is not allowed for public channels",
            ));
        } else if self.cdc.is_none() && !postgres_changes.is_empty() {
            // No tenant database to subscribe in.
            Some("realtime:server-event:postgres_changes")
        } else if flag(&["broadcast", "replication_ready"]) {
            Some("realtime:server-event:system")
        } else {
            None
        };
        if let Some(unit) = unit {
            return Err(not_implemented(unit));
        }
        // `Join.presence_key/1`: a non-empty string, number or boolean, else
        // a fresh UUID v1. The key is a JSON object key on the wire.
        let presence_key = match field(&["presence", "key"]) {
            Some(Value::String(key)) if !key.is_empty() => key.to_owned(),
            Some(Value::Number(key)) => key.to_string(),
            Some(Value::Bool(key)) => key.to_string(),
            _ => uuid1(),
        };
        Ok(JoinConfig {
            ack_broadcast: flag(&["broadcast", "ack"]),
            self_broadcast: flag(&["broadcast", "self"]),
            // The self-hosted tenant has `presence_enabled` false.
            presence_enabled: flag(&["presence", "enabled"]),
            presence_key,
            access_token: access_token.to_owned(),
            confirm_token_at,
            is_new_api: config.is_some(),
            postgres_changes,
        })
    }

    /// `assign_access_token/2`: `user_token`, else `access_token` (an `sb_`
    /// key means the socket token), else the socket token.
    fn assign_access_token<'p>(&'p self, params: &'p Map<String, Value>) -> &'p str {
        let string = |key: &str| params.get(key).and_then(Value::as_str);
        let sb_key = params
            .get("access_token")
            .filter(|v| !v.is_null())
            .or_else(|| params.get("user_token"))
            .and_then(Value::as_str)
            .is_some_and(|token| token.starts_with("sb_"));
        if sb_key {
            self.socket_token.as_str()
        } else {
            string("user_token")
                .or_else(|| string("access_token"))
                .unwrap_or(&self.socket_token)
        }
    }

    /// `confirm_token/1` at Unix time `now_ms`: when the `:confirm_token`
    /// timer fires next, `min(1 h, exp - now)` from now.
    fn confirm_token(&self, token: &str, now_ms: i64) -> Result<i64, Unconfirmed> {
        let now = now_ms.div_euclid(1000);
        let claims = authorize_conn(self.jwt, token, now).map_err(Unconfirmed::Token)?;
        let Some(exp) = claims.get("exp").and_then(Value::as_i64) else {
            return Err(Unconfirmed::ExpDiff(None));
        };
        let exp_diff = exp.saturating_sub(now);
        if exp_diff <= 0 {
            return Err(Unconfirmed::ExpDiff(Some(exp_diff)));
        }
        let interval = exp_diff
            .saturating_mul(1000)
            .min(CONFIRM_TOKEN_MAX_INTERVAL_MS);
        Ok(now_ms.saturating_add(interval))
    }
}

/// The join's `phx_reply` error response for a token `confirm_token/1`
/// rejected (`log_error/3` reasons).
fn join_error(error: &Unconfirmed) -> Value {
    match error {
        Unconfirmed::Token(TokenError::Expired(seconds)) => reason(&format!(
            "InvalidJWTToken: Token has expired {seconds} seconds ago"
        )),
        Unconfirmed::Token(TokenError::MissingClaims) => {
            reason("InvalidJWTToken: Fields `role` and `exp` are required in JWT")
        }
        Unconfirmed::Token(TokenError::Malformed) => {
            reason("MalformedJWT: The token provided is not a valid JWT")
        }
        Unconfirmed::Token(TokenError::Signature) => {
            reason("JwtSignatureError: Failed to validate JWT signature")
        }
        Unconfirmed::Token(TokenError::Signer) => reason(
            "JwtSignerError: Failed to generate JWT signer, check your JWT secret or JWKS configuration",
        ),
        Unconfirmed::Token(TokenError::UnsupportedAlgorithm) => {
            not_implemented("realtime:protocol-event:phx_join")
        }
        Unconfirmed::ExpDiff(Some(_)) => {
            reason("InvalidJWTToken: Token expiration time is invalid")
        }
        Unconfirmed::Token(TokenError::Other) | Unconfirmed::ExpDiff(None) => {
            reason("Unknown Error on Channel")
        }
    }
}

/// The `system` message `apply_access_token/3` shuts the channel down
/// with; `None` where the upstream text is an `inspect/1` of a term
/// Megabase does not model.
fn refresh_error(error: &Unconfirmed) -> Option<String> {
    let message = match error {
        Unconfirmed::Token(TokenError::Expired(seconds)) => {
            format!("Token has expired {seconds} seconds ago")
        }
        Unconfirmed::Token(TokenError::MissingClaims) => {
            "Fields `role` and `exp` are required in JWT".to_owned()
        }
        Unconfirmed::Token(TokenError::Malformed) => {
            "The token provided is not a valid JWT".to_owned()
        }
        Unconfirmed::Token(TokenError::Signer) => {
            "Failed to generate JWT signer, check your JWT secret or JWKS configuration".to_owned()
        }
        // `inspect(:signature_error)` and `inspect(exp_diff)`.
        Unconfirmed::Token(TokenError::Signature) => ":signature_error".to_owned(),
        Unconfirmed::ExpDiff(Some(exp_diff)) => exp_diff.to_string(),
        Unconfirmed::Token(TokenError::UnsupportedAlgorithm | TokenError::Other)
        | Unconfirmed::ExpDiff(None) => return None,
    };
    Some(message)
}

/// `FlexibleBoolean.cast/1` yielding `true`.
fn flexible_true(value: &Value) -> bool {
    match value {
        Value::Bool(flag) => *flag,
        Value::String(text) => text.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn reason(message: &str) -> Value {
    json!({ "reason": message })
}

/// Join or event reply for behaviour Megabase does not serve yet.
fn not_implemented(unit: &str) -> Value {
    let body = MegabaseNotImplemented::new(COMPONENT, unit);
    json!({
        "reason": body.message,
        "code": body.code,
        "component": body.component,
        "unit": body.unit,
    })
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::Receiver;

    use crate::hub::OUTBOX_CAPACITY;

    use super::*;

    const SECRET: &str = "super-secret-jwt-token-with-at-least-32-characters-long";
    const NOW: i64 = 1_700_000_000;

    fn jwt() -> Hs256 {
        Hs256::new(SECRET).unwrap()
    }

    fn token(claims: &Value) -> String {
        jwt().sign(claims).unwrap().as_str().to_owned()
    }

    fn anon() -> String {
        token(&json!({"role": "anon", "exp": NOW + 3600}))
    }

    fn frame(text: &str) -> Frame {
        Serializer::V2.decode(text).unwrap()
    }

    fn open<'a>(
        jwt: &'a Hs256,
        hub: &'a Hub,
        serializer: Serializer,
    ) -> (Session<'a>, Receiver<Wire>) {
        let (outbox, inbox) = Outbox::new(OUTBOX_CAPACITY);
        (Session::new(jwt, hub, serializer, anon(), outbox), inbox)
    }

    fn texts(wires: Vec<Wire>) -> Vec<String> {
        wires
            .into_iter()
            .map(|wire| match wire {
                Wire::Text(text) => text,
                Wire::Binary(bytes) => panic!("unexpected binary {bytes:?}"),
            })
            .collect()
    }

    fn run_at(session: &mut Session<'_>, text: &str, now_ms: i64) -> Vec<String> {
        texts(session.handle(frame(text), now_ms))
    }

    fn run(session: &mut Session<'_>, text: &str) -> Vec<String> {
        run_at(session, text, NOW * 1000)
    }

    fn drain(inbox: &mut Receiver<Wire>) -> Vec<Value> {
        let mut out = Vec::new();
        while let Ok(wire) = inbox.try_recv() {
            out.extend(values(&texts(vec![wire])));
        }
        out
    }

    fn values(frames: &[String]) -> Vec<Value> {
        frames
            .iter()
            .map(|text| serde_json::from_str(text).unwrap())
            .collect()
    }

    fn join(config: &Value) -> String {
        json!(["1", "1", "realtime:room", "phx_join", { "config": config }]).to_string()
    }

    const JOIN: &str = r#"["1","1","realtime:room","phx_join",{"config":{"broadcast":{"ack":false,"self":false},"presence":{"key":"","enabled":false},"postgres_changes":[],"private":false}}]"#;

    #[test]
    fn negotiates_vsn() {
        assert_eq!(Serializer::negotiate(None), Some(Serializer::V1));
        assert_eq!(Serializer::negotiate(Some("1.0.0")), Some(Serializer::V1));
        assert_eq!(Serializer::negotiate(Some("2.0.3")), Some(Serializer::V2));
        for vsn in ["2.1.0", "3.0.0", "2.0", "x", "2.0.0-rc", "02.0.0", ""] {
            assert_eq!(Serializer::negotiate(Some(vsn)), None, "{vsn}");
        }
    }

    #[test]
    fn heartbeat_replies_ok() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut session, _inbox) = open(&jwt, &hub, Serializer::V2);
        assert_eq!(
            run(&mut session, r#"[null,"7","phoenix","heartbeat",{}]"#),
            [r#"[null,"7","phoenix","phx_reply",{"response":{},"status":"ok"}]"#]
        );
    }

    #[test]
    fn join_then_leave() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut session, _inbox) = open(&jwt, &hub, Serializer::V2);
        assert_eq!(
            run(&mut session, JOIN),
            [
                r#"["1","1","realtime:room","phx_reply",{"response":{"postgres_changes":[]},"status":"ok"}]"#
            ]
        );
        assert_eq!(
            run(&mut session, r#"["1","2","realtime:room","phx_leave",{}]"#),
            [
                r#"["1","2","realtime:room","phx_reply",{"response":{},"status":"ok"}]"#,
                r#"["1","1","realtime:room","phx_close",{}]"#
            ]
        );
        assert_eq!(
            run(&mut session, r#"["1","3","realtime:room","phx_leave",{}]"#),
            [
                r#"["1","3","realtime:room","phx_reply",{"response":{"reason":"unmatched topic"},"status":"error"}]"#
            ]
        );
    }

    #[test]
    fn duplicate_join_closes_the_old_channel() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut session, _inbox) = open(&jwt, &hub, Serializer::V2);
        run(&mut session, JOIN);
        let again = JOIN.replacen(r#"["1","1""#, r#"["4","4""#, 1);
        let out = run(&mut session, &again);
        assert_eq!(out[0], r#"["1","1","realtime:room","phx_close",{}]"#);
        assert!(out[1].starts_with(r#"["4","4","realtime:room","phx_reply""#));
        // The old join ref no longer reaches the channel.
        assert!(run(&mut session, r#"["1","5","realtime:room","phx_leave",{}]"#).is_empty());
    }

    #[test]
    fn unmatched_topics() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut session, _inbox) = open(&jwt, &hub, Serializer::V2);
        assert_eq!(
            run(&mut session, r#"["1","1","room","phx_join",{}]"#),
            [
                r#"["1","1","room","phx_reply",{"response":{"reason":"unmatched topic"},"status":"error"}]"#
            ]
        );
        assert_eq!(
            run(&mut session, r#"["1","1","realtime:room","broadcast",{}]"#),
            [
                r#"["1","1","realtime:room","phx_reply",{"response":{"reason":"unmatched topic"},"status":"error"}]"#
            ]
        );
    }

    #[test]
    fn empty_topic_name_is_rejected() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut session, _inbox) = open(&jwt, &hub, Serializer::V2);
        let out = run(
            &mut session,
            r#"["1","1","realtime:","phx_join",{"config":{}}]"#,
        );
        assert!(out[0].contains("TopicNameRequired: You must provide a topic name"));
    }

    #[test]
    fn join_token_errors() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut session, _inbox) = open(&jwt, &hub, Serializer::V2);
        let cases = [
            (
                token(&json!({"role": "anon", "exp": NOW - 10})),
                "InvalidJWTToken: Token has expired 10 seconds ago",
            ),
            (
                token(&json!({"role": "anon"})),
                "InvalidJWTToken: Fields `role` and `exp` are required in JWT",
            ),
            (
                "nope".to_owned(),
                "MalformedJWT: The token provided is not a valid JWT",
            ),
        ];
        for (access_token, expected) in cases {
            let text = json!(["1", "1", "realtime:room", "phx_join",
                {"config": {}, "access_token": access_token}])
            .to_string();
            let out = run(&mut session, &text);
            assert!(out[0].contains(expected), "{}", out[0]);
            assert!(out[0].contains(r#""status":"error""#));
        }
    }

    #[test]
    fn sb_access_token_uses_the_socket_token() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut session, _inbox) = open(&jwt, &hub, Serializer::V2);
        let text = r#"["1","1","realtime:room","phx_join",{"config":{},"access_token":"sb_publishable_x"}]"#;
        assert!(run(&mut session, text)[0].contains(r#""status":"ok""#));
    }

    #[test]
    fn unserved_join_options_are_not_implemented() {
        let jwt = jwt();
        for (payload, unit) in [
            (json!({}), "realtime:server-event:postgres_changes"),
            (
                json!({"config": {"private": "TRUE"}}),
                "realtime:protocol-event:phx_join",
            ),
            (
                json!({"config": {"postgres_changes": [{"event": "*", "schema": "public"}]}}),
                "realtime:server-event:postgres_changes",
            ),
        ] {
            let hub = Hub::default();
            let (mut session, _inbox) = open(&jwt, &hub, Serializer::V2);
            let text = json!(["1", "1", "realtime:room", "phx_join", payload]).to_string();
            let out = run(&mut session, &text);
            assert!(
                out[0].contains(r#""code":"MEGABASE_NOT_IMPLEMENTED""#),
                "{}",
                out[0]
            );
            assert!(out[0].contains(unit), "{}", out[0]);
        }
    }

    #[test]
    fn replay_on_a_public_channel_is_refused() {
        let jwt = jwt();
        for replay in [json!({"since": 0, "limit": 5}), Value::Null] {
            let hub = Hub::default();
            let (mut session, _inbox) = open(&jwt, &hub, Serializer::V2);
            let out = run(
                &mut session,
                &join(&json!({"broadcast": {"replay": replay}})),
            );
            assert_eq!(
                values(&out),
                [
                    json!(["1", "1", "realtime:room", "phx_reply", {"status": "error",
                    "response": {"reason": "UnableToReplayMessages: Replay is not allowed for public channels"}}])
                ]
            );
        }
    }

    #[test]
    fn too_many_joins_reject_the_join_and_disconnect() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut session, _inbox) = open(&jwt, &hub, Serializer::V2);
        let now = NOW * 1000;
        for _ in 0..1000 {
            assert!(hub.count_join(now));
        }
        // Below the limit until the next tick.
        assert!(run_at(&mut session, JOIN, now)[0].contains(r#""status":"ok""#));
        assert!(!session.disconnected());
        assert!(hub.too_many_joins(now + 5_000));
        let other = JOIN.replace("realtime:room", "realtime:other");
        assert_eq!(
            values(&run_at(&mut session, &other, now + 5_000)),
            [
                json!(["1", "1", "realtime:other", "phx_reply", {"status": "error",
                "response": {"reason": "ClientJoinRateLimitReached: Too many joins per second"}}])
            ]
        );
        assert!(session.disconnected());
    }

    #[test]
    fn an_empty_topic_is_not_counted() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut session, _inbox) = open(&jwt, &hub, Serializer::V2);
        let now = NOW * 1000;
        for _ in 0..1000 {
            hub.count_join(now);
        }
        let empty = JOIN.replace("realtime:room", "realtime:");
        let out = run_at(&mut session, &empty, now + 5_000);
        assert!(out[0].contains("TopicNameRequired"), "{}", out[0]);
        assert!(!session.disconnected());
    }

    #[test]
    fn unknown_channel_events_are_ignored() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut session, _inbox) = open(&jwt, &hub, Serializer::V2);
        run(&mut session, JOIN);
        assert!(run(&mut session, r#"["1","3","realtime:room","whatever",{}]"#).is_empty());
    }

    fn refresh(token: &Value) -> String {
        json!(["1", "2", "realtime:room", "access_token", { "access_token": token }]).to_string()
    }

    fn system_close(message: &str) -> Vec<Value> {
        vec![
            json!(["1", null, "realtime:room", "system", {"channel": "room",
                "extension": "system", "message": message, "status": "error"}]),
            json!(["1", "1", "realtime:room", "phx_close", {}]),
        ]
    }

    #[test]
    fn access_token_refresh_is_silent_and_moves_the_expiry() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut session, _inbox) = open(&jwt, &hub, Serializer::V2);
        run(&mut session, JOIN);
        // `min(1 h, exp - now)`: the anon token expires in exactly an hour.
        assert_eq!(session.deadline(), Some((NOW + 3600) * 1000));

        let later = token(&json!({"role": "authenticated", "exp": NOW + 600}));
        assert!(run(&mut session, &refresh(&json!(later))).is_empty());
        assert_eq!(session.deadline(), Some((NOW + 600) * 1000));

        // The same token, `null`, an `sb_` key, a non-string and a payload
        // without `access_token` are no-ops.
        for payload in [
            refresh(&json!(later)),
            refresh(&Value::Null),
            refresh(&json!("sb_publishable_x")),
            refresh(&json!(5)),
            r#"["1","2","realtime:room","access_token",{}]"#.to_owned(),
            r#"["1","2","realtime:room","access_token","x"]"#.to_owned(),
        ] {
            assert!(run(&mut session, &payload).is_empty(), "{payload}");
        }
        assert_eq!(session.deadline(), Some((NOW + 600) * 1000));
    }

    #[test]
    fn access_token_errors_shut_the_channel_down() {
        let jwt = jwt();
        let other = Hs256::new("another-secret-that-is-at-least-32-bytes!!")
            .unwrap()
            .sign(&json!({"role": "anon", "exp": NOW + 60}))
            .unwrap();
        for (refreshed, message) in [
            (
                token(&json!({"role": "anon", "exp": NOW - 7})),
                "Token has expired 7 seconds ago",
            ),
            (
                token(&json!({"exp": NOW + 60})),
                "Fields `role` and `exp` are required in JWT",
            ),
            ("nope".to_owned(), "The token provided is not a valid JWT"),
            (other.as_str().to_owned(), ":signature_error"),
            (
                // Rounds down to `now`: `inspect(exp_diff)`.
                token(&json!({"role": "anon", "exp": NOW as f64 + 0.25})),
                "0",
            ),
        ] {
            let hub = Hub::default();
            let (mut session, _inbox) = open(&jwt, &hub, Serializer::V2);
            run(&mut session, JOIN);
            assert_eq!(
                values(&run(&mut session, &refresh(&json!(refreshed)))),
                system_close(message)
            );
            assert!(run(&mut session, BROADCAST)[0].contains("unmatched topic"));
            assert_eq!(session.deadline(), None);
        }
    }

    #[test]
    fn access_token_refreshes_are_throttled() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut session, _inbox) = open(&jwt, &hub, Serializer::V2);
        run(&mut session, JOIN);
        let start = NOW * 1000;
        let first = token(&json!({"role": "anon", "exp": NOW + 1000}));
        assert!(run_at(&mut session, &refresh(&json!(first)), start).is_empty());

        // Inside the window the token is held, the timer armed once.
        let held = token(&json!({"role": "anon", "exp": NOW + 2000}));
        let last = token(&json!({"role": "anon", "exp": NOW + 3000}));
        assert!(run_at(&mut session, &refresh(&json!(held)), start + 4000).is_empty());
        assert!(run_at(&mut session, &refresh(&json!(last)), start + 5000).is_empty());
        assert_eq!(session.deadline(), Some(start + ACCESS_TOKEN_THROTTLE_MS));
        assert!(session
            .tick(start + ACCESS_TOKEN_THROTTLE_MS - 1)
            .is_empty());

        // The timer applies the last held token: it expires 2990 s later.
        assert!(session.tick(start + ACCESS_TOKEN_THROTTLE_MS).is_empty());
        let applied = start + ACCESS_TOKEN_THROTTLE_MS;
        assert_eq!(session.deadline(), Some(applied + 2_990_000));

        // A held token that turns out invalid shuts the channel down.
        let expired = token(&json!({"role": "anon", "exp": NOW - 1}));
        assert!(run_at(&mut session, &refresh(&json!(expired)), applied + 1).is_empty());
        let out = session.tick(applied + ACCESS_TOKEN_THROTTLE_MS);
        let out = texts(out);
        assert_eq!(
            values(&out),
            system_close("Token has expired 21 seconds ago")
        );
    }

    #[test]
    fn token_expiry_shuts_the_channel_down() {
        let jwt = jwt();
        let hub = Hub::default();
        let (outbox, _inbox) = Outbox::new(OUTBOX_CAPACITY);
        let short = token(&json!({"role": "anon", "exp": NOW + 30}));
        let mut session = Session::new(&jwt, &hub, Serializer::V2, short, outbox);
        run(&mut session, JOIN);
        let expiry = (NOW + 30) * 1000;
        assert_eq!(session.deadline(), Some(expiry));
        assert!(session.tick(expiry - 1).is_empty());
        let out = texts(session.tick(expiry));
        assert_eq!(
            values(&out),
            system_close("Token has expired 0 seconds ago")
        );
        assert_eq!(session.deadline(), None);
    }

    #[test]
    fn far_expiry_is_reconfirmed_hourly() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut session, _inbox) = open(&jwt, &hub, Serializer::V2);
        let config = json!({});
        let long = token(&json!({"role": "anon", "exp": NOW + 10 * 3600}));
        let text = json!(["1", "1", "realtime:room", "phx_join",
            {"config": config, "access_token": long}])
        .to_string();
        run(&mut session, &text);
        let first = NOW * 1000 + CONFIRM_TOKEN_MAX_INTERVAL_MS;
        assert_eq!(session.deadline(), Some(first));
        assert!(session.tick(first).is_empty());
        assert_eq!(
            session.deadline(),
            Some(first + CONFIRM_TOKEN_MAX_INTERVAL_MS)
        );
    }

    const BROADCAST: &str = r#"["1","2","realtime:room","broadcast",{"type":"broadcast","event":"e","payload":{"n":1}}]"#;

    #[test]
    fn broadcast_reaches_the_other_subscribers() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut a, mut a_inbox) = open(&jwt, &hub, Serializer::V2);
        let (mut b, mut b_inbox) = open(&jwt, &hub, Serializer::V2);
        let (mut c, mut c_inbox) = open(&jwt, &hub, Serializer::V2);
        run(&mut a, JOIN);
        run(&mut b, JOIN);
        run(&mut c, &JOIN.replace("realtime:room", "realtime:other"));

        // Without `ack` there is no reply; without `self` no echo.
        assert!(run(&mut a, BROADCAST).is_empty());
        assert!(drain(&mut a_inbox).is_empty());
        assert_eq!(
            drain(&mut b_inbox),
            [json!([null, null, "realtime:room", "broadcast",
                {"type": "broadcast", "event": "e", "payload": {"n": 1}}])]
        );
        assert!(drain(&mut c_inbox).is_empty());

        // After leaving, b no longer receives.
        run(&mut b, r#"["1","3","realtime:room","phx_leave",{}]"#);
        run(&mut a, BROADCAST);
        assert!(drain(&mut b_inbox).is_empty());
    }

    #[test]
    fn broadcast_self_and_ack() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut a, mut a_inbox) = open(&jwt, &hub, Serializer::V2);
        run(
            &mut a,
            &join(&json!({"broadcast": {"self": true, "ack": "true"}})),
        );
        assert_eq!(
            values(&run(&mut a, BROADCAST)),
            [
                json!([null, null, "realtime:room", "broadcast",
                    {"type": "broadcast", "event": "e", "payload": {"n": 1}}]),
                json!(["1", "2", "realtime:room", "phx_reply", {"status": "ok", "response": {}}]),
            ]
        );
        assert!(drain(&mut a_inbox).is_empty());

        // A V2 socket drops a non-map payload; the ack still replies.
        assert_eq!(
            values(&run(
                &mut a,
                r#"["1","3","realtime:room","broadcast","text"]"#
            )),
            [json!(["1", "3", "realtime:room", "phx_reply", {"status": "ok", "response": {}}])]
        );
    }

    #[test]
    fn broadcast_payload_size_limit() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut a, _a_inbox) = open(&jwt, &hub, Serializer::V2);
        let (mut b, mut b_inbox) = open(&jwt, &hub, Serializer::V2);
        run(&mut a, &join(&json!({"broadcast": {"ack": true}})));
        run(&mut b, JOIN);
        let big = json!(["1", "2", "realtime:room", "broadcast",
            {"event": "e", "payload": "x".repeat(MAX_PAYLOAD_SIZE)}])
        .to_string();
        assert_eq!(
            values(&run(&mut a, &big)),
            [json!(["1", "2", "realtime:room", "phx_reply",
                {"status": "error", "response": {"error": "payload_size_exceeded"}}])]
        );
        assert!(drain(&mut b_inbox).is_empty());
    }

    fn track(payload: &Value) -> String {
        json!(["1", "5", "realtime:room", "presence",
            {"type": "presence", "event": "track", "payload": payload}])
        .to_string()
    }

    const OK5: &str = r#"["1","5","realtime:room","phx_reply",{"response":{},"status":"ok"}]"#;

    #[test]
    fn presence_enabled_join_pushes_state() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut a, mut a_inbox) = open(&jwt, &hub, Serializer::V2);
        let (mut b, mut b_inbox) = open(&jwt, &hub, Serializer::V2);
        let config = json!({"presence": {"enabled": true, "key": "alice"}});
        let out = values(&run(&mut a, &join(&config)));
        assert_eq!(
            out[1],
            json!(["1", null, "realtime:room", "presence_state", {}])
        );

        // Presence was on at join: no resync, only the diff (to a itself).
        assert_eq!(run(&mut a, &track(&json!({"online_at": 1}))), [OK5]);
        assert_eq!(drain(&mut a_inbox)[0][3], "presence_diff");
        assert!(drain(&mut b_inbox).is_empty(), "b has not joined");

        // b joins after the track and sees alice in its state.
        let config = json!({"presence": {"enabled": true, "key": 7}});
        let out = values(&run(&mut b, &join(&config)));
        let meta = &out[1][4]["alice"]["metas"][0];
        assert_eq!(meta["online_at"], 1);
        assert!(meta["phx_ref"].is_string());
    }

    #[test]
    fn presence_track_update_untrack() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut a, mut a_inbox) = open(&jwt, &hub, Serializer::V2);
        let (mut b, mut b_inbox) = open(&jwt, &hub, Serializer::V2);
        run(&mut a, &join(&json!({"presence": {"key": "alice"}})));
        run(&mut b, JOIN);

        // Presence was off: the track turns it on and resyncs the state.
        let out = values(&run(&mut a, &track(&json!({"n": 1}))));
        assert_eq!(out[0], serde_json::from_str::<Value>(OK5).unwrap());
        assert_eq!(out[1][3], "presence_state");
        assert_eq!(out[1][4]["alice"]["metas"][0]["n"], 1);
        let joined = drain(&mut a_inbox);
        assert_eq!(joined, drain(&mut b_inbox));
        assert_eq!(joined[0][3], "presence_diff");
        assert_eq!(joined[0][4]["joins"]["alice"]["metas"][0]["n"], 1);
        assert_eq!(joined[0][4]["leaves"], json!({}));

        // The same payload again is a no-op.
        assert_eq!(run(&mut a, &track(&json!({"n": 1}))), [OK5]);
        assert!(drain(&mut b_inbox).is_empty());

        // A new payload updates: no resync, a diff with both sides.
        assert_eq!(run(&mut a, &track(&json!({"n": 2}))), [OK5]);
        let diff = &drain(&mut b_inbox)[0][4];
        assert_eq!(diff["joins"]["alice"]["metas"][0]["n"], 2);
        assert_eq!(diff["leaves"]["alice"]["metas"][0]["n"], 1);

        let untrack =
            r#"["1","5","realtime:room","presence",{"type":"presence","event":"UNTRACK"}]"#;
        assert_eq!(run(&mut a, untrack), [OK5]);
        let diff = &drain(&mut b_inbox)[0][4];
        assert_eq!(diff["leaves"]["alice"]["metas"][0]["n"], 2);
        assert_eq!(hub.presence_state("realtime:room"), json!({}));
        drain(&mut a_inbox);
    }

    #[test]
    fn presence_leave_and_disconnect_untrack() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut b, mut b_inbox) = open(&jwt, &hub, Serializer::V2);
        run(&mut b, JOIN);
        {
            let (mut a, _a_inbox) = open(&jwt, &hub, Serializer::V2);
            run(&mut a, &join(&json!({"presence": {"key": "alice"}})));
            run(&mut a, &track(&json!({})));
            drain(&mut b_inbox);
            // The socket closes.
        }
        let diff = &drain(&mut b_inbox)[0][4];
        assert!(diff["leaves"]["alice"]["metas"][0]["phx_ref"].is_string());

        let (mut a, _a_inbox) = open(&jwt, &hub, Serializer::V2);
        run(&mut a, &join(&json!({"presence": {"key": "alice"}})));
        run(&mut a, &track(&json!({})));
        drain(&mut b_inbox);
        run(&mut a, r#"["1","9","realtime:room","phx_leave",{}]"#);
        assert_eq!(drain(&mut b_inbox)[0][4]["joins"], json!({}));
    }

    #[test]
    fn presence_errors() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut a, _a_inbox) = open(&jwt, &hub, Serializer::V2);
        run(&mut a, JOIN);
        // No event: a plain ok.
        assert_eq!(
            run(
                &mut a,
                r#"["1","5","realtime:room","presence",{"type":"presence"}]"#
            ),
            [OK5]
        );
        assert_eq!(
            values(&run(&mut a, &track(&json!([1])))),
            [
                json!(["1", "5", "realtime:room", "phx_reply", {"status": "error",
                "response": {"reason": "Presence track payload must be a map"}}])
            ]
        );
        assert_eq!(
            run(
                &mut a,
                r#"["1","5","realtime:room","presence",{"event":"nope"}]"#
            ),
            [r#"["1","5","realtime:room","phx_reply",{"response":{},"status":"error"}]"#]
        );
        // A non-string event crashes the channel.
        assert_eq!(
            run(
                &mut a,
                r#"["1","5","realtime:room","presence",{"event":1}]"#
            ),
            [r#"["1","1","realtime:room","phx_error",{}]"#]
        );
        assert!(run(&mut a, BROADCAST)[0].contains("unmatched topic"));
    }

    #[test]
    fn presence_client_rate_limit_shuts_the_channel_down() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut a, _a_inbox) = open(&jwt, &hub, Serializer::V2);
        run(&mut a, JOIN);
        let untrack = r#"["1","5","realtime:room","presence",{"event":"untrack"}]"#;
        let start = NOW * 1000;
        for _ in 0..CLIENT_PRESENCE_MAX_CALLS {
            assert_eq!(run_at(&mut a, untrack, start), [OK5]);
        }
        assert_eq!(
            values(&run_at(&mut a, untrack, start + CLIENT_PRESENCE_WINDOW_MS)),
            [
                json!(["1", null, "realtime:room", "system", {"channel": "room",
                    "extension": "system", "message": "Client presence rate limit exceeded",
                    "status": "error"}]),
                json!(["1", "1", "realtime:room", "phx_close", {}]),
            ]
        );

        // A new window starts after `window_ms`.
        run(&mut a, JOIN);
        for _ in 0..CLIENT_PRESENCE_MAX_CALLS {
            run_at(&mut a, untrack, start);
        }
        assert_eq!(
            run_at(&mut a, untrack, start + CLIENT_PRESENCE_WINDOW_MS + 1),
            [OK5]
        );
    }

    #[test]
    fn presence_keys() {
        let jwt = jwt();
        let hub = Hub::default();
        let (session, _inbox) = open(&jwt, &hub, Serializer::V2);
        let key = |config: Value| {
            session
                .join("realtime:room", &json!({ "config": config }), NOW * 1000)
                .unwrap()
                .presence_key
        };
        assert_eq!(key(json!({"presence": {"key": "k"}})), "k");
        assert_eq!(key(json!({"presence": {"key": 5}})), "5");
        assert_eq!(key(json!({"presence": {"key": false}})), "false");
        for config in [
            json!({}),
            json!({"presence": {"key": ""}}),
            json!({"presence": {"key": {}}}),
        ] {
            let generated = key(config);
            assert_eq!(generated.len(), 36);
            assert_eq!(&generated[14..15], "1");
        }
    }

    /// realtime-js `_encodeUserBroadcastPush` for `channel.send(...)`.
    fn user_push(reference: &str, topic: &str, event: &str, json: bool, payload: &[u8]) -> Vec<u8> {
        let header = [
            3,
            1,
            reference.len(),
            topic.len(),
            event.len(),
            0,
            usize::from(json),
        ];
        let mut bytes: Vec<u8> = header.iter().map(|b| u8::try_from(*b).unwrap()).collect();
        for field in ["1", reference, topic, event] {
            bytes.extend_from_slice(field.as_bytes());
        }
        bytes.extend_from_slice(payload);
        bytes
    }

    fn user_frame(json: bool, payload: &[u8]) -> BinaryFrame {
        Serializer::decode_binary(&user_push("2", "realtime:room", "e", json, payload)).unwrap()
    }

    #[test]
    fn decodes_binary_frames() {
        let frame = user_frame(true, br#"{"n":1}"#);
        assert_eq!(frame.join_ref, json!("1"));
        assert_eq!(frame.reference, json!("2"));
        assert_eq!(frame.topic, "realtime:room");
        assert_eq!(frame.event, "broadcast");
        assert_eq!(
            frame.body,
            Broadcast::User {
                event: "e".to_owned(),
                json: true,
                payload: br#"{"n":1}"#.to_vec()
            }
        );
        assert_eq!(frame.size, user_broadcast_size("e", true, 7, &json!({})));

        // Metadata counts toward the size and must be JSON.
        let mut with_meta = vec![3, 0, 0, 1, 1, 7, 0];
        with_meta.extend_from_slice(b"te{\"a\":1}");
        let frame = Serializer::decode_binary(&with_meta).unwrap();
        assert_eq!(
            frame.size,
            user_broadcast_size("e", false, 0, &json!({"a": 1}))
        );
        assert_eq!(frame.join_ref, json!(""));

        // `@push`: any event with a `{:binary, data}` payload.
        let push = [&[0, 1, 1, 1, 1][..], b"12tbxy"].concat();
        let frame = Serializer::decode_binary(&push).unwrap();
        assert_eq!((frame.topic.as_str(), frame.event.as_str()), ("t", "b"));
        assert_eq!(frame.body, Broadcast::Binary(b"xy".to_vec()));
        assert_eq!(frame.size, binary_tuple_size(2));

        for bad in [
            &b""[..],
            &[1, 0, 0, 0, 0],
            &[4, 0, 0, 0, 0],
            &[3, 0, 0, 5, 0, 0, 0, b'a'],
            &[3, 0, 0, 0, 0, 1, 0, b'{'],
            &[0, 0, 0, 1],
            &[0, 0, 0, 1, 0, 0xff],
        ] {
            assert!(Serializer::decode_binary(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn user_broadcast_fans_out_with_self_and_ack() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut a, mut a_inbox) = open(&jwt, &hub, Serializer::V2);
        let (mut b, mut b_inbox) = open(&jwt, &hub, Serializer::V2);
        let (mut c, mut c_inbox) = open(&jwt, &hub, Serializer::V1);
        run(
            &mut a,
            &join(&json!({"broadcast": {"self": true, "ack": true}})),
        );
        run(&mut b, JOIN);
        let v1_join = Serializer::V1
            .decode(
                r#"{"topic":"realtime:room","event":"phx_join","payload":{"config":{}},"ref":"1"}"#,
            )
            .unwrap();
        c.handle(v1_join, NOW * 1000);

        let relayed = Wire::Binary([&[4, 13, 1, 0, 1][..], b"realtime:roome{\"n\":1}"].concat());
        assert_eq!(
            a.handle_binary(user_frame(true, br#"{"n":1}"#)),
            [
                relayed.clone(),
                Wire::Text(
                    r#"["1","2","realtime:room","phx_reply",{"response":{},"status":"ok"}]"#
                        .to_owned()
                ),
            ]
        );
        assert!(a_inbox.try_recv().is_err());
        assert_eq!(b_inbox.try_recv().unwrap(), relayed);
        assert_eq!(
            drain(&mut c_inbox),
            [
                json!({"event": "broadcast", "ref": null, "topic": "realtime:room",
                "payload": {"event": "e", "payload": {"n": 1}, "type": "broadcast"}})
            ]
        );

        // Without `ack` or `self`, b sends silently; a still receives.
        assert!(b.handle_binary(user_frame(false, b"\x00\x01")).is_empty());
        assert_eq!(
            a_inbox.try_recv().unwrap(),
            Wire::Binary([&[4, 13, 1, 0, 0][..], b"realtime:roome\x00\x01"].concat())
        );
        assert!(c_inbox.try_recv().is_err());
    }

    #[test]
    fn user_broadcast_payload_size_limit() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut a, _a_inbox) = open(&jwt, &hub, Serializer::V2);
        let (mut b, mut b_inbox) = open(&jwt, &hub, Serializer::V2);
        run(&mut a, &join(&json!({"broadcast": {"ack": true}})));
        run(&mut b, JOIN);
        let big = vec![b'0'; MAX_PAYLOAD_SIZE];
        assert_eq!(
            texts(a.handle_binary(user_frame(true, &big))),
            [
                r#"["1","2","realtime:room","phx_reply",{"response":{"error":"payload_size_exceeded"},"status":"error"}]"#
            ]
        );
        assert!(b_inbox.try_recv().is_err());
    }

    #[test]
    fn binary_frames_route_like_text_frames() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut a, _a_inbox) = open(&jwt, &hub, Serializer::V2);
        let (mut b, mut b_inbox) = open(&jwt, &hub, Serializer::V2);
        // No channel yet.
        assert_eq!(
            texts(a.handle_binary(user_frame(true, b"{}"))),
            [
                r#"["1","2","realtime:room","phx_reply",{"response":{"reason":"unmatched topic"},"status":"error"}]"#
            ]
        );
        run(
            &mut a,
            &join(&json!({"broadcast": {"ack": true, "self": true}})),
        );
        run(&mut b, JOIN);

        // A stale join ref is dropped.
        let mut stale = user_push("2", "realtime:room", "e", true, b"{}");
        stale[7] = b'9';
        let stale = Serializer::decode_binary(&stale).unwrap();
        assert!(a.handle_binary(stale).is_empty());

        // `@push` broadcast: kind 2 to V2 sockets, and the ack.
        let push = |event: &str| {
            let mut bytes = vec![0, 1, 1, 13, u8::try_from(event.len()).unwrap()];
            bytes.extend_from_slice(format!("13realtime:room{event}xy").as_bytes());
            Serializer::decode_binary(&bytes).unwrap()
        };
        let relayed = Wire::Binary([&[2, 13, 9][..], b"realtime:roombroadcastxy"].concat());
        let ok3 = Wire::Text(
            r#"["1","3","realtime:room","phx_reply",{"response":{},"status":"ok"}]"#.to_owned(),
        );
        assert_eq!(
            a.handle_binary(push("broadcast")),
            [relayed.clone(), ok3.clone()]
        );
        assert_eq!(b_inbox.try_recv().unwrap(), relayed);
        // `presence` without an `event` key is a plain ok; others are ignored.
        assert_eq!(a.handle_binary(push("presence")), [ok3]);
        assert!(a.handle_binary(push("access_token")).is_empty());
        // A binary join is not served: the old channel closes first.
        let out = texts(a.handle_binary(push("phx_join")));
        assert_eq!(out[0], r#"["1","1","realtime:room","phx_close",{}]"#);
        assert!(out[1].contains("MEGABASE_NOT_IMPLEMENTED"), "{}", out[1]);

        assert_eq!(
            texts(b.handle_binary(push("phx_leave"))),
            [
                r#"["1","3","realtime:room","phx_reply",{"response":{},"status":"ok"}]"#,
                r#"["1","1","realtime:room","phx_close",{}]"#
            ]
        );
    }

    #[test]
    fn v1_frames() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut session, _inbox) = open(&jwt, &hub, Serializer::V1);
        let frame = Serializer::V1
            .decode(
                r#"{"topic":"realtime:room","event":"phx_join","payload":{"config":{}},"ref":"1"}"#,
            )
            .unwrap();
        let out = texts(session.handle(frame, NOW * 1000));
        assert_eq!(
            out,
            [
                r#"{"event":"phx_reply","payload":{"response":{"postgres_changes":[]},"status":"ok"},"ref":"1","topic":"realtime:room"}"#
            ]
        );
        assert!(Serializer::V1
            .decode(r#"{"topic":"x","event":"y"}"#)
            .is_none());
    }

    #[test]
    fn malformed_frames_decode_to_none() {
        for text in [
            "",
            "{",
            "[]",
            r#"[1,2,3,4]"#,
            r#"[null,null,1,"e",{}]"#,
            r#"{"a":1}"#,
        ] {
            assert!(Serializer::V2.decode(text).is_none(), "{text}");
        }
    }
}
