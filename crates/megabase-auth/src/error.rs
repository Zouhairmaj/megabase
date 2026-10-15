//! GoTrue HTTP errors and configuration failures.
//!
//! Ported from supabase/auth internal/api/apierrors/apierrors.go and
//! internal/api/errors.go (MIT), pin v2.197.0. The default API version
//! (no `X-Supabase-Api-Version: 2024-01-01`) uses `code` as the HTTP status
//! integer, `error_code`, and `msg`.

use axum::{
    http::{header::HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub const MAX_BODY_BYTES: usize = 1 << 20;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{key} is not a valid value: {value}")]
    Invalid { key: String, value: String },
}

pub struct GoTrueError {
    pub status: StatusCode,
    pub error_code: &'static str,
    pub message: String,
    pub weak_password: Option<Vec<&'static str>>,
}

impl GoTrueError {
    pub fn new(status: StatusCode, error_code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            error_code,
            message: message.into(),
            weak_password: None,
        }
    }

    pub fn weak_password(message: impl Into<String>, reasons: Vec<&'static str>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            error_code: "weak_password",
            message: message.into(),
            weak_password: Some(reasons),
        }
    }
}

impl IntoResponse for GoTrueError {
    fn into_response(self) -> Response {
        let mut body = json!({
            "code": self.status.as_u16(),
            "error_code": self.error_code,
            "msg": self.message,
        });
        if let Some(reasons) = &self.weak_password {
            body["weak_password"] = json!({ "reasons": reasons });
        }
        let mut response = (self.status, Json(body)).into_response();
        if let Ok(value) = HeaderValue::from_str(self.error_code) {
            response
                .headers_mut()
                .insert(HeaderName::from_static("x-sb-error-code"), value);
        }
        response
    }
}

pub fn validation(message: impl Into<String>) -> GoTrueError {
    GoTrueError::new(StatusCode::BAD_REQUEST, "validation_failed", message)
}

pub fn bad_json(detail: impl std::fmt::Display) -> GoTrueError {
    GoTrueError::new(
        StatusCode::BAD_REQUEST,
        "bad_json",
        format!("Could not parse request body as JSON: {detail}"),
    )
}

pub fn body_too_large() -> GoTrueError {
    GoTrueError::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        "request_entity_too_large",
        format!("Request body too large (max {MAX_BODY_BYTES} bytes)"),
    )
}

pub fn unexpected(message: impl Into<String>) -> GoTrueError {
    GoTrueError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "unexpected_failure",
        message,
    )
}

/// `apierrors.OAuthError`: always HTTP 400 with `error` and an
/// `error_description` that `omitempty` drops when empty. It carries no
/// `x-sb-error-code` header.
pub struct OAuthError {
    pub error: &'static str,
    pub description: String,
}

impl OAuthError {
    pub fn new(error: &'static str, description: impl Into<String>) -> Self {
        Self {
            error,
            description: description.into(),
        }
    }
}

impl IntoResponse for OAuthError {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.error });
        if !self.description.is_empty() {
            body["error_description"] = json!(self.description);
        }
        (StatusCode::BAD_REQUEST, Json(body)).into_response()
    }
}

/// Go `%q` (`strconv.Quote`): double quotes, Go escapes, and `\x`/`\u`/`\U`
/// escapes for runes Go does not print. The non-printable set is the
/// control, format, separator (other than space), private-use, and
/// noncharacter ranges of `unicode.IsPrint`; unassigned code points print.
pub fn go_quote(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{07}' => out.push_str("\\a"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0b}' => out.push_str("\\v"),
            ch if (ch as u32) < 0x20 || ch == '\u{7f}' => {
                out.push_str(&format!("\\x{:02x}", ch as u32));
            }
            ch if !go_printable(ch) => {
                let code = ch as u32;
                if code < 0x10000 {
                    out.push_str(&format!("\\u{code:04x}"));
                } else {
                    out.push_str(&format!("\\U{code:08x}"));
                }
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn go_printable(ch: char) -> bool {
    !matches!(ch as u32,
        0x80..=0xa0
        | 0xad
        | 0x600..=0x605
        | 0x61c
        | 0x6dd
        | 0x70f
        | 0x1680
        | 0x180e
        | 0x2000..=0x200f
        | 0x2028..=0x202f
        | 0x205f..=0x2064
        | 0x2066..=0x206f
        | 0x3000
        | 0xe000..=0xf8ff
        | 0xfeff
        | 0xfff9..=0xfffb
        | 0xfffe..=0xffff
        | 0xe0001
        | 0xe0020..=0xe007f
        | 0xf0000..)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_quote_matches_printf() {
        assert_eq!(go_quote("nope"), "\"nope\"");
        assert_eq!(go_quote("a\"b"), "\"a\\\"b\"");
        assert_eq!(go_quote("a\u{1}\u{7f}\u{7}"), "\"a\\x01\\x7f\\a\"");
        assert_eq!(go_quote("é\u{200b}\u{f0000}"), "\"é\\u200b\\U000f0000\"");
    }

    #[tokio::test]
    async fn oauth_error_omits_an_empty_description() {
        let response = OAuthError::new("invalid request", "").into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(response.headers().get("x-sb-error-code").is_none());
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&bytes[..], br#"{"error":"invalid request"}"#);
    }
}
