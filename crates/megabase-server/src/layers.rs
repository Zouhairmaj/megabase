//! Production HTTP middleware for the gateway.
//!
//! Defaults follow the pinned self-hosted stack where it documents a number:
//! Kong's functions `read_timeout` (150000 ms) and `FILE_SIZE_LIMIT`
//! (52428800 bytes). Request ids, panic catching, and header redaction are
//! Megabase controls; Kong does not set those in `volumes/api/kong.yml`.
//! The deadline uses the same 504 as `tower_http`'s timeout layer and arms
//! its timer only while the handler is still pending.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use megabase_core::Config;
use pin_project_lite::pin_project;
use tower::Layer;
use tower_http::{
    catch_panic::CatchPanicLayer,
    classify::ServerErrorsFailureClass,
    limit::RequestBodyLimitLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    sensitive_headers::SetSensitiveRequestHeadersLayer,
    trace::TraceLayer,
};
use tracing::Span;

/// Headers whose values must not appear in traces.
///
/// `apikey` is the Supabase client key header Kong's key-auth plugin reads.
pub fn sensitive_request_headers() -> [axum::http::HeaderName; 3] {
    [
        axum::http::header::AUTHORIZATION,
        axum::http::header::COOKIE,
        axum::http::HeaderName::from_static("apikey"),
    ]
}

/// Apply the production layer stack. The last layer is the outermost.
///
/// Request path, outer to inner: catch panic, assign `x-request-id`, mark
/// `Authorization` / `apikey` / `Cookie` sensitive, trace, copy the request id
/// onto the response, enforce the body limit, then the timeout.
pub fn apply_http_layers(router: Router, config: &Config) -> Router {
    let sensitive: Arc<[axum::http::HeaderName]> = Arc::from(sensitive_request_headers());
    router
        .layer(DeadlineLayer::new(
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            config.http_timeout,
        ))
        .layer(RequestBodyLimitLayer::new(config.request_body_limit))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<Body>| {
                    let request_id = request
                        .headers()
                        .get(axum::http::HeaderName::from_static("x-request-id"))
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("");
                    tracing::info_span!(
                        "http",
                        method = %request.method(),
                        uri = %request.uri(),
                        request_id = %request_id,
                    )
                })
                // The span already carries method, path, and request id.
                // Default callbacks emit a second event and still wrap the body.
                .on_request(())
                .on_response(())
                .on_body_chunk(())
                .on_eos(())
                .on_failure(
                    |error: ServerErrorsFailureClass, _latency: Duration, _span: &Span| {
                        tracing::error!(%error, "request failed");
                    },
                ),
        )
        .layer(SetSensitiveRequestHeadersLayer::from_shared(sensitive))
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(CatchPanicLayer::custom(|_| panic_response()))
}

/// Whole-request deadline with the same contract as `tower_http::timeout::TimeoutLayer`.
///
/// `TimeoutLayer` calls `tokio::time::sleep` before polling the handler, so every
/// request registers a timer. These smoke routes finish on the first poll, and
/// that registration was most of their latency. The timer is armed only when
/// the handler is still pending. The deadline is still measured from `call`.
#[derive(Clone, Copy)]
struct DeadlineLayer {
    status: StatusCode,
    timeout: Duration,
}

impl DeadlineLayer {
    fn new(status: StatusCode, timeout: Duration) -> Self {
        Self { status, timeout }
    }
}

impl<S> Layer<S> for DeadlineLayer {
    type Service = Deadline<S>;

    fn layer(&self, inner: S) -> Self::Service {
        Deadline {
            inner,
            status: self.status,
            timeout: self.timeout,
        }
    }
}

#[derive(Clone, Copy)]
struct Deadline<S> {
    inner: S,
    status: StatusCode,
    timeout: Duration,
}

impl<S, ReqBody, ResBody> tower::Service<Request<ReqBody>> for Deadline<S>
where
    S: tower::Service<Request<ReqBody>, Response = Response<ResBody>>,
    ResBody: Default,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = DeadlineFuture<S::Future>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let timeout = self.timeout;
        DeadlineFuture {
            inner: self.inner.call(req),
            deadline: tokio::time::Instant::now() + timeout,
            sleep: None,
            status: self.status,
        }
    }
}

pin_project! {
    struct DeadlineFuture<F> {
        #[pin]
        inner: F,
        deadline: tokio::time::Instant,
        sleep: Option<std::pin::Pin<Box<tokio::time::Sleep>>>,
        status: StatusCode,
    }
}

