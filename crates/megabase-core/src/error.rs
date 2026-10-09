// Megabase Core - Error types
// Ported from Supabase components (Apache-2.0, MIT licenses - see NOTICE)

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MegabaseNotImplemented {
    pub code: &'static str,
    pub component: &'static str,
    pub unit: String,
    pub message: String,
}

impl MegabaseNotImplemented {
    pub const CODE: &'static str = "MEGABASE_NOT_IMPLEMENTED";

    pub fn new(
        component: &'static str,
        unit: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: Self::CODE,
            component,
            unit: unit.into(),
            message: message.into(),
        }
    }

    pub fn rest(unit: impl Into<String>) -> Self {
        Self::new(
            "rest",
            unit,
            "This REST API endpoint is not yet implemented",
        )
    }

    pub fn auth(unit: impl Into<String>) -> Self {
        Self::new("auth", unit, "This Auth endpoint is not yet implemented")
    }

    pub fn realtime(unit: impl Into<String>) -> Self {
        Self::new(
            "realtime",
            unit,
            "This Realtime feature is not yet implemented",
        )
    }

    pub fn storage(unit: impl Into<String>) -> Self {
        Self::new(
            "storage",
            unit,
            "This Storage endpoint is not yet implemented",
        )
    }

    pub fn functions(unit: impl Into<String>) -> Self {
        Self::new(
            "functions",
            unit,
            "This Edge Functions endpoint is not yet implemented",
        )
    }

    pub fn pooler(unit: impl Into<String>) -> Self {
        Self::new("pooler", unit, "This Pooler feature is not yet implemented")
    }

    pub fn meta(unit: impl Into<String>) -> Self {
        Self::new(
            "meta",
            unit,
            "This Postgres Meta endpoint is not yet implemented",
        )
    }

    pub fn studio(unit: impl Into<String>) -> Self {
        Self::new(
            "studio",
            unit,
            "This Studio endpoint is not yet implemented",
        )
    }
}

impl IntoResponse for MegabaseNotImplemented {
    fn into_response(self) -> Response {
        (StatusCode::NOT_IMPLEMENTED, Json(self)).into_response()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Not implemented: {0:?}")]
    NotImplemented(MegabaseNotImplemented),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Configuration error: {0}")]
    Config(String),
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        match self {
            Error::NotImplemented(e) => e.into_response(),
            Error::Internal(msg) => {
                let body = serde_json::json!({
                    "error": "internal_error",
                    "message": msg
                });
                (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
            }
            Error::Config(msg) => {
                let body = serde_json::json!({
                    "error": "config_error",
                    "message": msg
                });
                (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
            }
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
