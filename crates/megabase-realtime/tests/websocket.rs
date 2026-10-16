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

#[tokio::test]
async fn supabase_js_join_heartbeat_leave() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router(state())).await.unwrap();
    });

    let url = format!(
        "ws://{addr}/realtime/v1/websocket?apikey={}&vsn=2.0.0",
        anon_key()
    );
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut socket, response) = tokio_tungstenite::client_async(url, stream).await.unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);

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
