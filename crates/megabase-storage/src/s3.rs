//! `/s3/**`: the S3 protocol routes, answered for requests that carry no
//! AWS Signature V4 (`specs/storage/s3.md`).

// Ported from supabase/storage src/http/routes/s3/index.ts,
// src/http/routes/s3/error-handler.ts, src/http/plugins/signature-v4.ts,
// src/storage/protocols/s3/signature-v4.ts, src/http/plugins/xml.ts and
// src/internal/errors/codes.ts (Apache-2.0), pin v1.80.2.

use axum::{
    body::{to_bytes, Body},
    extract::OriginalUri,
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};

use super::{multipart, not_implemented};

/// The Kong route prefix storage-api never sees (`strip_path: true`).
const KONG_PREFIX: &str = "/storage/v1";

/// The `fields` limit the S3 routes give `@fastify/multipart`.
const MAX_FORM_FIELDS: usize = 20;

/// busboy's default `fieldSize`; a longer value is truncated.
const MAX_FIELD_BYTES: usize = 1024 * 1024;

/// The largest POST policy body Megabase reads; a larger one is a 501.
const MAX_FORM_BYTES: usize = 2 * 1024 * 1024;

/// The form fields `parseMultipartSignature` requires as strings.
const POLICY_SIGNATURE_FIELDS: [&str; 4] = [
    "X-Amz-Credential",
    "X-Amz-Signature",
    "Policy",
    "X-Amz-Date",
];

/// `Object.prototype` keys, which `@fastify/multipart` rejects as field
/// names (`PrototypeViolationError`).
const PROTOTYPE_KEYS: [&str; 12] = [
    "__proto__",
    "constructor",
    "hasOwnProperty",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toLocaleString",
    "toString",
    "valueOf",
    "__defineGetter__",
    "__defineSetter__",
    "__lookupGetter__",
    "__lookupSetter__",
];

/// `S3_XML_NAMESPACE` in `routes/s3/index.ts`.
const S3_XML_NAMESPACE: &str = "http://s3.amazonaws.com/doc/2006-03-01/";

/// `XML_DECLARATION` in `plugins/xml.ts`.
const XML_DECLARATION: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;

/// find-my-way's default `maxParamLength`.
const MAX_PARAM_LENGTH: usize = 100;

/// `GET` on `/s3`, and `GET`, `DELETE`, `HEAD`, `POST` and `PUT` on
/// `/s3/:Bucket` and `/s3/:Bucket/*`. The `signatureV4` `preHandler` runs
/// before any S3 command is selected, so a request without a signature gets
/// `AccessDenied('Missing signature')` whatever its query or headers would
/// have matched. A `multipart/form-data` `POST` is read as a POST policy
/// form first; one without the policy signature fields gets
/// `InvalidSignature('Invalid signature format')`.
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
// megabase:unit storage:route:GET /storage/v1/s3/{Bucket}?list-type=2
// megabase:unit storage:route:GET /storage/v1/s3/{Bucket}?location
// megabase:unit storage:route:GET /storage/v1/s3/{Bucket}?uploads
// megabase:unit storage:route:GET /storage/v1/s3/{Bucket}?versioning
// megabase:unit storage:route:HEAD /storage/v1/s3/{Bucket}
// megabase:unit storage:route:HEAD /storage/v1/s3/{Bucket}/{*}
// megabase:unit storage:route:POST /storage/v1/s3/{Bucket}/{*}?uploadId
// megabase:unit storage:route:POST /storage/v1/s3/{Bucket}/{*}?uploads
// megabase:unit storage:route:POST /storage/v1/s3/{Bucket}?delete
// megabase:unit storage:route:POST /storage/v1/s3/{Bucket}|content-type=multipart/form-data
// megabase:unit storage:route:PUT /storage/v1/s3/{Bucket}
// megabase:unit storage:route:PUT /storage/v1/s3/{Bucket}/{*}
// megabase:unit storage:route:PUT /storage/v1/s3/{Bucket}/{*}?partNumber&uploadId|x-amz-copy-source
// megabase:unit storage:route:PUT /storage/v1/s3/{Bucket}/{*}?uploadId&partNumber
// megabase:unit storage:route:PUT /storage/v1/s3/{Bucket}/{*}|x-amz-copy-source
// megabase:unit storage:route:PUT /storage/v1/s3/{Bucket}?lifecycle
pub(super) async fn unsigned(
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Some(target) = target(uri.path()) else {
        return not_implemented(&method, &uri);
    };
    if is_signed(&headers, uri.query()) || !accepts_xml_fast_path(&headers) {
        return not_implemented(&method, &uri);
    }
    let missing_signature = || {
        s3_error(
            &method,
            StatusCode::FORBIDDEN,
            &resource(uri.path()),
            "AccessDenied",
            "Missing signature",
        )
    };
    let in_bucket = matches!(target, Target::Bucket | Target::Object);
    match (&method, target) {
        (&Method::GET, _) => missing_signature(),
        (&Method::HEAD, _) if in_bucket => missing_signature(),
        (&Method::DELETE, _) if in_bucket && !has_framed_content_type(&headers) => {
            missing_signature()
        }
        (&Method::POST | &Method::PUT, _) if in_bucket && !has_body(&headers) => {
            missing_signature()
        }
        (&Method::PUT, Target::Object) if passes_disabled_parsers(&headers) => missing_signature(),
        (&Method::POST, _) if in_bucket => match unsigned_post_policy(&headers, body).await {
            Some(()) => s3_error(
                &method,
                StatusCode::BAD_REQUEST,
                &resource(uri.path()),
                "InvalidSignature",
                "Invalid signature format",
            ),
            None => not_implemented(&method, &uri),
        },
        _ => not_implemented(&method, &uri),
    }
}

