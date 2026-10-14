//! Kong's `cors` plugin as the self-hosted gateway enables it.
//!
//! `vendor/supabase/docker/volumes/api/kong.yml` lists `- name: cors` with no
//! `config` on every service except `meta` (`/pg/`) and the Studio catch-all,
//! so the plugin defaults of Kong 3.9.3 (`docker-compose.yml`) apply:
//!
//! - A preflight (`OPTIONS` with `Origin` and `Access-Control-Request-Method`)
//!   is answered in Kong's access phase, before key-auth, ACL, and the
//!   upstream: 200, empty body, `Access-Control-Allow-Origin: *`,
//!   `Access-Control-Allow-Methods` listing every method, and
//!   `Access-Control-Allow-Headers` echoing `Access-Control-Request-Headers`
//!   when the request carries it. `credentials` and `max_age` are off, so
//!   `Access-Control-Allow-Credentials` and `Access-Control-Max-Age` are absent.
//! - Every other response gets `Access-Control-Allow-Origin: *`, whether or not
//!   the request has an `Origin`. Kong sets the header, so an upstream value
//!   (PostgREST's own `*`) is replaced, never duplicated.
//! - `exposed_headers` is not configured, so `Access-Control-Expose-Headers`
//!   comes from the upstream alone (see `megabase-rest`'s PostgREST layer).
//!
//! Specified in `specs/rest/cors.md`.

use axum::{
    extract::Request,
    http::{header, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// `Access-Control-Allow-Methods` on a preflight: Kong's `methods` default,
/// joined with `,` and no space.
pub const ALLOW_METHODS: &str = "GET,HEAD,PUT,PATCH,POST,DELETE,OPTIONS,TRACE,CONNECT";

/// Kong route prefixes whose service lists the `cors` plugin in `kong.yml`.
///
/// `/pg/` (`meta`) and the Studio catch-all do not, so they get no CORS
/// headers at all.
pub const CORS_ROUTES: &[&str] = &[
    "/auth/v1/",
    "/.well-known/oauth-authorization-server",
    "/rest/v1/",
    "/graphql/v1",
    "/realtime/v1/",
    "/storage/v1/",
    "/functions/v1/",
];

/// Whether the Kong service behind `prefix` runs the `cors` plugin.
#[must_use]
pub fn has_cors_plugin(prefix: &str) -> bool {
    CORS_ROUTES.contains(&prefix)
}

fn is_preflight(request: &Request) -> bool {
    request.method() == Method::OPTIONS
        && request.headers().contains_key(header::ORIGIN)
        && request
            .headers()
            .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD)
}

/// Answer a preflight, or forward the request and stamp
/// `Access-Control-Allow-Origin: *` on the response.
pub async fn kong_cors(request: Request, next: Next) -> Response {
    if is_preflight(&request) {
        return preflight_response(
            request
                .headers()
                .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
                .cloned(),
        );
    }
    let mut response = next.run(request).await;
    allow_any_origin(&mut response);
    response
}

fn allow_any_origin(response: &mut Response) {
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
}

fn preflight_response(requested_headers: Option<HeaderValue>) -> Response {
    let mut response = StatusCode::OK.into_response();
    allow_any_origin(&mut response);
    let headers = response.headers_mut();
    if let Some(requested) = requested_headers {
        headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, requested);
    }
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static(ALLOW_METHODS),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GATEWAY_ROUTES;

    #[test]
    fn every_gateway_route_is_classified() {
        for (prefix, component) in GATEWAY_ROUTES {
            assert_eq!(
                has_cors_plugin(prefix),
                *component != megabase_meta::COMPONENT,
                "{prefix}"
            );
        }
        assert!(!has_cors_plugin("/"));
        assert!(!has_cors_plugin(crate::HEALTH_PATH));
    }

    #[test]
    fn preflight_needs_origin_and_requested_method() {
        let options = |headers: &[(header::HeaderName, &str)]| {
            let mut builder = Request::builder().method(Method::OPTIONS).uri("/rest/v1/x");
            for (name, value) in headers {
                builder = builder.header(name, *value);
            }
            builder.body(axum::body::Body::empty()).unwrap()
        };
        assert!(is_preflight(&options(&[
            (header::ORIGIN, "http://localhost:3000"),
            (header::ACCESS_CONTROL_REQUEST_METHOD, "GET"),
        ])));
        assert!(!is_preflight(&options(&[(
            header::ORIGIN,
            "http://localhost:3000"
        )])));
        assert!(!is_preflight(&options(&[(
            header::ACCESS_CONTROL_REQUEST_METHOD,
            "GET"
        )])));
        let get = Request::builder()
            .method(Method::GET)
            .uri("/rest/v1/x")
            .header(header::ORIGIN, "http://localhost:3000")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
            .body(axum::body::Body::empty())
            .unwrap();
        assert!(!is_preflight(&get));
    }

    #[test]
    fn preflight_echoes_requested_headers_only_when_present() {
        let with = preflight_response(Some(HeaderValue::from_static("apikey,prefer")));
        assert_eq!(with.status(), StatusCode::OK);
        assert_eq!(
            with.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS],
            "apikey,prefer"
        );
        assert_eq!(
            with.headers()[header::ACCESS_CONTROL_ALLOW_METHODS],
            ALLOW_METHODS
        );
        assert_eq!(with.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert!(with
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
            .is_none());
        assert!(with.headers().get(header::ACCESS_CONTROL_MAX_AGE).is_none());

        let without = preflight_response(None);
        assert!(without
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
            .is_none());
    }
}
