//! `/s3/**`: the S3 protocol routes, answered for requests that carry no
//! AWS Signature V4 (`specs/storage/s3.md`).

// Ported from supabase/storage src/http/routes/s3/index.ts,
// src/http/routes/s3/error-handler.ts, src/http/plugins/signature-v4.ts,
// src/http/plugins/xml.ts and src/internal/errors/codes.ts (Apache-2.0),
// pin v1.80.2.

use axum::{
    extract::OriginalUri,
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};

use super::not_implemented;

/// The Kong route prefix storage-api never sees (`strip_path: true`).
const KONG_PREFIX: &str = "/storage/v1";

/// `S3_XML_NAMESPACE` in `routes/s3/index.ts`.
const S3_XML_NAMESPACE: &str = "http://s3.amazonaws.com/doc/2006-03-01/";

/// `XML_DECLARATION` in `plugins/xml.ts`.
const XML_DECLARATION: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;

/// find-my-way's default `maxParamLength`.
const MAX_PARAM_LENGTH: usize = 100;

/// `GET` and `DELETE` on `/s3`, `/s3/:Bucket` and `/s3/:Bucket/*`. The
/// `signatureV4` `preHandler` runs before any S3 command is selected, so a
/// request without a signature gets `AccessDenied('Missing signature')`
/// whatever its query or headers would have matched.
///
/// Signed requests, Fastify body parsing, `Accept` negotiation and the
/// router's not-found and bad-URL answers are not ported and stay 501.
// megabase:unit storage:route:DELETE /storage/v1/s3/{Bucket}
// megabase:unit storage:route:DELETE /storage/v1/s3/{Bucket}/{*}
// megabase:unit storage:route:DELETE /storage/v1/s3/{Bucket}/{*}?uploadId
// megabase:unit storage:route:DELETE /storage/v1/s3/{Bucket}?lifecycle
// megabase:unit storage:route:GET /storage/v1/s3
// megabase:unit storage:route:GET /storage/v1/s3/{Bucket}
// megabase:unit storage:route:GET /storage/v1/s3/{Bucket}/{*}
// megabase:unit storage:route:GET /storage/v1/s3/{Bucket}/{*}?tagging
// megabase:unit storage:route:GET /storage/v1/s3/{Bucket}/{*}?uploadId
// megabase:unit storage:route:GET /storage/v1/s3/{Bucket}?lifecycle
pub(super) async fn unsigned(
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let served = matches!(method, Method::GET | Method::DELETE)
        && !(method == Method::DELETE && has_framed_content_type(&headers))
        && routable(uri.path())
        && !is_signed(&headers, uri.query())
        && accepts_xml_fast_path(&headers);
    if !served {
        return not_implemented(&method, &uri);
    }
    s3_error(
        StatusCode::FORBIDDEN,
        &resource(uri.path()),
        "AccessDenied",
        "Missing signature",
    )
}

/// `DELETE` bodies are parsed only when a `Content-Type` comes with a
/// `Transfer-Encoding` or a `Content-Length` (fastify `handleRequest`).
fn has_framed_content_type(headers: &HeaderMap) -> bool {
    headers.contains_key(header::CONTENT_TYPE)
        && (headers.contains_key(header::TRANSFER_ENCODING)
            || headers.contains_key(header::CONTENT_LENGTH))
}

/// Paths find-my-way matches to an S3 route without its not-found or
/// bad-URL answer: `/s3`, `/s3/`, or a non-empty bucket of at most
/// `maxParamLength` characters followed by an optional key; the bucket and
/// the key must have percent-encoding that decodes to UTF-8.
fn routable(path: &str) -> bool {
    let Some(rest) = path
        .strip_prefix(KONG_PREFIX)
        .and_then(|p| p.strip_prefix("/s3"))
    else {
        return false;
    };
    if rest.is_empty() || rest == "/" {
        return true;
    }
    let Some(rest) = rest.strip_prefix('/') else {
        return false;
    };
    let (bucket, key) = rest.split_once('/').unwrap_or((rest, ""));
    !bucket.is_empty()
        && bucket.len() <= MAX_PARAM_LENGTH
        && decodes_to_utf8(bucket)
        && decodes_to_utf8(key)
}

