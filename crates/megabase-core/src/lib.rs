//! Shared types for Megabase: configuration, JWT, errors and the structured
//! `MEGABASE_NOT_IMPLEMENTED` fallback every component uses.

pub mod config;
pub mod error;
pub mod jwt;

use axum::{extract::OriginalUri, http::Method, Router};

pub use config::Config;
pub use error::{Error, MegabaseNotImplemented, Result};
pub use jwt::{bearer_token, Hs256, JwtClaims, JwtError};

/// A router that answers every method and path with HTTP 501.
///
/// The `unit` field is `"<METHOD> <path>"` using the path as the client sent
/// it (including the gateway prefix), so it can be matched against
/// `coverage/units.json`.
pub fn not_implemented_router(component: &'static str) -> Router {
    Router::new().fallback(
        move |method: Method, OriginalUri(uri): OriginalUri| async move {
            MegabaseNotImplemented::new(component, format!("{method} {}", uri.path()))
        },
    )
}
