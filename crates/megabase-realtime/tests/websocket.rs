//! `GET /realtime/v1/websocket` over a real TCP connection.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use futures_util::{SinkExt, StreamExt};
use megabase_core::Hs256;
use megabase_realtime::{router, RealtimeState};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

const SECRET: &str = "super-secret-jwt-token-with-at-least-32-characters-long";

fn jwt() -> Hs256 {
    Hs256::new(SECRET).unwrap()
}

fn anon_key() -> String {
    jwt()
        .sign(&json!({"role": "anon", "iss": "supabase", "iat": 1_700_000_000, "exp": 4_000_000_000_i64}))
        .unwrap()
        .as_str()
        .to_owned()
}

fn state() -> RealtimeState {
    RealtimeState::new(Some(jwt()))
}

async fn get(state: RealtimeState, uri: &str) -> (StatusCode, String) {
    let response = router(state)
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn handshake_errors_match_user_socket() {
    let (status, body) = get(state(), "/realtime/v1/websocket?vsn=2.0.0").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body, r#"{"error":"API key is missing"}"#);

    let (status, body) = get(state(), "/realtime/v1/websocket?apikey=abc&vsn=2.0.0").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body, r#"{"error":"The token provided is not a valid JWT"}"#);

    let expired = jwt().sign(&json!({"role": "anon", "exp": 1})).unwrap();
    let uri = format!("/realtime/v1/websocket?apikey={}", expired.as_str());
    let (status, body) = get(state(), &uri).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body, r#"{"error":"Token has expired"}"#);

    let uri = format!("/realtime/v1/websocket?apikey={}&vsn=9.0.0", anon_key());
    let (status, body) = get(state(), &uri).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body, "");
}

#[tokio::test]
async fn not_upgrade_or_no_secret_is_501() {
    let uri = format!("/realtime/v1/websocket?apikey={}", anon_key());
    let (status, body) = get(state(), &uri).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert!(body.contains("realtime:route:GET /realtime/v1/websocket"));

    let (status, _) = get(RealtimeState::default(), &uri).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
}

async fn next_json<S>(socket: &mut S) -> Value
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match socket.next().await.unwrap().unwrap() {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("unexpected {other:?}"),
        }
    }
}

type Socket = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

async fn serve() -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router(state())).await.unwrap();
    });
    addr
}

async fn connect(addr: std::net::SocketAddr) -> Socket {
    let url = format!(
        "ws://{addr}/realtime/v1/websocket?apikey={}&vsn=2.0.0",
        anon_key()
    );
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (socket, response) = tokio_tungstenite::client_async(url, stream).await.unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    socket
}

async fn send(socket: &mut Socket, frame: Value) {
    socket.send(Message::Text(frame.to_string())).await.unwrap();
}

/// What realtime-js sends for `supabase.channel('room', { config })`.
async fn join(socket: &mut Socket, config: Value) -> Value {
    send(
        socket,
        json!(["1", "1", "realtime:room", "phx_join",
            {"config": config, "access_token": anon_key()}]),
    )
    .await;
    next_json(socket).await
}