/// `DELETE` bodies are parsed only when a `Content-Type` comes with a
/// `Transfer-Encoding` or a `Content-Length` (fastify `handleRequest`).
fn has_framed_content_type(headers: &HeaderMap) -> bool {
    headers.contains_key(header::CONTENT_TYPE)
        && (headers.contains_key(header::TRANSFER_ENCODING)
            || headers.contains_key(header::CONTENT_LENGTH))
}

/// `POST` bodies are parsed unless there is no `Content-Type`, no
/// `Transfer-Encoding`, and `Content-Length` is absent or `0` (fastify
/// `handleRequest`).
fn has_body(headers: &HeaderMap) -> bool {
    headers.contains_key(header::CONTENT_TYPE)
        || headers.contains_key(header::TRANSFER_ENCODING)
        || headers
            .get(header::CONTENT_LENGTH)
            .is_some_and(|length| length.as_bytes() != b"0")
}

/// The `PUT /:Bucket/*` routes set `disableContentTypeParser`, so every
/// parser that can match there leaves the body unread, except
/// `@fastify/multipart`, which makes `extractSignature` read a form. True
/// for a well-formed `Content-Type` whose type is not `multipart`; a missing
/// or malformed one stays 501 rather than relying on Fastify's fallback.
fn passes_disabled_parsers(headers: &HeaderMap) -> bool {
    let Some(Ok(content_type)) = headers.get(header::CONTENT_TYPE).map(|v| v.to_str()) else {
        return false;
    };
    let mut parts = content_type.split(';');
    let Some((kind, subtype)) = parts.next().and_then(|media| media.split_once('/')) else {
        return false;
    };
    is_token(kind)
        && is_token(subtype)
        && !kind.eq_ignore_ascii_case("multipart")
        && parts.all(|parameter| {
            parameter
                .trim_matches([' ', '\t'])
                .split_once('=')
                .is_some_and(|(name, value)| {
                    is_token(name) && (is_token(value) || is_plain_quoted_string(value))
                })
        })
}

/// A `quoted-string` without escapes or non-ASCII text.
fn is_plain_quoted_string(value: &str) -> bool {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .is_some_and(|inner| {
            inner
                .bytes()
                .all(|b| b == b' ' || (b.is_ascii_graphic() && b != b'"' && b != b'\\'))
        })
}

