//! PostgREST-compatible REST API (`/rest/v1`) for Megabase.
//!
//! Target behavior: PostgREST, vendor/postgrest (MIT), pinned in `vendor/`.
//! No unit is implemented yet; every request returns the structured
//! `MEGABASE_NOT_IMPLEMENTED` 501. When logic is ported, each file gets a
//! header naming the upstream repository, path and license.

use axum::Router;

pub const COMPONENT: &str = "rest";

pub fn router() -> Router {
    megabase_core::not_implemented_router(COMPONENT)
}

/// Walk a PostgREST-style query string without interpreting operators.
///
/// Filter and query-param parsing is not implemented yet (Level 1 REST).
/// This walker exists so the `rest_query` cargo-fuzz target can hit the crate
/// without inventing operator semantics. Replace it when those units land.
#[must_use]
pub fn walk_query_string(query: &str) -> Vec<(&str, Option<&str>)> {
    if query.is_empty() {
        return Vec::new();
    }
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| match part.split_once('=') {
            Some((key, value)) => (key, Some(value)),
            None => (part, None),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_query_string_splits_pairs_without_decoding() {
        assert!(walk_query_string("").is_empty());
        assert_eq!(
            walk_query_string("id=eq.1&select=name"),
            vec![("id", Some("eq.1")), ("select", Some("name"))]
        );
        assert_eq!(walk_query_string("or"), vec![("or", None)]);
        assert_eq!(walk_query_string("&&id=eq.1&"), vec![("id", Some("eq.1"))]);
    }
}
