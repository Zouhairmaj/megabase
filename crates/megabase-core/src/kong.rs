//! Kong gateway rejections from the self-hosted stack.
//!
//! The pinned gateway (`vendor/supabase/docker/volumes/api/kong.yml`,
//! Apache-2.0) runs Kong 3.9 with the `key-auth` and `acl` plugins on most
//! routes. Kong writes its own error bodies with an explicit charset and its
//! per-request id: 32 lowercase hex digits, generated fresh for every request.

use axum::{
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};

/// Header the `key-auth` plugin reads (`key_names` default `["apikey"]`).
pub const API_KEY_HEADER: &str = "apikey";

/// `key-auth` realm advertised on a missing key (`Key realm="kong"`).
const KEY_REALM: &str = "Key realm=\"kong\"";

/// `key-auth` answer when the request carries no key: 401
/// `No API key found in request`.
pub fn no_api_key() -> Response {
    let mut response = error(StatusCode::UNAUTHORIZED, "No API key found in request");
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static(KEY_REALM),
    );
    response
}

/// `acl` answer when the consumer's group is not allowed: 403
/// `You cannot consume this service`.
pub fn acl_forbidden() -> Response {
    error(StatusCode::FORBIDDEN, "You cannot consume this service")
}

/// The key `key-auth` would read: the `apikey` header, else the `apikey`
/// query parameter (`key_in_header` and `key_in_query` default to true).
/// An empty value counts as missing.
pub fn api_key<'a>(headers: &'a HeaderMap, query: Option<&'a str>) -> Option<&'a str> {
    let from_header = headers
        .get(API_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|key| !key.is_empty());
    from_header.or_else(|| {
        query?
            .split('&')
            .filter_map(|pair| pair.split_once('=').or(Some((pair, ""))))
            .find(|(name, _)| *name == API_KEY_HEADER)
            .map(|(_, value)| value)
            .filter(|key| !key.is_empty())
    })
}

fn error(status: StatusCode, message: &str) -> Response {
    let body = serde_json::json!({
        "message": message,
        "request_id": uuid::Uuid::new_v4().simple().to_string(),
    });
    (
        status,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        body.to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_comes_from_header_then_query() {
        let mut headers = HeaderMap::new();
        assert_eq!(api_key(&headers, None), None);
        assert_eq!(api_key(&headers, Some("a=1&apikey=q")), Some("q"));
        assert_eq!(api_key(&headers, Some("apikey=")), None);
        assert_eq!(api_key(&headers, Some("apikey")), None);
        headers.insert(API_KEY_HEADER, HeaderValue::from_static(""));
        assert_eq!(api_key(&headers, None), None);
        headers.insert(API_KEY_HEADER, HeaderValue::from_static("h"));
        assert_eq!(api_key(&headers, Some("apikey=q")), Some("h"));
    }

    #[test]
    fn no_api_key_is_kong_401() {
        let response = no_api_key();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()[header::WWW_AUTHENTICATE], KEY_REALM);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/json; charset=utf-8"
        );
    }

    #[test]
    fn acl_forbidden_is_kong_403() {
        assert_eq!(acl_forbidden().status(), StatusCode::FORBIDDEN);
    }
}
