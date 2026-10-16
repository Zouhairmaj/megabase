//! `/functions/v1/{name}` end to end against an in-process worker: the
//! runtime under test answers like a function that echoes its request.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use megabase_core::Hs256;
use megabase_functions::{
    router, BootError, FunctionsSettings, FunctionsState, Runtime, StartFuture, WorkerHandle,
    WorkerSpec,
};
use serde_json::{json, Value};
use tower::ServiceExt;

const SECRET: &str = "super-secret-jwt-token-with-at-least-32-characters-long";

/// Echoes method, target, `Host`, `Authorization` and body as JSON. A
/// `x-status` request header picks the status; the response always carries
/// `x-function: <name>`.
async fn echo(name: String, request: Request) -> impl IntoResponse {
    let (parts, body) = request.into_parts();
    let header = |key: &str| {
        parts
            .headers
            .get(key)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let status = header("x-status")
        .and_then(|raw| raw.parse::<u16>().ok())
        .and_then(|code| StatusCode::from_u16(code).ok())
        .unwrap_or(StatusCode::OK);
    let body = to_bytes(body, usize::MAX).await.unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-function", name.parse().unwrap());
    (
        status,
        headers,
        Json(json!({
            "method": parts.method.as_str(),
            "target": parts.uri.to_string(),
            "host": header("host"),
            "authorization": header("authorization"),
            "content_type": header("content-type"),
            "body": String::from_utf8_lossy(&body),
        })),
    )
}

#[derive(Clone, Default)]
struct EchoRuntime {
    starts: Arc<AtomicUsize>,
}

impl Runtime for EchoRuntime {
    fn start<'a>(&'a self, spec: WorkerSpec<'a>) -> StartFuture<'a> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let name = spec.name.to_owned();
        Box::pin(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let addr: SocketAddr = listener.local_addr()?;
            let app = axum::Router::new().fallback(move |request: Request| echo(name, request));
            tokio::spawn(async move { axum::serve(listener, app).await });
            Ok(WorkerHandle::external(addr))
        })
    }
}

struct FailingRuntime;

impl Runtime for FailingRuntime {
    fn start<'a>(&'a self, _spec: WorkerSpec<'a>) -> StartFuture<'a> {
        Box::pin(async { Err(BootError::Spawn(std::io::Error::other("no deno"))) })
    }
}

fn functions_dir() -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("megabase-functions-invoke-{}", std::process::id()));
    for name in ["hello", "other"] {
        std::fs::create_dir_all(dir.join(name)).unwrap();
        std::fs::write(dir.join(name).join("index.ts"), "// fixture\n").unwrap();
    }
    dir
}

fn state(verify_jwt: bool, runtime: impl Runtime) -> FunctionsState {
    FunctionsState::new(
        FunctionsSettings {
            functions_dir: functions_dir(),
            verify_jwt,
        },
        Some(Hs256::new(SECRET).unwrap()),
        runtime,
    )
}

fn anon_key() -> String {
    Hs256::new(SECRET)
        .unwrap()
        .sign(&json!({"role": "anon", "iss": "supabase"}))
        .unwrap()
        .as_str()
        .to_owned()
}

