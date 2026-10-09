//! Supabase Auth-compatible API (`/auth/v1`) for Megabase.
//!
//! Target behavior: Supabase Auth, vendor/auth (MIT), pinned in `vendor/`.
//! HTTP routes are not implemented yet; every request returns the structured
//! `MEGABASE_NOT_IMPLEMENTED` 501. Database objects listed in
//! `specs/auth/database.md` are installed when `DATABASE_URL` is set.
//! When HTTP logic is ported, each file gets a header naming the upstream
//! repository, path and license.

mod schema;

pub use schema::{install_schema, SchemaError};

use axum::Router;

pub const COMPONENT: &str = "auth";

pub fn router() -> Router {
    megabase_core::not_implemented_router(COMPONENT)
}