/// An RFC 9110 `token`.
fn is_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// `Some` when `extractSignature` would read the body as a POST policy
/// form and `parseMultipartSignature` would find a policy signature field
/// missing. `None` for anything else: another content type, a form busboy
/// or `@fastify/multipart` would reject or read differently, or a form
/// with every signature field (a signed request).
async fn unsigned_post_policy(headers: &HeaderMap, body: Body) -> Option<()> {
    let content_type = headers.get(header::CONTENT_TYPE)?.to_str().ok()?;
    // `@fastify/multipart` registers its parser for this exact media type.
    if content_type.split(';').next()?.trim() != "multipart/form-data" {
        return None;
    }
    let body = to_bytes(body, MAX_FORM_BYTES).await.ok()?;
    let form = multipart::read_form(content_type, &body)?;
    // busboy keeps adding the parts after the file to `data.fields` while
    // `extractSignature` reads them, so only a form that ends with its file
    // part reads the same way every time.
    if !form.closed_after_file || form.fields.len() > MAX_FORM_FIELDS {
        return None;
    }
    let mut names: Vec<&str> = form.fields.iter().map(|(name, _)| name.as_str()).collect();
    names.extend(form.file_name());
    // busboy decodes non-ASCII names by charset; that is not ported.
    if names
        .iter()
        .any(|name| !name.is_ascii() || PROTOTYPE_KEYS.contains(name))
        || form
            .fields
            .iter()
            .any(|(_, value)| value.len() > MAX_FIELD_BYTES)
    {
        return None;
    }
    let count = names.len();
    names.sort_unstable();
    names.dedup();
    // A repeated name becomes an array in `data.fields`.
    if names.len() != count {
        return None;
    }
    // Without a file part `req.file()` resolves to `undefined` and no field
    // reaches the `FormData`. A file part not named `file` is appended as
    // the string `undefined`.
    let Some(file) = form.file_name() else {
        return Some(());
    };
    let signed = POLICY_SIGNATURE_FIELDS
        .iter()
        .all(|field| *field == file || form.fields.iter().any(|(name, _)| name == field));
    (!signed).then_some(())
}

/// What an S3 path addresses once find-my-way matches it to a route.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    /// `/s3` or `/s3/`: only `GET` has a route.
    Service,
    /// `/s3/:Bucket` or `/s3/:Bucket/`.
    Bucket,
    /// `/s3/:Bucket/*` with a non-empty wildcard.
    Object,
}

