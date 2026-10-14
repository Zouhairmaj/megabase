//! Kong's `cors` plugin as the pinned self-hosted stack attaches it.
//!
//! Every service in `vendor/supabase/docker/volumes/api/kong.yml`
//! (Apache-2.0) except Postgres Meta (`/pg/`), the blocked Realtime admin
//! routes, and the MCP routes lists `- name: cors` with no `config`, so the
//! Kong `3.9.3` image from `vendor/supabase/docker/docker-compose.yml` runs
//! the plugin with its defaults: no configured origins, headers, exposed
//! headers, or max age; `credentials: false`; `preflight_continue: false`;
//! the full default method list. The plugin's `access` phase runs before
//! `key-auth`, `acl`, and `basic-auth`, so a preflight needs no `apikey`.
//!
//! With those defaults the plugin does two things:
//!
//! - On `OPTIONS` carrying both `Origin` and `Access-Control-Request-Method`
//!   it answers `200` with an empty body before the upstream service:
//!   `Access-Control-Allow-Origin: *`, `Access-Control-Allow-Headers` echoing
//!   `Access-Control-Request-Headers` when the request sent one, and
//!   `Access-Control-Allow-Methods` with the default list. It sets no
//!   `Access-Control-Allow-Credentials`, `Access-Control-Max-Age`, or `Vary`.
//! - On every other response it sets `Access-Control-Allow-Origin: *`,
//!   replacing any value the upstream service set, whether or not the request
//!   carried `Origin`.
//!
//! Kong's route table picks the longest matching path prefix, so the
//! exclusions below are plain prefix tests as in [`crate::match_gateway_route`].
//! Spec: `specs/server/cors.md`.

use std::future::Future;
use std::pin::Pin;
use std::task::{ready, Context, Poll};

use axum::http::{header, HeaderValue, Method, Request, Response, StatusCode};
use pin_project_lite::pin_project;
use tower::{Layer, Service};

/// `Access-Control-Allow-Methods` of a preflight: the plugin's default
/// `methods` list, joined with commas as Kong emits it.
pub const CORS_ALLOW_METHODS: &str = "GET,HEAD,PUT,PATCH,POST,DELETE,OPTIONS,TRACE,CONNECT";

/// Kong route path prefixes whose service has no `cors` plugin in the pinned
/// `kong.yml`: Postgres Meta, the two blocked Realtime admin routes, and
/// both MCP routes. Every other path, including the Studio catch-all, has it.
pub const ROUTES_WITHOUT_CORS: &[&str] = &[
    "/pg/",
    "/realtime/v1/api/openapi",
    "/realtime/v1/api/tenants",
    "/api/mcp",
    "/mcp",
];

/// Whether the pinned gateway runs the `cors` plugin for `path`.
#[must_use]
pub fn cors_applies(path: &str) -> bool {
    !ROUTES_WITHOUT_CORS
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

/// Kong's preflight test: `OPTIONS` with `Origin` and
/// `Access-Control-Request-Method`, any values.
fn is_preflight<B>(request: &Request<B>) -> bool {
    request.method() == Method::OPTIONS
        && request.headers().contains_key(header::ORIGIN)
        && request
            .headers()
            .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD)
}

/// The response Kong's `access` phase sends for a preflight. The request is
/// consumed: Kong never forwards a preflight upstream with these defaults.
fn preflight_response<ReqBody, ResBody: Default>(
    mut request: Request<ReqBody>,
) -> Response<ResBody> {
    let mut response = Response::new(ResBody::default());
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    // `kong.request.get_header` returns the first value of a repeated header.
    if let Some(requested) = request
        .headers_mut()
        .remove(header::ACCESS_CONTROL_REQUEST_HEADERS)
    {
        headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, requested);
    }
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static(CORS_ALLOW_METHODS),
    );
    response
}

/// Tower layer for [`Cors`].
#[derive(Clone, Copy, Debug, Default)]
pub struct CorsLayer;

impl<S> Layer<S> for CorsLayer {
    type Service = Cors<S>;

    fn layer(&self, inner: S) -> Self::Service {
        Cors { inner }
    }
}

/// Service that answers preflights and stamps `Access-Control-Allow-Origin`
/// on every other response for paths where [`cors_applies`].
#[derive(Clone, Copy, Debug)]
pub struct Cors<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for Cors<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    ResBody: Default,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = CorsFuture<S::Future, ResBody>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<ReqBody>) -> Self::Future {
        if !cors_applies(request.uri().path()) {
            return CorsFuture::Inner {
                inner: self.inner.call(request),
                allow_origin: false,
            };
        }
        if is_preflight(&request) {
            return CorsFuture::Preflight {
                response: Some(preflight_response(request)),
            };
        }
        CorsFuture::Inner {
            inner: self.inner.call(request),
            allow_origin: true,
        }
    }
}

