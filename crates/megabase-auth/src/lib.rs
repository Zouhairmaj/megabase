//! Supabase Auth-compatible API (`/auth/v1`) for Megabase.
//!
//! Target behavior: Supabase Auth, vendor/auth (MIT), pinned in `vendor/`.
//! No unit is implemented yet; every request returns the structured
//! `MEGABASE_NOT_IMPLEMENTED` 501. When logic is ported, each file gets a
//! header naming the upstream repository, path and license.

use axum::Router;

pub const COMPONENT: &str = "auth";

pub fn router() -> Router {
    megabase_core::not_implemented_router(COMPONENT)
}