fn decodes_to_utf8(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let Some(byte) = bytes
                .get(i + 1..i + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            else {
                return false;
            };
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).is_ok()
}

/// `extractSignature` would read a signature: an `AWS4-HMAC-SHA256 `
/// `Authorization` header or an `X-Amz-Credential` query parameter. Query
/// keys are compared without regard to case or encoding, so a near miss
/// stays 501 rather than risking the wrong answer.
fn is_signed(headers: &HeaderMap, query: Option<&str>) -> bool {
    let header_signed = headers
        .get(header::AUTHORIZATION)
        .is_some_and(|value| value.as_bytes().starts_with(b"AWS4-HMAC-SHA256 "));
    let query_signed = query.is_some_and(|query| {
        query.split('&').any(|pair| {
            let key = pair.split_once('=').map_or(pair, |(key, _)| key);
            key.contains('%') || key.eq_ignore_ascii_case("x-amz-credential")
        })
    });
    header_signed || query_signed
}

/// The `acceptsXmlResponse` checks that need no media-type negotiation.
fn accepts_xml_fast_path(headers: &HeaderMap) -> bool {
    let Some(accept) = headers.get(header::ACCEPT) else {
        return true;
    };
    let Ok(accept) = accept.to_str() else {
        return false;
    };
    let accept = accept.trim().to_ascii_lowercase();
    matches!(
        accept.as_str(),
        "*/*" | "application/xml" | "text/xml" | "text/html"
    )
}

