//! Phoenix channel protocol over the Realtime socket: frames, heartbeat,
//! `phx_join` on `realtime:*` topics, public broadcast, presence and
//! `phx_leave`.

// Ported from supabase/realtime lib/realtime_web/channels/realtime_channel.ex,
// lib/realtime_web/channels/realtime_channel/broadcast_handler.ex,
// lib/realtime_web/channels/realtime_channel/presence_handler.ex,
// lib/realtime_web/socket/v2_serializer.ex and lib/realtime_web/socket.ex (Apache-2.0),
// pin v2.143.3. Socket-level handling follows Phoenix 1.7 `Phoenix.Socket`, which
// Realtime uses unchanged.

use std::collections::HashMap;

use megabase_core::{Hs256, MegabaseNotImplemented};
use serde_json::{json, Map, Value};

use crate::hub::{ConnId, Hub, Outbox};
use crate::term::{external_size, uuid1, MAX_PAYLOAD_SIZE};
use crate::token::{authorize_conn, TokenError};
use crate::COMPONENT;

/// `CLIENT_PRESENCE_MAX_CALLS` default: presence events per window and channel.
const CLIENT_PRESENCE_MAX_CALLS: u32 = 5;
/// `CLIENT_PRESENCE_WINDOW_MS` default.
const CLIENT_PRESENCE_WINDOW_MS: i64 = 30_000;

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
    fn close(join_ref: &Value, topic: &str) -> Self {
        Self::exit(join_ref, topic, "phx_close")
    }

    /// `Phoenix.Socket` `encode_on_exit/4` for a crashed channel.
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

impl Serializer {
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
}

/// One joined channel (the channel process's assigns).
struct Channel {
    join_ref: Value,
    config: JoinConfig,
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
}

