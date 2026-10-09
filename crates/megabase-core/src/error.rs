use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

/// Body of every response for behavior Megabase does not implement yet
/// (GOAL.md section 3, rule 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MegabaseNotImplemented {
    pub code: &'static str,
    pub component: &'static str,
    pub unit: String,
    pub message: String,
}

impl MegabaseNotImplemented {
    pub const CODE: &'static str = "MEGABASE_NOT_IMPLEMENTED";

    pub fn new(component: &'static str, unit: impl Into<String>) -> Self {
        let unit = unit.into();
        let message = format!("{unit} is not implemented by Megabase yet");
        Self {
            code: Self::CODE,
            component,
            unit,
            message,
        }
    }
}

impl IntoResponse for MegabaseNotImplemented {
    fn into_response(self) -> Response {
        (StatusCode::NOT_IMPLEMENTED, Json(self)).into_response()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not implemented: {}", .0.unit)]
    NotImplemented(MegabaseNotImplemented),
    #[error("configuration error: {0}")]
    Config(String),
}

impl From<MegabaseNotImplemented> for Error {
    fn from(value: MegabaseNotImplemented) -> Self {
        Error::NotImplemented(value)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