/// `getS3ErrorResource`: the path without the first `/s3` and without empty
/// segments.
fn resource(path: &str) -> String {
    let path = path.strip_prefix(KONG_PREFIX).unwrap_or(path);
    let path = path.replacen("/s3", "", 1);
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// `formatS3ErrorResponse` serialized by the `xmlParser` plugin.
fn s3_error(status: StatusCode, resource: &str, code: &str, message: &str) -> Response {
    let resource = if resource.is_empty() {
        "<Resource/>".to_owned()
    } else {
        format!("<Resource>{}</Resource>", escape_text(resource))
    };
    let body = format!(
        "{XML_DECLARATION}<Error xmlns=\"{S3_XML_NAMESPACE}\">{resource}<Code>{}</Code><Message>{}</Message></Error>",
        escape_text(code),
        escape_text(message),
    );
    (
        status,
        [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
        body,
    )
        .into_response()
}

/// `escapeXmlValue` for text nodes: `\t` and `\n` stay as they are.
fn escape_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            '\t' | '\n' => out.push(character),
            c if u32::from(c) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {
                out.push('\u{FFFD}');
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use tower::ServiceExt;

    use super::super::{router_with_state, StorageState};
    use super::*;

    async fn send(
        method: &str,
        path: &str,
        extra: &[(header::HeaderName, &str)],
    ) -> (StatusCode, HeaderMap, String) {
        let mut request = axum::http::Request::builder().method(method).uri(path);
        for (name, value) in extra {
            request = request.header(name, *value);
        }
        let response = router_with_state(StorageState::default())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
    }

    fn missing_signature(resource: &str) -> String {
        let resource = if resource.is_empty() {
            "<Resource/>".to_owned()
        } else {
            format!("<Resource>{resource}</Resource>")
        };
        format!(
            "{XML_DECLARATION}<Error xmlns=\"{S3_XML_NAMESPACE}\">{resource}\
             <Code>AccessDenied</Code><Message>Missing signature</Message></Error>"
        )
    }

    const ROUTES: [(&str, &str, &str); 13] = [
        ("GET", "/s3", ""),
        ("GET", "/s3/", ""),
        ("GET", "/s3/photos", "photos"),
        ("GET", "/s3/photos/", "photos"),
        ("GET", "/s3/photos?lifecycle", "photos"),
        ("GET", "/s3/photos/a/b.png", "photos/a/b.png"),
        ("GET", "/s3/photos/a.png?tagging", "photos/a.png"),
        ("GET", "/s3/photos/a.png?uploadId=u1", "photos/a.png"),
        ("DELETE", "/s3/photos", "photos"),
        ("DELETE", "/s3/photos?lifecycle", "photos"),
        ("DELETE", "/s3/photos//a%20b.png", "photos/a%20b.png"),
        ("DELETE", "/s3/photos/a.png?uploadId=u1", "photos/a.png"),
        ("GET", "/s3/ph%6Ftos", "ph%6Ftos"),
    ];

    #[tokio::test]
    async fn unsigned_requests_are_access_denied() {
        for (method, path, resource) in ROUTES {
            let (status, headers, body) = send(method, &format!("{KONG_PREFIX}{path}"), &[]).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
            assert_eq!(
                headers[header::CONTENT_TYPE],
                "application/xml; charset=utf-8"
            );
            assert_eq!(body, missing_signature(resource), "{method} {path}");
        }
    }

    #[tokio::test]
    async fn a_jwt_is_not_a_signature() {
        for (name, value) in [
            (header::AUTHORIZATION, "Bearer eyJhbGciOiJIUzI1NiJ9.e30.x"),
            (header::AUTHORIZATION, "aws4-hmac-sha256 Credential=x"),
            (header::ACCEPT, "*/*"),
            (header::ACCEPT, " Application/XML "),
            (header::ACCEPT, "text/html"),
        ] {
            let (status, _, _) = send("GET", "/storage/v1/s3/photos", &[(name, value)]).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{value}");
        }
    }

    #[tokio::test]
    async fn signed_requests_stay_501() {
        let (status, _, _) = send(
            "GET",
            "/storage/v1/s3/photos",
            &[(
                header::AUTHORIZATION,
                "AWS4-HMAC-SHA256 Credential=k/20260101/local/s3/aws4_request",
            )],
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        for query in [
            "X-Amz-Credential=k",
            "x-amz-credential",
            "X%2DAmz-Credential=k",
        ] {
            let (status, _, _) =
                send("GET", &format!("/storage/v1/s3/photos/a?{query}"), &[]).await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{query}");
        }
    }

    #[tokio::test]
    async fn unported_paths_stay_501() {
        let long = "b".repeat(MAX_PARAM_LENGTH + 1);
        for (method, path, extra) in [
            ("HEAD", "/s3/photos".to_owned(), vec![]),
            ("PUT", "/s3/photos".to_owned(), vec![]),
            ("POST", "/s3/photos?delete".to_owned(), vec![]),
            ("GET", "/s3//key".to_owned(), vec![]),
            ("GET", format!("/s3/{long}"), vec![]),
            ("GET", "/s3/ph%zztos".to_owned(), vec![]),
            ("GET", "/s3/ph%ff".to_owned(), vec![]),
            ("GET", "/s3/photos/%ff".to_owned(), vec![]),
            ("GET", "/s3/photos/%zz".to_owned(), vec![]),
            ("GET", "/s3x/photos".to_owned(), vec![]),
            (
                "GET",
                "/s3/photos".to_owned(),
                vec![(header::ACCEPT, "application/json")],
            ),
            (
                "DELETE",
                "/s3/photos".to_owned(),
                vec![
                    (header::CONTENT_TYPE, "application/xml"),
                    (header::CONTENT_LENGTH, "0"),
                ],
            ),
        ] {
            let (status, _, _) = send(method, &format!("{KONG_PREFIX}{path}"), &extra).await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{method} {path}");
        }
    }

    #[test]
    fn resource_drops_the_prefix_and_empty_segments() {
        assert_eq!(resource("/storage/v1/s3"), "");
        assert_eq!(resource("/storage/v1/s3/b//k/"), "b/k");
        assert_eq!(resource("/storage/v1/s3/b/s3/k"), "b/s3/k");
    }

    #[test]
    fn text_is_escaped_as_xml_ts_does() {
        assert_eq!(escape_text("a&b<c>\r\t\n"), "a&amp;b&lt;c&gt;&#xD;\t\n");
        assert_eq!(escape_text("\u{1}"), "\u{FFFD}");
    }
}
