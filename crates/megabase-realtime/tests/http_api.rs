//! `POST /realtime/v1/api/broadcast` and `/broadcast/{topic}/events/{event}`.

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use megabase_core::Hs256;
use megabase_realtime::{router, RealtimeState};
use serde_json::{json, Value};
use tower::ServiceExt;

const SECRET: &str = "super-secret-jwt-token-with-at-least-32-characters-long";

fn jwt() -> Hs256 {
    Hs256::new(SECRET).unwrap()
}

fn token(claims: &Value) -> String {
    jwt().sign(claims).unwrap().as_str().to_owned()
}

fn anon_key() -> String {
    token(&json!({"role": "anon", "iss": "supabase", "exp": 4_000_000_000_i64}))
}

struct Reply {
    status: StatusCode,
    content_type: Option<String>,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
    }
}

async fn post(
    uri: &str,
    content_type: Option<&str>,
    auth: Option<&str>,
    body: impl Into<Body>,
) -> Reply {
    post_with(
        RealtimeState::new(Some(jwt())),
        uri,
        content_type,
        auth,
        body,
    )
    .await
}

async fn post_with(
    state: RealtimeState,
    uri: &str,
    content_type: Option<&str>,
    auth: Option<&str>,
    body: impl Into<Body>,
) -> Reply {
    let mut request = Request::post(uri);
    if let Some(content_type) = content_type {
        request = request.header(header::CONTENT_TYPE, content_type);
    }
    if let Some(auth) = auth {
        request = request.header("apikey", auth);
    }
    let response = router(state)
        .oneshot(request.body(body.into()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_owned());
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    Reply {
        status,
        content_type,
        body: String::from_utf8(body.to_vec()).unwrap(),
    }
}

const BATCH: &str = "/realtime/v1/api/broadcast";
const SINGLE: &str = "/realtime/v1/api/broadcast/room:1/events/message";
const JSON: Option<&str> = Some("application/json");

#[tokio::test]
async fn batch_accepts_public_messages() {
    let key = anon_key();
    let body = json!({"messages": [
        {"topic": "a", "event": "e", "payload": {"x": 1}},
        {"topic": "b", "event": "e", "payload": {}, "id": "m1", "private": false},
    ]});
    let reply = post(BATCH, JSON, Some(&key), body.to_string()).await;
    assert_eq!(reply.status, StatusCode::ACCEPTED);
    assert_eq!(reply.body, "");
}

#[tokio::test]
async fn batch_reports_changeset_errors_per_message() {
    let key = anon_key();
    let body = json!({"messages": [
        {"topic": "topic"},
        {"payload": {"data": "data"}},
        {"topic": "topic", "payload": {"data": "data"}, "event": "event"},
        {"topic": 1, "payload": [], "event": "  ", "private": "maybe"},
    ]});
    let reply = post(BATCH, JSON, Some(&key), body.to_string()).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.content_type.as_deref(),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(
        reply.json(),
        json!({"errors": {"messages": [
            {"payload": ["can't be blank"], "event": ["can't be blank"]},
            {"topic": ["can't be blank"], "event": ["can't be blank"]},
            {},
            {"topic": ["is invalid"], "payload": ["is invalid"], "event": ["can't be blank"], "private": ["is invalid"]},
        ]}})
    );
}

#[tokio::test]
async fn batch_requires_messages() {
    let key = anon_key();
    for body in ["{}", r#"{"messages": []}"#, "", "[1]"] {
        let reply = post(BATCH, JSON, Some(&key), body).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(
            reply.json(),
            json!({"errors": {"messages": ["can't be blank"]}})
        );
    }
    for body in [r#"{"messages": "x"}"#, r#"{"messages": [1]}"#] {
        let reply = post(BATCH, JSON, Some(&key), body).await;
        assert_eq!(
            reply.json(),
            json!({"errors": {"messages": ["is invalid"]}})
        );
    }
    // Without a parsed body the query string is the only source.
    let reply = post(&format!("{BATCH}?messages=x"), None, Some(&key), "").await;
    assert_eq!(
        reply.json(),
        json!({"errors": {"messages": ["is invalid"]}})
    );
}

#[tokio::test]
async fn batch_limits_payload_size_and_batch_length() {
    let key = anon_key();
    let big = "x".repeat(3_000_500);
    let body = json!({"messages": [{"topic": "t", "event": "e", "payload": {"d": big}}]});
    let reply = post(BATCH, JSON, Some(&key), body.to_string()).await;
    assert_eq!(
        reply.json(),
        json!({"errors": {"messages": [{"payload": ["Payload size exceeds tenant limit"]}]}})
    );

    let messages: Vec<Value> = (0..101)
        .map(|_| json!({"topic": "t", "event": "e", "payload": {}}))
        .collect();
    let reply = post(
        BATCH,
        JSON,
        Some(&key),
        json!({ "messages": messages }).to_string(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        reply.json(),
        json!({"message": "Too many messages to broadcast, please reduce the batch size"})
    );
}

#[tokio::test]
async fn batch_private_messages_are_not_implemented() {
    let key = anon_key();
    let body = json!({"messages": [{"topic": "t", "event": "e", "payload": {}, "private": true}]});
    let reply = post(BATCH, JSON, Some(&key), body.to_string()).await;
    assert_eq!(reply.status, StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn auth_tenant_rejects_bad_tokens() {
    let expired = token(&json!({"role": "anon", "exp": 1}));
    let no_role = token(&json!({"exp": 4_000_000_000_i64}));
    let body = json!({"messages": [{"topic": "t", "event": "e", "payload": {}}]}).to_string();
    for auth in [
        None,
        Some("garbage"),
        Some(expired.as_str()),
        Some(no_role.as_str()),
    ] {
        for uri in [BATCH, SINGLE] {
            let reply = post(uri, JSON, auth, body.clone()).await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{uri} {auth:?}");
            assert_eq!(reply.json(), json!({"message": "Unauthorized"}));
        }
    }
}

#[tokio::test]
async fn bearer_header_wins_over_apikey() {
    let request = Request::post(SINGLE)
        .header(header::CONTENT_TYPE, "application/json")
        .header("apikey", "garbage")
        .header(header::AUTHORIZATION, format!("Bearer {}", anon_key()))
        .body(Body::from("{}"))
        .unwrap();
    let response = router(RealtimeState::new(Some(jwt())))
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn single_accepts_json_binary_and_no_content_type() {
    let key = anon_key();
    for (content_type, body) in [
        (JSON, r#"{"text": "hi"}"#),
        (Some("application/json; charset=utf-8"), "[1, 2]"),
        (Some("application/octet-stream"), "\u{1}\u{2}"),
        (Some("application/octet-stream"), ""),
        (None, "anything"),
    ] {
        let reply = post(SINGLE, content_type, Some(&key), body).await;
        assert_eq!(reply.status, StatusCode::ACCEPTED, "{content_type:?}");
        assert_eq!(reply.body, "");
    }
}

#[tokio::test]
async fn single_rejects_other_media_types_before_auth() {
    for content_type in ["text/plain", "garbage"] {
        let reply = post(SINGLE, Some(content_type), None, "x").await;
        assert_eq!(reply.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(
            reply.content_type.as_deref(),
            Some("application/json; charset=utf-8")
        );
        assert_eq!(
            reply.json(),
            json!({"error": "Unsupported Media Type. Use application/json or application/octet-stream"})
        );
    }
}

#[tokio::test]
async fn single_validates_the_message() {
    let key = anon_key();
    let reply = post(
        "/realtime/v1/api/broadcast/%20/events/%20?persist=true",
        JSON,
        Some(&key),
        "{}",
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.json(),
        json!({"errors": {
            "topic": ["can't be blank"],
            "event": ["can't be blank"],
            "persist": ["can only be used on private channels"],
        }})
    );

    let reply = post(SINGLE, JSON, Some(&key), r#"{"_binary": null}"#).await;
    assert_eq!(
        reply.json(),
        json!({"errors": {"payload": ["can't be blank"]}})
    );

    let big = vec![0_u8; 3_000_495];
    let reply = post(SINGLE, Some("application/octet-stream"), Some(&key), big).await;
    assert_eq!(
        reply.json(),
        json!({"errors": {"payload": ["Payload size exceeds tenant limit"]}})
    );
    let fits = vec![0_u8; 3_000_494];
    let reply = post(SINGLE, Some("application/octet-stream"), Some(&key), fits).await;
    assert_eq!(reply.status, StatusCode::ACCEPTED);
}

#[tokio::test]
async fn single_private_is_not_implemented() {
    let key = anon_key();
    for (uri, body) in [
        (format!("{SINGLE}?private=true"), "{}"),
        (SINGLE.to_owned(), r#"{"private": true}"#),
    ] {
        let reply = post(&uri, JSON, Some(&key), body).await;
        assert_eq!(reply.status, StatusCode::NOT_IMPLEMENTED, "{uri} {body}");
    }
}

#[tokio::test]
async fn parser_errors_render_the_reason_phrase() {
    let reply = post(SINGLE, JSON, None, "{").await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.content_type.as_deref(),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(reply.body, "Bad Request");

    let request = Request::post(BATCH)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json")
        .body(Body::from("{"))
        .unwrap();
    let response = router(RealtimeState::new(Some(jwt())))
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&body[..], br#""Bad Request""#);

    let reply = post(BATCH, JSON, None, vec![b' '; 8_000_001]).await;
    assert_eq!(reply.status, StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn without_a_secret_both_routes_are_not_implemented() {
    for uri in [BATCH, SINGLE] {
        let reply = post_with(RealtimeState::default(), uri, JSON, None, "{}").await;
        assert_eq!(reply.status, StatusCode::NOT_IMPLEMENTED);
    }
}