impl<F, ResBody, E> std::future::Future for DeadlineFuture<F>
where
    F: std::future::Future<Output = Result<Response<ResBody>, E>>,
    ResBody: Default,
{
    type Output = Result<Response<ResBody>, E>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.project();
        if let std::task::Poll::Ready(result) = this.inner.poll(cx) {
            return std::task::Poll::Ready(result);
        }
        let sleep = this
            .sleep
            .get_or_insert_with(|| Box::pin(tokio::time::sleep_until(*this.deadline)));
        if sleep.as_mut().poll(cx).is_ready() {
            let mut response = Response::new(ResBody::default());
            *response.status_mut() = *this.status;
            return std::task::Poll::Ready(Ok(response));
        }
        std::task::Poll::Pending
    }
}

fn panic_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(serde_json::json!({
            "code": "internal_error",
            "message": "internal error",
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, routing::get, Router};
    use megabase_core::{Config, DEFAULT_HTTP_TIMEOUT, DEFAULT_REQUEST_BODY_LIMIT};
    use tower::ServiceExt;

    #[test]
    fn defaults_match_the_pinned_stack() {
        let config = Config::default();
        assert_eq!(config.http_timeout, DEFAULT_HTTP_TIMEOUT);
        assert_eq!(config.http_timeout, Duration::from_millis(150_000));
        assert_eq!(config.request_body_limit, DEFAULT_REQUEST_BODY_LIMIT);
        assert_eq!(config.request_body_limit, 52_428_800);
        let names: Vec<_> = sensitive_request_headers()
            .into_iter()
            .map(|name| name.to_string())
            .collect();
        assert_eq!(names, ["authorization", "cookie", "apikey"]);
    }

    #[tokio::test]
    async fn authorization_is_marked_sensitive_before_tracing() {
        async fn show(headers: axum::http::HeaderMap) -> String {
            format!(
                "{:?}",
                headers.get(axum::http::header::AUTHORIZATION).unwrap()
            )
        }
        let app = apply_http_layers(
            Router::new().route("/show", get(show)),
            &Config {
                http_timeout: Duration::from_secs(2),
                request_body_limit: 1024,
                ..Config::default()
            },
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/show")
                    .header(axum::http::header::AUTHORIZATION, "Bearer secret-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(std::str::from_utf8(&bytes).unwrap(), "Sensitive");
    }

    #[allow(unreachable_code)]
    async fn boom() -> &'static str {
        panic!("secret boom");
        "ok"
    }

    fn limited(timeout: Duration, body_limit: usize) -> Router {
        apply_http_layers(
            Router::new()
                .route(
                    "/slow",
                    get(|| async {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        "done"
                    }),
                )
                .route("/panic", get(boom))
                .route("/ok", get(|| async { "ok" }))
                .route(
                    "/echo",
                    axum::routing::post(
                        |body: axum::body::Bytes| async move { body.len().to_string() },
                    ),
                ),
            &Config {
                http_timeout: timeout,
                request_body_limit: body_limit,
                ..Config::default()
            },
        )
    }

    #[tokio::test]
    async fn request_id_is_generated_and_echoed() {
        let response = limited(Duration::from_secs(2), 1024)
            .oneshot(Request::builder().uri("/ok").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let generated = response
            .headers()
            .get("x-request-id")
            .expect("generated id")
            .to_str()
            .unwrap()
            .to_string();
        assert!(!generated.is_empty());

        let response = limited(Duration::from_secs(2), 1024)
            .oneshot(
                Request::builder()
                    .uri("/ok")
                    .header("x-request-id", "req-fixed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.headers().get("x-request-id").unwrap(), "req-fixed");
    }

    #[tokio::test]
    async fn body_over_the_limit_is_413() {
        let response = limited(Duration::from_secs(2), 4)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/echo")
                    .body(Body::from("12345"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn slow_handler_returns_504() {
        let response = limited(Duration::from_millis(50), 1024)
            .oneshot(Request::builder().uri("/slow").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::GATEWAY_TIMEOUT);
    }

    #[tokio::test]
    async fn panic_is_500_without_the_message() {
        let response = limited(Duration::from_secs(2), 1024)
            .oneshot(
                Request::builder()
                    .uri("/panic")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = std::str::from_utf8(&bytes).unwrap();
        assert!(body.contains("internal_error"), "{body}");
        assert!(!body.contains("secret boom"), "{body}");
    }
}
