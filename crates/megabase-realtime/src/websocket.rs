//! `GET /realtime/v1/websocket`: the Phoenix socket handshake and frame loop.

// Ported from supabase/realtime lib/realtime_web/endpoint.ex and
// lib/realtime_web/channels/user_socket.ex (Apache-2.0), pin v2.143.3.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use megabase_core::MegabaseNotImplemented;
use tokio::sync::Notify;

use crate::hub::{Outbox, OUTBOX_CAPACITY};
use crate::protocol::{Serializer, Session, Wire};
use crate::token::{authorize_conn, TokenError};
use crate::{RealtimeState, COMPONENT};

pub(crate) const UNIT: &str = "realtime:route:GET /realtime/v1/websocket";

/// `endpoint.ex` `max_frame_size`.
const MAX_FRAME_SIZE: usize = 5_000_000;

fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn unix_now() -> i64 {
    unix_now_ms().div_euclid(1000)
}

/// `UserSocket.handle_error/2`: `{"error": message}` as JSON.
fn connect_error(status: StatusCode, message: &str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        serde_json::json!({ "error": message }).to_string(),
    )
        .into_response()
}

/// Phoenix `Transport` websocket `call/2` with `UserSocket.connect/3`.
// megabase:unit realtime:route:GET /realtime/v1/websocket
pub(crate) async fn websocket(
    State(state): State<Arc<RealtimeState>>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    upgrade: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Response {
    let Some(jwt) = state.jwt.as_ref() else {
        return MegabaseNotImplemented::new(COMPONENT, UNIT).into_response();
    };
    // `negotiate_serializer/2` runs before the user `connect/3`; no match is `:error`.
    let Some(serializer) = Serializer::negotiate(params.get("vsn").map(String::as_str)) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    // `access_token/2`: the `x-api-key` header Kong adds, else the `apikey` param.
    let token = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .or_else(|| params.get("apikey").cloned());
    let Some(token) = token else {
        return connect_error(StatusCode::UNAUTHORIZED, "API key is missing");
    };
    if let Err(error) = authorize_conn(jwt, &token, unix_now()) {
        return match error {
            TokenError::Expired(_) => connect_error(StatusCode::UNAUTHORIZED, "Token has expired"),
            TokenError::MissingClaims => connect_error(
                StatusCode::UNAUTHORIZED,
                "Fields `role` and `exp` are required in JWT",
            ),
            TokenError::Malformed => connect_error(
                StatusCode::UNAUTHORIZED,
                "The token provided is not a valid JWT",
            ),
            TokenError::Signature => {
                connect_error(StatusCode::UNAUTHORIZED, "Failed to validate JWT signature")
            }
            TokenError::Signer => connect_error(
                StatusCode::UNAUTHORIZED,
                "Failed to generate JWT signer, check your JWT secret or JWKS configuration",
            ),
            TokenError::UnsupportedAlgorithm => {
                MegabaseNotImplemented::new(COMPONENT, UNIT).into_response()
            }
            TokenError::Other => connect_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Error connecting to Realtime",
            ),
        };
    }
    // `TenantRateLimiters.check_tenant/1`; Megabase does not model the
    // `max_concurrent_users` check that runs before it.
    if state.hub.too_many_joins(unix_now_ms()) {
        return connect_error(StatusCode::TOO_MANY_REQUESTS, "Too many joins per second");
    }
    // Phoenix answers a failed upgrade with WebSockAdapter's 400 text, which
    // depends on the HTTP server; Megabase does not model it.
    let Ok(upgrade) = upgrade else {
        return MegabaseNotImplemented::new(COMPONENT, UNIT).into_response();
    };
    upgrade
        .max_frame_size(MAX_FRAME_SIZE)
        .max_message_size(MAX_FRAME_SIZE)
        .on_upgrade(move |socket| serve(socket, state, serializer, token))
}

async fn serve(
    mut socket: WebSocket,
    state: Arc<RealtimeState>,
    serializer: Serializer,
    token: String,
) {
    let Some(jwt) = state.jwt.as_ref() else {
        return;
    };
    let (outbox, mut inbox) = Outbox::new(OUTBOX_CAPACITY);
    let evicted = outbox.evicted();
    let mut session = Session::new(jwt, &state.hub, serializer, token, outbox)
        .with_cdc(state.cdc.as_ref().map(Arc::clone));
    loop {
        let wait = session.deadline().map(|at| {
            Duration::from_millis(u64::try_from(at.saturating_sub(unix_now_ms())).unwrap_or(0))
        });
        let timer = async move {
            match wait {
                Some(wait) => tokio::time::sleep(wait).await,
                None => std::future::pending().await,
            }
        };
        let replies = tokio::select! {
            // The hub dropped this socket for not draining its queue.
            () = evicted.notified() => break,
            // Channel timers: token re-confirmation and held refreshes.
            () = timer => session.tick(unix_now_ms()),
            // Pushes from other channels (broadcasts, presence diffs).
            Some(wire) = inbox.recv() => vec![wire],
            message = socket.recv() => match message {
                Some(Ok(message)) => match receive(&mut session, serializer, message) {
                    Some(replies) => replies,
                    None => break,
                },
                Some(Err(_)) | None => break,
            },
        };
        if !send_all(&mut socket, &evicted, replies).await {
            return;
        }
        // `Phoenix.Socket` `__info__/2` on `%Broadcast{event: "disconnect"}`:
        // `{:stop, {:shutdown, :disconnected}, state}` closes the transport.
        if session.disconnected() {
            let _closed = socket.send(Message::Close(None)).await;
            return;
        }
    }
}

/// The replies to one client message; `None` once the client closed.
/// Malformed frames are ignored.
fn receive(
    session: &mut Session<'_>,
    serializer: Serializer,
    message: Message,
) -> Option<Vec<Wire>> {
    let text = match (message, serializer) {
        (Message::Text(text), _) => text,
        (Message::Binary(bytes), Serializer::V2) => {
            return Some(
                Serializer::decode_binary(&bytes)
                    .map(|frame| session.handle_binary(frame))
                    .unwrap_or_default(),
            );
        }
        // The V1 serializer decodes JSON whatever the opcode.
        (Message::Binary(bytes), Serializer::V1) => match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => return Some(Vec::new()),
        },
        (Message::Close(_), _) => return None,
        // Pings are answered by the WebSocket layer.
        (Message::Ping(_) | Message::Pong(_), _) => return Some(Vec::new()),
    };
    Some(
        serializer
            .decode(&text)
            .map(|frame| session.handle(frame, unix_now_ms()))
            .unwrap_or_default(),
    )
}

/// Sends `replies` in order; `false` once the socket is gone or evicted.
async fn send_all(socket: &mut WebSocket, evicted: &Notify, replies: Vec<Wire>) -> bool {
    for wire in replies {
        let message = match wire {
            Wire::Text(text) => Message::Text(text),
            Wire::Binary(bytes) => Message::Binary(bytes),
        };
        // A client that stops reading blocks `send`; eviction must still win.
        tokio::select! {
            sent = socket.send(message) => {
                if sent.is_err() {
                    return false;
                }
            }
            () = evicted.notified() => return false,
        }
    }
    true
}
