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

/// Go `%q` for logout scope (double quotes, common escapes).
pub fn go_quote(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", ch as u32));
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_quote_matches_printf() {
        assert_eq!(go_quote("nope"), "\"nope\"");
        assert_eq!(go_quote("a\"b"), "\"a\\\"b\"");
    }
}
