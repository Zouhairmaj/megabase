//! Supabase Auth-compatible API (`/auth/v1`) for Megabase.
//!
//! Target behavior: Supabase Auth, vendor/auth (MIT), pinned in `vendor/`.
//! Issue #6 serves the first `/auth/v1/admin` GET/DELETE batch. Issue #7
//! serves the admin user, SSO, OAuth client, custom-provider, and
//! generate-link reads and creates in that epic. Issue #15 serves health,
//! settings, autoconfirm email signup, and logout. Issue #17 serves
//! `POST /token` for the password and refresh-token grants. Issue #21
//! serves `GET`/`POST /verify` for signup, invite, recovery, and email
//! change. Issue #19 serves `GET` and `PUT /user`, identity authorize and
//! unlink, and the OAuth grant list and revoke. Issue #339 runs the
//! `pg-functions://` custom access token hook at every token issuance.
//! Issue #47 serves the `/factors` authentication, configuration, and AAL
//! gates, factor unenroll, and the recovery-code status and delete routes.
//! Issue #46 serves `GET`/`POST /callback` up to the provider code exchange
//! and `POST /magiclink` and `POST /otp` up to mail and SMS delivery.
//! Issue #382 serves `POST /recover` through GoTrue's no-op mail client and
//! an unreachable SMTP host. Issue #401 serves `POST /invite` the same way.
//! Issue #55 serves the JWKS and OIDC / OAuth discovery documents.
//! Every other Auth path returns `MEGABASE_NOT_IMPLEMENTED`. Database objects
//! listed in `specs/auth/database.md` are installed when `DATABASE_URL` is
//! set, after the standard API roles in `specs/core/roles.md` and the
//! platform schemas in `specs/core/platform-schemas.md`.

mod admin;
mod admin_batch2;
mod config;
mod confirm;
mod error;
mod external;
mod factors;
mod gojson;
mod hooks;
mod http;
mod invite;
mod jsonutil;
mod otp;
mod platform;
mod ratelimit;
mod recover;
mod roles;
mod routes;
mod schema;
mod state;
mod store;
mod token;
mod user;
mod verify;
mod well_known;

pub use config::AuthConfig;
pub use platform::{install_platform, PlatformError};
pub use roles::{install_roles, RolesError};
pub use schema::{install_schema, SchemaError};
pub use state::{AuthState, MailConfig};
pub use store::Backend;

use axum::{extract::OriginalUri, http::Method, Router};
use megabase_core::MegabaseNotImplemented;

pub const COMPONENT: &str = "auth";

/// Routes from the environment. This does not open a database pool:
/// `AuthState::from_env` leaves `backend` empty. `megabase_server::run`
/// installs the schema, calls `Backend::connect`, then `router_with_state`.
pub fn router() -> Router {
    router_with_state(AuthState::from_env())
}

pub fn router_with_state(state: AuthState) -> Router {
    admin::router(state.clone())
        .merge(factors::router(state.clone()))
        .merge(well_known::router(state.clone()))
        .merge(routes::router(state))
        // Path match without a method handler is Axum 405; send those to the
        // same 501 as unknown paths (GOAL.md §3 rule 5).
        .method_not_allowed_fallback(not_implemented)
        .fallback(not_implemented)
}

async fn not_implemented(method: Method, OriginalUri(uri): OriginalUri) -> MegabaseNotImplemented {
    MegabaseNotImplemented::new(COMPONENT, format!("{method} {}", uri.path()))
}