/// realtime-js 2.x `channel.send({type: 'broadcast', event: 'cursor', payload: {x: 1}})`
/// on a `vsn=2.0.0` socket: a binary `userBroadcastPush` frame.
#[tokio::test]
async fn supabase_js_binary_broadcast() {
    let addr = serve().await;
    let mut alice = connect(addr).await;
    let mut bob = connect(addr).await;
    let config = json!({"broadcast": {"ack": true, "self": false}});
    assert_eq!(join(&mut alice, config).await[4]["status"], "ok");
    assert_eq!(join(&mut bob, json!({})).await[4]["status"], "ok");

    let mut push = vec![3, 1, 1, 13, 6, 0, 1];
    push.extend_from_slice(br#"12realtime:roomcursor{"x":1}"#);
    alice.send(Message::Binary(push)).await.unwrap();
    assert_eq!(
        next_json(&mut alice).await,
        json!(["1", "2", "realtime:room", "phx_reply", {"status": "ok", "response": {}}])
    );
    let mut relayed = vec![4, 13, 6, 0, 1];
    relayed.extend_from_slice(br#"realtime:roomcursor{"x":1}"#);
    loop {
        match bob.next().await.unwrap().unwrap() {
            Message::Binary(bytes) => break assert_eq!(bytes, relayed),
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[tokio::test]
async fn broadcast_between_two_sockets() {
    let addr = serve().await;
    let mut alice = connect(addr).await;
    let mut bob = connect(addr).await;
    let config = json!({"broadcast": {"ack": true, "self": true}, "presence": {"key": ""}});
    assert_eq!(join(&mut alice, config).await[4]["status"], "ok");
    assert_eq!(join(&mut bob, json!({})).await[4]["status"], "ok");

    let message = json!({"type": "broadcast", "event": "cursor", "payload": {"x": 1}});
    send(
        &mut alice,
        json!(["1", "2", "realtime:room", "broadcast", message]),
    )
    .await;
    let pushed = json!([null, null, "realtime:room", "broadcast", message]);
    // `self` echoes to the sender, then `ack` replies.
    assert_eq!(next_json(&mut alice).await, pushed);
    assert_eq!(
        next_json(&mut alice).await,
        json!(["1", "2", "realtime:room", "phx_reply", {"status": "ok", "response": {}}])
    );
    assert_eq!(next_json(&mut bob).await, pushed);
}

#[tokio::test]
async fn presence_between_two_sockets() {
    let addr = serve().await;
    let mut alice = connect(addr).await;
    let mut bob = connect(addr).await;
    let presence = |key: &str| json!({"presence": {"enabled": true, "key": key}});
    join(&mut alice, presence("alice")).await;
    assert_eq!(
        next_json(&mut alice).await,
        json!(["1", null, "realtime:room", "presence_state", {}])
    );

    send(
        &mut alice,
        json!(["1", "2", "realtime:room", "presence",
            {"type": "presence", "event": "track", "payload": {"status": "online"}}]),
    )
    .await;
    assert_eq!(next_json(&mut alice).await[4]["status"], "ok");
    let diff = next_json(&mut alice).await;
    assert_eq!(diff[3], "presence_diff");
    assert_eq!(diff[4]["joins"]["alice"]["metas"][0]["status"], "online");

    join(&mut bob, presence("bob")).await;
    let state = next_json(&mut bob).await;
    assert_eq!(state[3], "presence_state");
    assert_eq!(state[4]["alice"]["metas"][0]["status"], "online");

    // Alice disconnects: bob gets her leave.
    alice.close(None).await.unwrap();
    let diff = next_json(&mut bob).await;
    assert_eq!(diff[3], "presence_diff");
    assert_eq!(diff[4]["joins"], json!({}));
    assert_eq!(diff[4]["leaves"]["alice"]["metas"][0]["status"], "online");
}

#[tokio::test]
async fn supabase_js_join_heartbeat_leave() {
    let addr = serve().await;
    let mut socket = connect(addr).await;

    // What realtime-js sends for `supabase.channel('room').subscribe()`.
    let join = json!(["1", "1", "realtime:room", "phx_join", {
        "config": {
            "broadcast": {"ack": false, "self": false},
            "presence": {"key": "", "enabled": false},
            "postgres_changes": [],
            "private": false
        },
        "access_token": anon_key()
    }]);
    socket.send(Message::Text(join.to_string())).await.unwrap();
    assert_eq!(
        next_json(&mut socket).await,
        json!(["1", "1", "realtime:room", "phx_reply",
            {"status": "ok", "response": {"postgres_changes": []}}])
    );

    // Malformed frames are ignored; the socket stays open.
    socket.send(Message::Text("not json".into())).await.unwrap();

    socket
        .send(Message::Text(
            r#"[null,"2","phoenix","heartbeat",{}]"#.into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        next_json(&mut socket).await,
        json!([null, "2", "phoenix", "phx_reply", {"status": "ok", "response": {}}])
    );

    socket
        .send(Message::Text(
            r#"["1","3","realtime:room","phx_leave",{}]"#.into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        next_json(&mut socket).await,
        json!(["1", "3", "realtime:room", "phx_reply", {"status": "ok", "response": {}}])
    );
    assert_eq!(
        next_json(&mut socket).await,
        json!(["1", "1", "realtime:room", "phx_close", {}])
    );
    socket.close(None).await.unwrap();
}

#[tokio::test]
async fn refreshed_token_expiry_closes_the_channel() {
    let addr = serve().await;
    let mut socket = connect(addr).await;
    assert_eq!(join(&mut socket, json!({})).await[4]["status"], "ok");

    // realtime-js `setAuth`: a token that expires within two seconds.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let short = jwt()
        .sign(&json!({"role": "authenticated", "exp": now + 2}))
        .unwrap();
    send(
        &mut socket,
        json!(["1", "2", "realtime:room", "access_token", {"access_token": short.as_str()}]),
    )
    .await;

    // No reply to the refresh; the `:confirm_token` timer shuts the channel.
    let system = next_json(&mut socket).await;
    assert_eq!(system[0], "1");
    assert_eq!(system[1], Value::Null);
    assert_eq!(system[3], "system");
    assert_eq!(system[4]["status"], "error");
    let message = system[4]["message"].as_str().unwrap();
    assert!(message.starts_with("Token has expired "), "{message}");
    assert_eq!(
        next_json(&mut socket).await,
        json!(["1", "1", "realtime:room", "phx_close", {}])
    );
}