pin_project! {
    /// Response future of [`Cors`].
    #[project = CorsFutureProj]
    pub enum CorsFuture<F, B> {
        /// A preflight answered by the gateway; `None` once yielded.
        Preflight { response: Option<Response<B>> },
        /// The inner service's response, stamped when `allow_origin`.
        Inner { #[pin] inner: F, allow_origin: bool },
    }
}

impl<F, B, E> Future for CorsFuture<F, B>
where
    F: Future<Output = Result<Response<B>, E>>,
{
    type Output = Result<Response<B>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project() {
            CorsFutureProj::Preflight { response } => {
                // Polling a completed future breaks the `Future` contract.
                let response = response.take().expect("CorsFuture polled after completion");
                Poll::Ready(Ok(response))
            }
            CorsFutureProj::Inner {
                inner,
                allow_origin,
            } => {
                let result = ready!(inner.poll(cx));
                Poll::Ready(result.map(|mut response| {
                    if *allow_origin {
                        response.headers_mut().insert(
                            header::ACCESS_CONTROL_ALLOW_ORIGIN,
                            HeaderValue::from_static("*"),
                        );
                    }
                    response
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;

    #[test]
    fn cors_applies_everywhere_but_the_kong_routes_without_the_plugin() {
        for path in [
            "/auth/v1/signup",
            "/auth/v1/token",
            "/.well-known/oauth-authorization-server",
            "/rest/v1/",
            "/rest/v1/todos",
            "/graphql/v1",
            "/realtime/v1/websocket",
            "/realtime/v1/api/broadcast",
            "/storage/v1/object/b/k",
            "/functions/v1/hello",
            "/",
            "/auth/v1",
            "/pg",
            "/_megabase/health",
        ] {
            assert!(cors_applies(path), "{path}");
        }
        for path in [
            "/pg/",
            "/pg/tables",
            "/realtime/v1/api/openapi",
            "/realtime/v1/api/openapi/x",
            "/realtime/v1/api/tenants",
            "/realtime/v1/api/tenants/realtime-dev",
            "/api/mcp",
            "/mcp",
            "/mcp/anything",
        ] {
            assert!(!cors_applies(path), "{path}");
        }
    }

    #[test]
    fn preflight_needs_options_origin_and_request_method() {
        let request = |method: &str, headers: &[(&str, &str)]| {
            let mut builder = Request::builder().method(method).uri("/auth/v1/signup");
            for (name, value) in headers {
                builder = builder.header(*name, *value);
            }
            builder.body(Body::empty()).unwrap()
        };
        assert!(is_preflight(&request(
            "OPTIONS",
            &[
                ("origin", "http://localhost:3000"),
                ("access-control-request-method", "POST"),
            ],
        )));
        assert!(is_preflight(&request(
            "OPTIONS",
            &[("origin", ""), ("access-control-request-method", "")],
        )));
        assert!(!is_preflight(&request(
            "OPTIONS",
            &[("origin", "http://localhost:3000")],
        )));
        assert!(!is_preflight(&request(
            "OPTIONS",
            &[("access-control-request-method", "POST")],
        )));
        assert!(!is_preflight(&request(
            "POST",
            &[
                ("origin", "http://localhost:3000"),
                ("access-control-request-method", "POST"),
            ],
        )));
    }

    #[test]
    fn preflight_response_echoes_the_first_requested_headers_value() {
        let request = Request::builder()
            .method("OPTIONS")
            .uri("/auth/v1/signup")
            .header("origin", "http://localhost:3000")
            .header("access-control-request-method", "POST")
            .header("access-control-request-headers", "apikey,content-type")
            .header("access-control-request-headers", "x-client-info")
            .body(Body::empty())
            .unwrap();
        let response: Response<Body> = preflight_response(request);
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS],
            "apikey,content-type"
        );
        assert_eq!(
            response
                .headers()
                .get_all(header::ACCESS_CONTROL_ALLOW_HEADERS)
                .iter()
                .count(),
            1
        );
    }

    #[test]
    fn preflight_response_without_requested_headers_has_no_allow_headers() {
        let request = Request::builder()
            .method("OPTIONS")
            .uri("/auth/v1/signup")
            .header("origin", "http://localhost:3000")
            .header("access-control-request-method", "POST")
            .body(Body::empty())
            .unwrap();
        let response: Response<Body> = preflight_response(request);
        assert!(response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
            .is_none());
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_METHODS],
            CORS_ALLOW_METHODS
        );
        assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    }
}
