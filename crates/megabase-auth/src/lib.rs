//! Supabase Auth-compatible API (`/auth/v1`) for Megabase.
//!
//! Target behavior: Supabase Auth, vendor/auth (MIT), pinned in `vendor/`.
//! Issue #6 serves the first `/auth/v1/admin` GET/DELETE batch. Every other
//! Auth path still returns `MEGABASE_NOT_IMPLEMENTED`. Database objects
//! listed in `specs/auth/database.md` are installed when `DATABASE_URL` is
//! set.

mod admin;
mod http;
mod schema;
mod state;

pub use schema::{install_schema, SchemaError};
pub use state::AuthState;

use axum::{extract::OriginalUri, http::Method, Router};
use megabase_core::MegabaseNotImplemented;

pub const COMPONENT: &str = "auth";

pub fn router() -> Router {
    router_with_state(AuthState::from_env())
}

pub fn router_with_state(state: AuthState) -> Router {
    admin::router(state).fallback(
        move |method: Method, OriginalUri(uri): OriginalUri| async move {
            MegabaseNotImplemented::new(COMPONENT, format!("{method} {}", uri.path()))
        },
    )
}
