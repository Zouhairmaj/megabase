//! Phoenix channel protocol over the Realtime socket: frames, heartbeat,
//! `phx_join` on `realtime:*` topics, and `phx_leave`.

// Ported from supabase/realtime lib/realtime_web/channels/realtime_channel.ex,
// lib/realtime_web/socket/v2_serializer.ex and lib/realtime_web/socket.ex (Apache-2.0),
// pin v2.143.3. Socket-level handling follows Phoenix 1.7 `Phoenix.Socket`, which
// Realtime uses unchanged.

use std::collections::HashMap;

use megabase_core::{Hs256, MegabaseNotImplemented};
use serde_json::{json, Map, Value};

use crate::token::{authorize_conn, TokenError};
use crate::COMPONENT;

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

    /// `Phoenix.Socket` `encode_close/3`: `ref` is the join ref.
    fn close(join_ref: &Value, topic: &str) -> Self {
        Self {
            join_ref: join_ref.clone(),
            reference: join_ref.clone(),
            topic: topic.to_owned(),
            event: "phx_close".to_owned(),
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
}

/// Per-connection channel state.
pub(crate) struct Session<'a> {
    jwt: &'a Hs256,
    /// The socket's own token (`tenant_token`).
    socket_token: String,
    /// Joined topic -> its join ref.
    channels: HashMap<String, Value>,
}

impl<'a> Session<'a> {
    pub(crate) fn new(jwt: &'a Hs256, socket_token: String) -> Self {
        Self {
            jwt,
            socket_token,
            channels: HashMap::new(),
        }
    }

    /// Frames to send back for one client frame, at Unix time `now`.
    pub(crate) fn handle(&mut self, frame: Frame, now: i64) -> Vec<Frame> {
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
                out.push(Frame::close(current, &frame.topic));
                self.channels.remove(&frame.topic);
            } else if !current.is_null() && *current != frame.join_ref {
                // The client raced a server close; Phoenix drops the message.
                return out;
            } else {
                let join_ref = current.clone();
                out.extend(self.channel_event(join_ref, frame));
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
            Ok(response) => {
                self.channels
                    .insert(frame.topic.clone(), frame.join_ref.clone());
                out.push(Frame::reply(
                    frame.join_ref,
                    frame.reference,
                    &frame.topic,
                    "ok",
                    response,
                ));
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

    /// An event on a joined channel.
    fn channel_event(&mut self, join_ref: Value, frame: Frame) -> Vec<Frame> {
        let unit = match frame.event.as_str() {
            "phx_leave" => {
                self.channels.remove(&frame.topic);
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
            "broadcast" => "realtime:client-event:broadcast",
            "presence" => "realtime:client-event:presence",
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

    /// `RealtimeChannel.join/3` for a public channel with no extensions.
    fn join(&self, topic: &str, params: &Value, now: i64) -> Result<Value, Value> {
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
        } else if flag(&["presence", "enabled"]) {
            Some("realtime:server-event:presence_state")
        } else {
            None
        };
        match unit {
            Some(unit) => Err(not_implemented(unit)),
            None => Ok(json!({ "postgres_changes": [] })),
        }
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

    fn run(session: &mut Session<'_>, text: &str) -> Vec<String> {
        session
            .handle(frame(text), NOW)
            .into_iter()
            .map(|f| Serializer::V2.encode(f))
            .collect()
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
        let mut session = Session::new(&jwt, anon());
        assert_eq!(
            run(&mut session, r#"[null,"7","phoenix","heartbeat",{}]"#),
            [r#"[null,"7","phoenix","phx_reply",{"response":{},"status":"ok"}]"#]
        );
    }

    #[test]
    fn join_then_leave() {
        let jwt = jwt();
        let mut session = Session::new(&jwt, anon());
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
        let mut session = Session::new(&jwt, anon());
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
        let mut session = Session::new(&jwt, anon());
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
        let mut session = Session::new(&jwt, anon());
        let out = run(
            &mut session,
            r#"["1","1","realtime:","phx_join",{"config":{}}]"#,
        );
        assert!(out[0].contains("TopicNameRequired: You must provide a topic name"));
    }

    #[test]
    fn join_token_errors() {
        let jwt = jwt();
        let mut session = Session::new(&jwt, anon());
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
        let mut session = Session::new(&jwt, anon());
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
                json!({"config": {"presence": {"enabled": true}}}),
                "realtime:server-event:presence_state",
            ),
            (
                json!({"config": {"postgres_changes": [{"event": "*", "schema": "public"}]}}),
                "realtime:server-event:postgres_changes",
            ),
        ] {
            let mut session = Session::new(&jwt, anon());
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
        let mut session = Session::new(&jwt, anon());
        run(&mut session, JOIN);
        let out = run(
            &mut session,
            r#"["1","2","realtime:room","broadcast",{"type":"broadcast","event":"e","payload":{}}]"#,
        );
        assert!(out[0].contains("realtime:client-event:broadcast"));
        assert!(run(&mut session, r#"["1","3","realtime:room","whatever",{}]"#).is_empty());
    }

    #[test]
    fn v1_frames() {
        let jwt = jwt();
        let mut session = Session::new(&jwt, anon());
        let frame = Serializer::V1
            .decode(
                r#"{"topic":"realtime:room","event":"phx_join","payload":{"config":{}},"ref":"1"}"#,
            )
            .unwrap();
        let out: Vec<String> = session
            .handle(frame, NOW)
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