/// Paths find-my-way matches to an S3 route without its not-found or
/// bad-URL answer: `/s3`, `/s3/`, or a non-empty bucket of at most
/// `maxParamLength` characters followed by an optional key; the bucket and
/// the key must have percent-encoding that decodes to UTF-8.
fn target(path: &str) -> Option<Target> {
    let rest = path.strip_prefix(KONG_PREFIX)?.strip_prefix("/s3")?;
    if rest.is_empty() || rest == "/" {
        return Some(Target::Service);
    }
    let rest = rest.strip_prefix('/')?;
    let (bucket, key) = rest.split_once('/').unwrap_or((rest, ""));
    if bucket.is_empty()
        || bucket.len() > MAX_PARAM_LENGTH
        || !decodes_to_utf8(bucket)
        || !decodes_to_utf8(key)
    {
        return None;
    }
    Some(if key.is_empty() {
        Target::Bucket
    } else {
        Target::Object
    })
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

/// `formatS3ErrorResponse` serialized by the `xmlParser` plugin. A `HEAD`
/// response keeps the `Content-Length` of that body and sends none (Node's
/// `http` drops it).
fn s3_error(
    method: &Method,
    status: StatusCode,
    resource: &str,
    code: &str,
    message: &str,
) -> Response {
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
    let content_type = (header::CONTENT_TYPE, "application/xml; charset=utf-8");
    if method == Method::HEAD {
        let length = (header::CONTENT_LENGTH, body.len().to_string());
        return (status, [content_type], [length], ()).into_response();
    }
    (status, [content_type], body).into_response()
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
        send_body(method, path, extra, String::new()).await
    }

    async fn send_body(
        method: &str,
        path: &str,
        extra: &[(header::HeaderName, &str)],
        body: String,
    ) -> (StatusCode, HeaderMap, String) {
        let mut request = axum::http::Request::builder().method(method).uri(path);
        for (name, value) in extra {
            request = request.header(name, *value);
        }
        let response = router_with_state(StorageState::default())
            .oneshot(request.body(Body::from(body)).unwrap())
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

    const ROUTES: [(&str, &str, &str); 33] = [
        ("PUT", "/s3/photos", "photos"),
        ("PUT", "/s3/photos/", "photos"),
        ("PUT", "/s3/photos?lifecycle", "photos"),
        ("PUT", "/s3/photos/a/b.png", "photos/a/b.png"),
        (
            "PUT",
            "/s3/photos/a.png?uploadId=u1&partNumber=1",
            "photos/a.png",
        ),
        (
            "PUT",
            "/s3/photos/a.png?partNumber=1&uploadId=u1",
            "photos/a.png",
        ),
        ("PUT", "/s3/photos//a.png", "photos/a.png"),
        ("PUT", "/s3/ph%6Ftos/a%20b", "ph%6Ftos/a%20b"),
        ("GET", "/s3", ""),
        ("GET", "/s3/", ""),
        ("GET", "/s3/photos", "photos"),
        ("GET", "/s3/photos/", "photos"),
        ("GET", "/s3/photos?lifecycle", "photos"),
        ("GET", "/s3/photos?list-type=2&prefix=a%2F", "photos"),
        ("GET", "/s3/photos?location", "photos"),
        ("GET", "/s3/photos/?uploads", "photos"),
        ("GET", "/s3/photos?versioning", "photos"),
        ("POST", "/s3/photos?delete", "photos"),
        ("POST", "/s3/photos/", "photos"),
        ("POST", "/s3/photos/a.png?uploads", "photos/a.png"),
        ("POST", "/s3/photos/a.png?uploadId=u1", "photos/a.png"),
        ("POST", "/s3/photos/a/b.png", "photos/a/b.png"),
        ("HEAD", "/s3/photos", "photos"),
        ("HEAD", "/s3/photos/", "photos"),
        ("HEAD", "/s3/photos/a/b.png", "photos/a/b.png"),
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
            let expected = missing_signature(resource);
            if method == "HEAD" {
                assert_eq!(body, "", "{path}");
                assert_eq!(
                    headers[header::CONTENT_LENGTH],
                    expected.len().to_string(),
                    "{path}"
                );
            } else {
                assert_eq!(body, expected, "{method} {path}");
            }
        }
    }

    #[tokio::test]
    async fn bodiless_posts_are_access_denied() {
        let (status, _, _) = send(
            "POST",
            "/storage/v1/s3/photos?delete",
            &[(header::CONTENT_LENGTH, "0")],
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    const FORM_TYPE: &str = "multipart/form-data; boundary=----b";

    fn form(fields: &[&str], file: Option<&str>) -> String {
        let mut body = String::new();
        for name in fields {
            body.push_str(&format!(
                "------b\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\nv\r\n"
            ));
        }
        if let Some(name) = file {
            body.push_str(&format!(
                "------b\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"a.txt\"\r\n\
                 Content-Type: text/plain\r\n\r\nhello\r\n"
            ));
        }
        body.push_str("------b--\r\n");
        body
    }

    async fn post_form(path: &str, body: String) -> (StatusCode, String) {
        let (status, _, body) = send_body(
            "POST",
            &format!("{KONG_PREFIX}{path}"),
            &[(header::CONTENT_TYPE, FORM_TYPE)],
            body,
        )
        .await;
        (status, body)
    }

    #[tokio::test]
    async fn unsigned_post_policies_are_invalid_signatures() {
        let invalid = format!(
            "{XML_DECLARATION}<Error xmlns=\"{S3_XML_NAMESPACE}\"><Resource>photos</Resource>\
             <Code>InvalidSignature</Code><Message>Invalid signature format</Message></Error>"
        );
        for body in [
            form(&["key", "Content-Type"], Some("file")),
            form(&["key"], None),
            form(
                &["X-Amz-Credential", "X-Amz-Signature", "Policy"],
                Some("file"),
            ),
            // Without a file part no field reaches the `FormData`.
            form(
                &[
                    "X-Amz-Credential",
                    "X-Amz-Signature",
                    "Policy",
                    "X-Amz-Date",
                ],
                None,
            ),
        ] {
            assert_eq!(
                post_form("/s3/photos", body.clone()).await,
                (StatusCode::BAD_REQUEST, invalid.clone()),
                "{body}"
            );
        }
    }

    #[tokio::test]
    async fn unported_post_bodies_stay_501() {
        let many: Vec<String> = (0..=MAX_FORM_FIELDS).map(|i| format!("f{i}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let mut trailing = form(&["key"], Some("file"));
        trailing.truncate(trailing.len() - "------b--\r\n".len());
        trailing.push_str(
            "------b\r\nContent-Disposition: form-data; name=\"x\"\r\n\r\nv\r\n------b--\r\n",
        );
        for body in [
            // Every signature field: a signed request.
            form(
                &[
                    "X-Amz-Credential",
                    "X-Amz-Signature",
                    "Policy",
                    "X-Amz-Date",
                ],
                Some("file"),
            ),
            // The file part is appended as the string `undefined`.
            form(
                &["X-Amz-Credential", "X-Amz-Signature", "X-Amz-Date"],
                Some("Policy"),
            ),
            form(&many, Some("file")),
            form(&["key", "key"], Some("file")),
            form(&["key"], Some("key")),
            form(&["__proto__"], Some("file")),
            trailing,
            "garbage".to_owned(),
            String::new(),
        ] {
            assert_eq!(
                post_form("/s3/photos", body.clone()).await.0,
                StatusCode::NOT_IMPLEMENTED,
                "{body}"
            );
        }
        for content_type in ["application/xml", "Multipart/Form-Data; boundary=----b"] {
            let (status, _, _) = send_body(
                "POST",
                "/storage/v1/s3/photos?delete",
                &[(header::CONTENT_TYPE, content_type)],
                form(&["key"], Some("file")),
            )
            .await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{content_type}");
        }
        let (status, _, _) = send(
            "POST",
            "/storage/v1/s3/photos?delete",
            &[(header::CONTENT_LENGTH, "12")],
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn object_puts_with_a_body_are_access_denied() {
        for (content_type, extra) in [
            ("application/octet-stream", None),
            ("image/png", None),
            ("application/json", None),
            ("text/plain; charset=utf-8", None),
            (
                "application/xml",
                Some((
                    header::HeaderName::from_static("x-amz-copy-source"),
                    "photos/b.png",
                )),
            ),
            ("text/xml; charset=\"utf-8\"", None),
        ] {
            let mut headers = vec![(header::CONTENT_TYPE, content_type)];
            headers.extend(extra);
            let (status, _, body) = send_body(
                "PUT",
                "/storage/v1/s3/photos/a.png?uploadId=u1&partNumber=2",
                &headers,
                "not xml {".to_owned(),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{content_type}");
            assert_eq!(body, missing_signature("photos/a.png"), "{content_type}");
        }
    }

    #[tokio::test]
    async fn unported_put_bodies_stay_501() {
        for (path, extra) in [
            // `PUT /:Bucket` parses XML, JSON and text bodies.
            (
                "/s3/photos",
                vec![(header::CONTENT_TYPE, "application/xml")],
            ),
            ("/s3/photos?lifecycle", vec![(header::CONTENT_LENGTH, "4")]),
            // A form is read by `extractSignature`.
            ("/s3/photos/a.png", vec![(header::CONTENT_TYPE, FORM_TYPE)]),
            (
                "/s3/photos/a.png",
                vec![(header::CONTENT_TYPE, "Multipart/Mixed; boundary=x")],
            ),
            // No or a malformed `Content-Type` with a body.
            ("/s3/photos/a.png", vec![(header::CONTENT_LENGTH, "4")]),
            ("/s3/photos/a.png", vec![(header::CONTENT_TYPE, "png")]),
            (
                "/s3/photos/a.png",
                vec![(header::CONTENT_TYPE, " image/png")],
            ),
            (
                "/s3/photos/a.png",
                vec![(header::CONTENT_TYPE, "image/png; charset")],
            ),
            (
                "/s3/photos/a.png",
                vec![(header::CONTENT_TYPE, "image/png; a=\"b")],
            ),
        ] {
            let (status, _, _) = send_body(
                "PUT",
                &format!("{KONG_PREFIX}{path}"),
                &extra,
                "body".to_owned(),
            )
            .await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{path} {extra:?}");
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
            ("HEAD", "/s3".to_owned(), vec![]),
            ("POST", "/s3/".to_owned(), vec![]),
            ("DELETE", "/s3".to_owned(), vec![]),
            ("PUT", "/s3".to_owned(), vec![]),
            ("PUT", "/s3/".to_owned(), vec![]),
            ("PATCH", "/s3/photos".to_owned(), vec![]),
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
