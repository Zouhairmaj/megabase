//! Supabase Auth-compatible API (`/auth/v1`) for Megabase.
//!
//! Target behavior: Supabase Auth, vendor/auth (MIT), pinned in `vendor/`.
//! Issue #6 serves the first `/auth/v1/admin` GET/DELETE batch. Issue #15
//! serves health, settings, autoconfirm email signup, and logout. Issue #17
//! serves `POST /token` for the password and refresh-token grants. Issue #19
//! serves `GET` and `PUT /user`, identity unlink, and the OAuth grant list.
//! Every other Auth path returns `MEGABASE_NOT_IMPLEMENTED`. Database objects
//! listed in `specs/auth/database.md` are installed when `DATABASE_URL` is
//! set.

mod admin;
mod config;
mod error;
mod http;
mod jsonutil;
mod routes;
mod schema;
mod state;
mod store;
mod token;
mod user;

pub use config::AuthConfig;
pub use schema::{install_schema, SchemaError};
pub use state::AuthState;
pub use store::Backend;

use axum::{extract::OriginalUri, http::Method, Router};
use megabase_core::MegabaseNotImplemented;

pub const COMPONENT: &str = "auth";

pub fn router() -> Router {
    router_with_state(AuthState::from_env())
}

pub fn router_with_state(state: AuthState) -> Router {
    admin::router(state.clone())
        .merge(routes::router(state))
        // Path match without a method handler is Axum 405; send those to the
        // same 501 as unknown paths (GOAL.md §3 rule 5).
        .method_not_allowed_fallback(not_implemented)
        .fallback(not_implemented)
}

async fn not_implemented(method: Method, OriginalUri(uri): OriginalUri) -> MegabaseNotImplemented {
    MegabaseNotImplemented::new(COMPONENT, format!("{method} {}", uri.path()))
}