async fn send(
    state: &FunctionsState,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (StatusCode, HeaderMap, Value) {
    let mut request = Request::builder().method(method).uri(uri);
    for (key, value) in headers {
        request = request.header(*key, *value);
    }
    let response = router(state.clone())
        .oneshot(request.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, body)
}

#[tokio::test]
async fn post_forwards_method_headers_body_and_authorization() {
    let state = state(false, EchoRuntime::default());
    let auth = format!("Bearer {}", anon_key());
    let (status, headers, body) = send(
        &state,
        "POST",
        "/functions/v1/hello",
        &[
            ("authorization", &auth),
            ("content-type", "application/json"),
        ],
        r#"{"name":"x"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["x-function"], "hello");
    assert_eq!(body["method"], "POST");
    assert_eq!(body["target"], "/hello");
    assert_eq!(body["host"], "functions:9000");
    assert_eq!(body["authorization"], auth);
    assert_eq!(body["content_type"], "application/json");
    assert_eq!(body["body"], r#"{"name":"x"}"#);
}

#[tokio::test]
async fn get_forwards_sub_path_and_query_and_reuses_the_worker() {
    let runtime = EchoRuntime::default();
    let state = state(false, runtime.clone());
    for _ in 0..2 {
        let (status, _, body) = send(
            &state,
            "GET",
            "/functions/v1/hello/deep/path?a=1&b=x%20y",
            &[],
            "",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["method"], "GET");
        assert_eq!(body["target"], "/hello/deep/path?a=1&b=x%20y");
    }
    assert_eq!(runtime.starts.load(Ordering::SeqCst), 1);
    let (_, headers, _) = send(&state, "GET", "/functions/v1/other", &[], "").await;
    assert_eq!(headers["x-function"], "other");
    assert_eq!(runtime.starts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn the_function_status_and_options_requests_pass_through() {
    let state = state(true, EchoRuntime::default());
    let (status, _, body) = send(
        &state,
        "OPTIONS",
        "/functions/v1/hello",
        &[("x-status", "204")],
        "",
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(body, Value::Null);

    let auth = format!("Bearer {}", anon_key());
    let (status, _, body) = send(
        &state,
        "PUT",
        "/functions/v1/hello",
        &[("authorization", &auth), ("x-status", "418")],
        "",
    )
    .await;
    assert_eq!(status, StatusCode::IM_A_TEAPOT);
    assert_eq!(body["method"], "PUT");
}

#[tokio::test]
async fn redirect_and_reset_content_bodies_are_dropped_like_user_workers_fetch() {
    let state = state(false, EchoRuntime::default());
    for code in ["302", "307", "205"] {
        let request = Request::builder()
            .uri("/functions/v1/hello")
            .header("x-status", code)
            .body(Body::empty())
            .unwrap();
        let response = router(state.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status().as_str(), code);
        assert_eq!(response.headers()["x-function"], "hello", "{code}");
        // The function's JSON length is gone; axum frames the empty body.
        assert_eq!(response.headers()["content-length"], "0", "{code}");
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(bytes.is_empty(), "{code}");
    }

    let (status, headers, body) = send(&state, "HEAD", "/functions/v1/hello", &[], "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["x-function"], "hello");
    assert_eq!(body, Value::Null);
}

#[tokio::test]
async fn missing_function_is_a_worker_creation_error() {
    let runtime = EchoRuntime::default();
    let state = state(false, runtime.clone());
    for uri in [
        "/functions/v1/nope",
        "/functions/v1/..",
        "/functions/v1/nope/x",
    ] {
        let (status, _, body) = send(&state, "POST", uri, &[], "{}").await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{uri}");
        assert_eq!(
            body,
            json!({"msg": "InvalidWorkerCreation: worker boot error: failed to bootstrap runtime: could not find an appropriate entrypoint"}),
            "{uri}"
        );
    }
    assert_eq!(runtime.starts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_runtime_that_cannot_start_is_a_worker_creation_error() {
    let state = state(false, FailingRuntime);
    let (status, _, body) = send(&state, "GET", "/functions/v1/hello", &[], "").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body,
        json!({"msg": "InvalidWorkerCreation: worker boot error: no deno"})
    );
}

#[tokio::test]
async fn empty_function_name_is_400() {
    let state = state(false, EchoRuntime::default());
    for uri in ["/functions/v1/", "/functions/v1//hello"] {
        let (status, _, body) = send(&state, "GET", uri, &[], "").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(body, json!({"msg": "missing function name in request"}));
    }
}

#[tokio::test]
async fn verify_jwt_rejects_like_the_main_service() {
    let runtime = EchoRuntime::default();
    let state = state(true, runtime.clone());
    let cases = [
        (None, "Error: Missing authorization header"),
        (
            Some("Basic abc"),
            "Error: Auth header is not 'Bearer {token}'",
        ),
        (
            Some("Bearer garbage"),
            "TypeError: Invalid Token or Protected Header formatting",
        ),
        (
            Some("Bearer eyJhbGciOiJIUzI1NiJ9.e30.c2lnbmF0dXJl"),
            "Invalid JWT",
        ),
    ];
    for (authorization, msg) in cases {
        let headers: Vec<(&str, &str)> = authorization
            .map(|value| ("authorization", value))
            .into_iter()
            .collect();
        let (status, _, body) = send(&state, "POST", "/functions/v1/hello", &headers, "{}").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{authorization:?}");
        assert_eq!(body, json!({ "msg": msg }), "{authorization:?}");
    }
    // The gate runs before the name check, as upstream.
    let (status, _, _) = send(&state, "GET", "/functions/v1/", &[], "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(runtime.starts.load(Ordering::SeqCst), 0);

    let auth = format!("Bearer {}", anon_key());
    let (status, _, body) = send(
        &state,
        "POST",
        "/functions/v1/hello",
        &[("authorization", &auth)],
        "{}",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["authorization"], auth);
}

#[tokio::test]
async fn disabled_state_is_501() {
    let (status, _, body) = send(
        &FunctionsState::disabled(),
        "GET",
        "/functions/v1/hello",
        &[],
        "",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(body["code"], "MEGABASE_NOT_IMPLEMENTED");
    assert_eq!(body["unit"], "GET /functions/v1/hello");
}
