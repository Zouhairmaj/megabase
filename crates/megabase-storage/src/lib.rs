//! Supabase Storage-compatible API (`/storage/v1`) for Megabase.
//!
//! Target behavior: Supabase Storage, vendor/storage (Apache-2.0), pinned in `vendor/`.
//! No unit is implemented yet; every request returns the structured
//! `MEGABASE_NOT_IMPLEMENTED` 501. When logic is ported, each file gets a
//! header naming the upstream repository, path and license.

use axum::Router;

pub const COMPONENT: &str = "storage";

pub fn router() -> Router {
    megabase_core::not_implemented_router(COMPONENT)
}
