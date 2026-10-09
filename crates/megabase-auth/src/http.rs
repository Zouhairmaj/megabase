// Ported from supabase/auth internal/api/apierrors/apierrors.go and
// internal/api/errors.go (MIT), pin v2.197.0.

//! Legacy GoTrue HTTP errors (no `X-Supabase-Api-Version` header).

use axum::{
    http::{header::CONTENT_TYPE, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Serialize;

pub const ERROR_CODE_HEADER: &str = "x-sb-error-code";

#[derive(Debug, Clone)]
pub struct AuthError {
    pub status: u16,
    pub error_code: &'static str,
    pub message: String,
}

#[derive(Serialize)]
struct AuthErrorBody<'a> {
    code: u16,
    error_code: &'a str,
    msg: &'a str,
}

impl AuthError {
    pub fn new(status: u16, error_code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            error_code,
            message: message.into(),
        }
    }

    pub fn no_authorization() -> Self {
        Self::new(
            401,
            "no_authorization",
            "This endpoint requires a valid Bearer token",
        )
    }

    pub fn bad_jwt(detail: impl std::fmt::Display) -> Self {
        Self::new(
            403,
            "bad_jwt",
            format!("invalid JWT: unable to parse or verify signature, {detail}"),
        )
    }

    pub fn not_admin() -> Self {
        Self::new(403, "not_admin", "User not allowed")
    }

    pub fn feature_disabled(message: &'static str) -> Self {
        Self::new(404, "feature_disabled", message)
    }

    pub fn validation(status: u16, message: impl Into<String>) -> Self {
        Self::new(status, "validation_failed", message)
    }

    pub fn not_found(error_code: &'static str, message: impl Into<String>) -> Self {
        Self::new(404, error_code, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(500, "unexpected_failure", message)
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = AuthErrorBody {
            code: self.status,
            error_code: self.error_code,
            msg: &self.message,
        };
        let mut response = (
            status,
            [(CONTENT_TYPE, HeaderValue::from_static("application/json"))],
            serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec()),
        )
            .into_response();
        if let Ok(value) = HeaderValue::from_str(self.error_code) {
            response.headers_mut().insert(ERROR_CODE_HEADER, value);
        }
        response
    }
}

pub fn json_ok(value: serde_json::Value) -> Response {
    (
        StatusCode::OK,
        [(CONTENT_TYPE, HeaderValue::from_static("application/json"))],
        serde_json::to_vec(&value).unwrap_or_else(|_| b"null".to_vec()),
    )
        .into_response()
}

pub fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}
