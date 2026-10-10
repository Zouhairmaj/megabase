//! PostgREST-compatible REST API (`/rest/v1`) for Megabase.
//!
//! Target behavior: PostgREST, vendor/postgrest (MIT), pinned in `vendor/`.
//! `GET /rest/v1/{relation}` runs for the horizontal filters in
//! [`specs/rest/filtering.md`](../../../specs/rest/filtering.md) and the query
//! parameters in [`specs/rest/query-params.md`](../../../specs/rest/query-params.md).
//! Every other REST request returns `MEGABASE_NOT_IMPLEMENTED`.

mod db;
mod filter;
mod params;
mod query;
mod read;

use std::sync::Arc;

use axum::Router;
use megabase_core::{Config, Hs256};

pub use db::connect;
pub use query::interpret_query;

pub const COMPONENT: &str = "rest";

/// Pool and JWT verifier for REST reads.
#[derive(Clone)]
pub struct RestState {
    pool: Option<sqlx::PgPool>,
    jwt: Option<Arc<Hs256>>,
}

impl RestState {
    /// Verifier from `config`, with no pool.
    ///
    /// A missing `JWT_SECRET` leaves the verifier empty. Anonymous reads then
    /// use the `anon` role. A bearer token in that process is `PGRST300`.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        let jwt = config.jwt_hs256().ok().map(Arc::new);
        Self { pool: None, jwt }
    }

    /// Attach a pool opened by [`connect`].
    #[must_use]
    pub fn with_pool(mut self, pool: sqlx::PgPool) -> Self {
        self.pool = Some(pool);
        self
    }

    /// Close the pool. A state with no pool returns immediately.
    pub async fn close(&self) {
        if let Some(pool) = &self.pool {
            pool.close().await;
        }
    }
}

/// REST router. `state` carries the pool and the HS256 verifier.
pub fn router(state: RestState) -> Router {
    read::router(state)
}

/// Walk a PostgREST-style query string without interpreting operators.
///
/// Splits on `&` and `=` and does not percent-decode. The `rest_query` fuzzer
/// calls this together with [`interpret_query`].
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
