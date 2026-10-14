//! Kong's `key-auth` plugin as the pinned self-hosted stack attaches it.
//!
//! `vendor/supabase/docker/volumes/api/kong.yml` (Apache-2.0) runs
//! `key-auth` on the Auth, REST, GraphQL, Realtime and Postgres Meta services.
//! A request on one of those routes without an `apikey` header or query
//! parameter gets Kong's 401 before it reaches the service. Storage, Edge
//! Functions, OAuth discovery and Studio have no `key-auth`.
//!
//! Kong picks the longest matching route path, so the open Auth routes
//! (`auth-v1-open*`) and the terminated Realtime admin routes win over the
//! broader `/auth/v1/` and `/realtime/v1/` routes. Matching is a plain prefix
//! test, as in [`crate::match_gateway_route`].
//!
//! Megabase only checks that a key is present. The pinned file maps the
//! `SUPABASE_*_KEY` values to consumers; Megabase is not given those values,
//! so a present but unknown key still reaches the service. Spec:
//! `specs/server/key-auth.md`.

use axum::http::HeaderMap;

/// Route path prefixes whose Kong service lists `key-auth`.
pub const KEY_AUTH_ROUTES: &[&str] = &[
    "/auth/v1/",
    "/rest/v1/",
    "/graphql/v1",
    "/realtime/v1/",
    "/pg/",
];

/// Longer Kong route paths under [`KEY_AUTH_ROUTES`] whose service has no
/// `key-auth`: the open Auth routes and the `request-termination` Realtime
/// admin routes.
pub const KEY_AUTH_EXEMPT_ROUTES: &[&str] = &[
    "/auth/v1/verify",
    "/auth/v1/callback",
    "/auth/v1/authorize",
    "/auth/v1/.well-known/jwks.json",
    "/auth/v1/sso/saml/acs",
    "/auth/v1/sso/saml/metadata",
    "/realtime/v1/api/openapi",
    "/realtime/v1/api/tenants",
];

/// Whether Kong runs `key-auth` for a request on `path`.
#[must_use]
pub fn key_auth_applies(path: &str) -> bool {
    KEY_AUTH_ROUTES
        .iter()
        .any(|prefix| path.starts_with(prefix))
        && !KEY_AUTH_EXEMPT_ROUTES
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

/// Kong's 401 when `key-auth` applies to `path` and the request has no key.
pub fn reject_missing_key(
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
) -> Option<axum::response::Response> {
    if key_auth_applies(path) && megabase_core::kong::api_key(headers, query).is_none() {
        Some(megabase_core::kong::no_api_key())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_auth_follows_kong_routes() {
        for path in [
            "/auth/v1/health",
            "/auth/v1/settings",
            "/auth/v1/token",
            "/rest/v1/",
            "/rest/v1/todos",
            "/graphql/v1",
            "/realtime/v1/api/ping",
            "/realtime/v1/websocket",
            "/pg/health",
        ] {
            assert!(key_auth_applies(path), "{path}");
        }
        for path in [
            "/auth/v1/verify",
            "/auth/v1/authorize",
            "/auth/v1/callback",
            "/auth/v1/.well-known/jwks.json",
            "/auth/v1/sso/saml/acs",
            "/auth/v1/sso/saml/metadata",
            "/realtime/v1/api/tenants",
            "/realtime/v1/api/openapi",
            "/storage/v1/bucket",
            "/functions/v1/hello",
            "/.well-known/oauth-authorization-server",
            "/auth/v1",
            "/",
        ] {
            assert!(!key_auth_applies(path), "{path}");
        }
    }

    #[test]
    fn missing_key_is_rejected_only_where_key_auth_applies() {
        let headers = HeaderMap::new();
        assert!(reject_missing_key("/auth/v1/health", None, &headers).is_some());
        assert!(reject_missing_key("/auth/v1/health", Some("apikey=k"), &headers).is_none());
        assert!(reject_missing_key("/auth/v1/verify", None, &headers).is_none());
        let mut with_key = HeaderMap::new();
        with_key.insert("apikey", "k".parse().unwrap());
        assert!(reject_missing_key("/pg/health", None, &with_key).is_none());
    }
}
