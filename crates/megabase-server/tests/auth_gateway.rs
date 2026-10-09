use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use megabase_server::create_router;
use serde_json::Value;
use tower::ServiceExt;

async fn send(method: &str, path: &str, body: Option<&str>) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let request = builder
        .body(match body {
            Some(body) => Body::from(body.to_string()),
            None => Body::empty(),
        })
        .unwrap();
    let response = create_router().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, json)
}

#[tokio::test]
async fn auth_prefix_reaches_health_settings_and_stubs() {
    let (status, body) = send("GET", "/auth/v1/health", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "GoTrue");
    assert_eq!(body["version"], "v2.197.0");

    let (status, body) = send("GET", "/auth/v1/settings", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mailer_autoconfirm"], true);
    assert_eq!(body["external"]["email"], true);

    let (status, body) = send("GET", "/auth/v1/reauthenticate", None).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(body["unit"], "auth:route:GET /auth/v1/reauthenticate");

    let (status, body) = send(
        "POST",
        "/auth/v1/signup",
        Some(r#"{"email":"a@example.com","password":"123"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error_code"], "weak_password");
}
