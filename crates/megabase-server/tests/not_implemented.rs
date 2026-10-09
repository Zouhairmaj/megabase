use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use megabase_server::{create_router, GATEWAY_ROUTES, HEALTH_PATH};
use serde_json::Value;
use tower::ServiceExt;

async fn send(method: &str, path: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let response = create_router().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, body)
}

fn assert_not_implemented(body: &Value, component: &str, unit: &str) {
    assert_eq!(body["code"], "MEGABASE_NOT_IMPLEMENTED", "{body}");
    assert_eq!(body["component"], component, "{unit}");
    assert_eq!(body["unit"], unit);
    assert!(body["message"].as_str().is_some_and(|m| !m.is_empty()));
    assert_eq!(
        body.as_object().unwrap().len(),
        4,
        "unexpected fields: {body}"
    );
}

#[tokio::test]
async fn every_method_on_every_route_is_501() {
    let methods = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
    for (prefix, component) in GATEWAY_ROUTES {
        for suffix in ["", "x", "/some/deep/path"] {
            for method in methods {
                let path = format!("{prefix}{suffix}");
                let (status, body) = send(method, &path).await;
                assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{method} {path}");
                if method != "HEAD" {
                    assert_not_implemented(&body, component, &format!("{method} {path}"));
                }
            }
        }
    }
}

#[tokio::test]
async fn unit_keeps_full_path_without_query() {
    let (status, body) = send("PATCH", "/rest/v1/todos?id=eq.1").await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_not_implemented(&body, "rest", "PATCH /rest/v1/todos");
}

#[tokio::test]
async fn paths_outside_kong_routes_belong_to_studio() {
    // `/auth/v1` and `/rest/v1` lack the trailing slash Kong's routes require.
    for path in [
        "/",
        "/auth/v1",
        "/rest/v1",
        "/pooler/api/tenants",
        "/api/platform/profile",
    ] {
        let (status, body) = send("GET", path).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{path}");
        assert_not_implemented(&body, "studio", &format!("GET {path}"));
    }
}

#[tokio::test]
async fn health_is_200() {
    let (status, body) = send("GET", HEALTH_PATH).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
}
