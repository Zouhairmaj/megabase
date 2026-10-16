// Ported from supabase/edge-runtime ext/workers/user_workers.js (MIT), pin v1.77.4.

//! Forward one request to a worker over HTTP/1.1 and stream the response
//! back, the way Kong proxies `/functions/v1/*` to `http://functions:9000/*`
//! (`vendor/supabase/docker/volumes/api/kong.yml:344`, `strip_path: true`)
//! and edge-runtime's `UserWorker.fetch` relays the worker's answer.

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
use axum::response::Response;
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tracing::debug;

/// `Host` the worker sees. Kong does not preserve the client's host, so the
/// upstream request carries the `functions-v1` service host, and
/// `new URL(req.url)` in a function reads `http://functions:9000/<name>...`.
pub const UPSTREAM_HOST: &str = "functions:9000";

/// Why a worker did not answer.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    /// Connecting to the worker failed.
    #[error("{0}")]
    Connect(#[from] std::io::Error),
    /// The HTTP exchange with the worker failed.
    #[error("{0}")]
    Http(#[from] hyper::Error),
    /// The upstream path is not a valid request-target.
    #[error("{0}")]
    Target(#[from] axum::http::uri::InvalidUri),
}

/// Hop-by-hop headers (RFC 9110 section 7.6.1) a proxy does not forward.
const HOP_BY_HOP: &[HeaderName] = &[
    header::CONNECTION,
    header::PROXY_AUTHENTICATE,
    header::PROXY_AUTHORIZATION,
    header::TE,
    header::TRAILER,
    header::TRANSFER_ENCODING,
    header::UPGRADE,
];

fn strip_hop_by_hop(headers: &mut HeaderMap) {
    let listed: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| HeaderName::try_from(name.trim()).ok())
        .collect();
    for name in HOP_BY_HOP.iter().chain(&listed) {
        headers.remove(name);
    }
    headers.remove("keep-alive");
    headers.remove("proxy-connection");
}

/// Send `request` to the worker at `addr` with request-target `target`
/// (the path after `/functions/v1`, with the query). Method, headers
/// (including `Authorization`) and body pass through unchanged.
///
/// # Errors
///
/// [`ProxyError`] when the worker cannot be reached or the exchange fails.
pub async fn forward(
    addr: SocketAddr,
    request: Request,
    target: &str,
) -> Result<Response, ProxyError> {
    let stream = TcpStream::connect(addr).await?;
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake::<_, Body>(TokioIo::new(stream)).await?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            debug!(%error, "edge function connection closed");
        }
    });

    let (mut parts, body) = request.into_parts();
    let method = parts.method.clone();
    parts.uri = target.parse::<Uri>()?;
    parts.version = axum::http::Version::HTTP_11;
    strip_hop_by_hop(&mut parts.headers);
    parts
        .headers
        .insert(header::HOST, HeaderValue::from_static(UPSTREAM_HOST));
    parts.extensions = axum::http::Extensions::new();

    let response = sender
        .send_request(Request::from_parts(parts, body))
        .await?;
    let (mut parts, body) = response.into_parts();
    strip_hop_by_hop(&mut parts.headers);
    if drops_body(&method, parts.status) {
        // A redirect or 205 keeps no bytes, so its stale length goes too, and
        // RFC 9110 §9.3.6 forbids `Content-Length` in a 2xx answer to CONNECT;
        // a HEAD or 204/304 `Content-Length` still describes the resource.
        if drops_content_length(&method, parts.status) {
            parts.headers.remove(header::CONTENT_LENGTH);
        }
        return Ok(Response::from_parts(parts, Body::empty()));
    }
    Ok(Response::from_parts(parts, Body::new(body)))
}

// `nullBodyStatus` and `redirectStatus` (`user_workers.js:28`–`:35`):
// `UserWorker.fetch` closes the worker's body for these statuses and for
// `HEAD` / `CONNECT` (`:107`–`:122`) and answers `new Response(null, ...)`.
fn null_body_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 101 | 204 | 205 | 304)
}

fn redirect_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

fn drops_content_length(method: &Method, status: StatusCode) -> bool {
    redirect_status(status)
        || status == StatusCode::RESET_CONTENT
        || (*method == Method::CONNECT && status.is_success())
}

fn drops_body(method: &Method, status: StatusCode) -> bool {
    null_body_status(status)
        || redirect_status(status)
        || *method == Method::HEAD
        || *method == Method::CONNECT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hop_by_hop_headers_and_connection_tokens_are_removed() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONNECTION,
            HeaderValue::from_static("close, x-drop"),
        );
        headers.insert("x-drop", HeaderValue::from_static("1"));
        headers.insert("keep-alive", HeaderValue::from_static("timeout=5"));
        headers.insert(
            header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer x"));
        strip_hop_by_hop(&mut headers);
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[header::AUTHORIZATION], "Bearer x");
    }

    #[test]
    fn null_body_and_redirect_statuses_and_head_drop_the_body() {
        for code in [101, 204, 205, 304, 301, 302, 303, 307, 308] {
            let status = StatusCode::from_u16(code).unwrap();
            assert!(drops_body(&Method::GET, status), "{code}");
        }
        for code in [200, 201, 300, 400, 404, 500] {
            let status = StatusCode::from_u16(code).unwrap();
            assert!(!drops_body(&Method::POST, status), "{code}");
            assert!(drops_body(&Method::HEAD, status), "{code}");
            assert!(drops_body(&Method::CONNECT, status), "{code}");
        }
    }

    #[test]
    fn content_length_is_dropped_for_redirects_205_and_connect_2xx() {
        let status = |code| StatusCode::from_u16(code).unwrap();
        for code in [301, 302, 303, 307, 308, 205] {
            assert!(drops_content_length(&Method::GET, status(code)), "{code}");
        }
        assert!(drops_content_length(&Method::CONNECT, status(200)));
        assert!(!drops_content_length(&Method::CONNECT, status(500)));
        assert!(!drops_content_length(&Method::HEAD, status(200)));
        assert!(!drops_content_length(&Method::GET, status(204)));
        assert!(!drops_content_length(&Method::GET, status(304)));
    }
}