impl Drop for Session<'_> {
    /// The transport went away: every channel stops, which unsubscribes it
    /// and untracks its presence.
    fn drop(&mut self) {
        for topic in self.channels.keys() {
            self.hub.unsubscribe(topic, self.conn);
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
        }
    }

    /// Frames to send back for one client frame, at Unix time `now_ms` in
    /// milliseconds. Pushes from other channels arrive on the outbox.
    pub(crate) fn handle(&mut self, frame: Frame, now_ms: i64) -> Vec<Frame> {
        let now = now_ms.div_euclid(1000);
        if frame.topic == "phoenix" && frame.event == "heartbeat" {
            return vec![Frame::reply(
                Value::Null,
                frame.reference,
                "phoenix",
                "ok",
                json!({}),
            )];
        }
        let mut out = Vec::new();
        if let Some(current) = self.channels.get(&frame.topic) {
            if frame.event == "phx_join" {
                // Duplicate join: Phoenix shuts the existing channel down first.
                out.push(Frame::close(&current.join_ref, &frame.topic));
                self.stop(&frame.topic);
            } else if !current.join_ref.is_null() && current.join_ref != frame.join_ref {
                // The client raced a server close; Phoenix drops the message.
                return out;
            } else {
                out.extend(self.channel_event(frame, now_ms));
                return out;
            }
        }
        if frame.event != "phx_join" || !frame.topic.starts_with("realtime:") {
            out.push(Frame::reply(
                frame.join_ref,
                frame.reference,
                &frame.topic,
                "error",
                json!({ "reason": "unmatched topic" }),
            ));
            return out;
        }
        match self.join(&frame.topic, &frame.payload, now) {
            Ok(config) => {
                let presence_enabled = config.presence_enabled;
                self.hub.subscribe(
                    &frame.topic,
                    self.conn,
                    self.serializer,
                    self.outbox.clone(),
                );
                self.channels.insert(
                    frame.topic.clone(),
                    Channel {
                        join_ref: frame.join_ref.clone(),
                        config,
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
                    json!({ "postgres_changes": [] }),
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

    /// The channel on `topic` stopped: forget it, unsubscribe it and untrack
    /// its presence.
    fn stop(&mut self, topic: &str) {
        self.channels.remove(topic);
        self.hub.unsubscribe(topic, self.conn);
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
    fn channel_event(&mut self, frame: Frame, now_ms: i64) -> Vec<Frame> {
        let Some(join_ref) = self.channels.get(&frame.topic).map(|c| c.join_ref.clone()) else {
            return Vec::new();
        };
        let unit = match frame.event.as_str() {
            "phx_leave" => {
                self.stop(&frame.topic);
                return vec![
                    Frame::reply(
                        join_ref.clone(),
                        frame.reference,
                        &frame.topic,
                        "ok",
                        json!({}),
                    ),
                    Frame::close(&join_ref, &frame.topic),
                ];
            }
            "broadcast" => return self.broadcast(join_ref, frame),
            "presence" => return self.presence(&join_ref, &frame, now_ms),
            "access_token" => "realtime:client-event:access_token",
            // `handle_in/3` catch-all: logged, no reply.
            _ => return Vec::new(),
        };
        vec![Frame::reply(
            join_ref,
            frame.reference,
            &frame.topic,
            "error",
            not_implemented(unit),
        )]
    }

    /// `BroadcastHandler.handle/3` on a public channel: fan out to the other
    /// subscribers (and this one with `self`), reply only with `ack`.
    // megabase:unit realtime:client-event:broadcast
    fn broadcast(&mut self, join_ref: Value, frame: Frame) -> Vec<Frame> {
        let Some(channel) = self.channels.get(&frame.topic) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let (status, response) = if external_size(&frame.payload) > MAX_PAYLOAD_SIZE {
            ("error", json!({ "error": "payload_size_exceeded" }))
        } else {
            self.hub.broadcast(&frame.topic, self.conn, &frame.payload);
            if channel.config.self_broadcast
                && (self.serializer == Serializer::V1 || frame.payload.is_object())
            {
                out.push(Frame::push(
                    Value::Null,
                    &frame.topic,
                    "broadcast",
                    &frame.payload,
                ));
            }
            ("ok", json!({}))
        };
        if channel.config.ack_broadcast {
            out.push(Frame::reply(
                join_ref,
                frame.reference,
                &frame.topic,
                status,
                response,
            ));
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

    /// `shutdown_response/2`: a `system` error push, then the channel stops.
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
    fn join(&self, topic: &str, params: &Value, now: i64) -> Result<JoinConfig, Value> {
        let sub_topic = &topic["realtime:".len()..];
        if sub_topic.is_empty() {
            return Err(reason("TopicNameRequired: You must provide a topic name"));
        }
        let empty = Map::new();
        let params = params.as_object().unwrap_or(&empty);
        self.confirm_token(params, now)?;

        let config = params.get("config");
        let field = |path: &[&str]| -> Option<&Value> {
            path.iter()
                .try_fold(config?, |value, key| value.as_object()?.get(*key))
        };
        let flag = |path: &[&str]| field(path).is_some_and(flexible_true);
        let unit = if config.is_none() {
            // Old API: the topic itself names a Postgres subscription.
            Some("realtime:server-event:postgres_changes")
        } else if flag(&["private"]) {
            Some("realtime:protocol-event:phx_join")
        } else if field(&["broadcast", "replay"]).is_some_and(|v| !v.is_null()) {
            Some("realtime:server-event:broadcast")
        } else if field(&["postgres_changes"])
            .and_then(Value::as_array)
            .is_some_and(|changes| changes.iter().any(|c| !c.is_null()))
        {
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
        })
    }

    /// `assign_access_token/2` then `confirm_token/1`.
    fn confirm_token(&self, params: &Map<String, Value>, now: i64) -> Result<(), Value> {
        let string = |key: &str| params.get(key).and_then(Value::as_str);
        let sb_key = params
            .get("access_token")
            .filter(|v| !v.is_null())
            .or_else(|| params.get("user_token"))
            .and_then(Value::as_str)
            .is_some_and(|token| token.starts_with("sb_"));
        let token = if sb_key {
            self.socket_token.as_str()
        } else {
            string("user_token")
                .or_else(|| string("access_token"))
                .unwrap_or(&self.socket_token)
        };
        let claims = authorize_conn(self.jwt, token, now).map_err(|error| match error {
            TokenError::Expired(seconds) => reason(&format!(
                "InvalidJWTToken: Token has expired {seconds} seconds ago"
            )),
            TokenError::MissingClaims => reason(
                "InvalidJWTToken: Fields `role` and `exp` are required in JWT",
            ),
            TokenError::Malformed => {
                reason("MalformedJWT: The token provided is not a valid JWT")
            }
            TokenError::Signature => {
                reason("JwtSignatureError: Failed to validate JWT signature")
            }
            TokenError::Signer => reason(
                "JwtSignerError: Failed to generate JWT signer, check your JWT secret or JWKS configuration",
            ),
            TokenError::UnsupportedAlgorithm => {
                not_implemented("realtime:protocol-event:phx_join")
            }
            TokenError::Other => reason("Unknown Error on Channel"),
        })?;
        match claims.get("exp").and_then(Value::as_i64) {
            Some(exp) if exp > now => Ok(()),
            Some(_) => Err(reason("InvalidJWTToken: Token expiration time is invalid")),
            None => Err(reason("Unknown Error on Channel")),
        }
    }
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
    ) -> (Session<'a>, Receiver<String>) {
        let (outbox, inbox) = Outbox::new(OUTBOX_CAPACITY);
        (Session::new(jwt, hub, serializer, anon(), outbox), inbox)
    }

    fn run_at(session: &mut Session<'_>, text: &str, now_ms: i64) -> Vec<String> {
        session
            .handle(frame(text), now_ms)
            .into_iter()
            .map(|f| Serializer::V2.encode(f))
            .collect()
    }

    fn run(session: &mut Session<'_>, text: &str) -> Vec<String> {
        run_at(session, text, NOW * 1000)
    }

    fn drain(inbox: &mut Receiver<String>) -> Vec<Value> {
        let mut out = Vec::new();
        while let Ok(text) = inbox.try_recv() {
            out.push(serde_json::from_str(&text).unwrap());
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
    fn channel_events() {
        let jwt = jwt();
        let hub = Hub::default();
        let (mut session, _inbox) = open(&jwt, &hub, Serializer::V2);
        run(&mut session, JOIN);
        let out = run(
            &mut session,
            r#"["1","2","realtime:room","access_token",{"access_token":"x"}]"#,
        );
        assert!(out[0].contains("realtime:client-event:access_token"));
        assert!(run(&mut session, r#"["1","3","realtime:room","whatever",{}]"#).is_empty());
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
                .join("realtime:room", &json!({ "config": config }), NOW)
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
        let out: Vec<String> = session
            .handle(frame, NOW * 1000)
            .into_iter()
            .map(|f| Serializer::V1.encode(f))
            .collect();
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
