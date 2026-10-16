//! `GET /realtime/v1/websocket`: the Phoenix socket handshake and frame loop.

// Ported from supabase/realtime lib/realtime_web/endpoint.ex and
// lib/realtime_web/channels/user_socket.ex (Apache-2.0), pin v2.143.3.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use megabase_core::MegabaseNotImplemented;
use tokio::sync::mpsc::unbounded_channel;

use crate::protocol::{Serializer, Session};
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
    let (outbox, mut inbox) = unbounded_channel();
    let mut session = Session::new(jwt, &state.hub, serializer, token, outbox);
    loop {
        let replies = tokio::select! {
            // Pushes from other channels (broadcasts, presence diffs).
            Some(text) = inbox.recv() => vec![text],
            message = socket.recv() => {
                let text = match message {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                    // Binary user broadcasts are not served; pings are answered
                    // by the WebSocket layer.
                    Some(Ok(_)) => continue,
                };
                let Some(frame) = serializer.decode(&text) else {
                    continue;
                };
                session
                    .handle(frame, unix_now_ms())
                    .into_iter()
                    .map(|reply| serializer.encode(reply))
                    .collect()
            }
        };
        for text in replies {
            if socket.send(Message::Text(text)).await.is_err() {
                return;
            }
        }
    }
}
