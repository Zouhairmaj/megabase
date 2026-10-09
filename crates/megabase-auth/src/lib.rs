// Megabase Auth - GoTrue-compatible authentication API
// Ported from Supabase Auth (GoTrue) (Apache-2.0 license) - see NOTICE
// Upstream: vendor/auth

use axum::{
    extract::Path,
    http::Method,
    routing::{any, get, post},
    Router,
};
use megabase_core::MegabaseNotImplemented;

pub fn router() -> Router {
    Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .route("/signup", post(signup_handler))
        .route("/token", post(token_handler))
        .route("/user", any(user_handler))
        .route("/logout", post(logout_handler))
        .route("/recover", post(recover_handler))
        .route("/verify", any(verify_handler))
        .route("/otp", post(otp_handler))
        .route("/magiclink", post(magiclink_handler))
        .route("/authorize", get(authorize_handler))
        .route("/callback", any(callback_handler))
        .nest("/admin", admin_router())
        .fallback(fallback_handler)
}

fn admin_router() -> Router {
    Router::new()
        .route("/users", any(admin_users_handler))
        .route("/users/{user_id}", any(admin_user_handler))
        .route("/generate_link", post(admin_generate_link_handler))
        .route("/audit", any(admin_audit_handler))
        .fallback(admin_fallback_handler)
}

async fn root_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth("GET /auth/v1/")
}

async fn health_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth("GET /auth/v1/health")
}

async fn signup_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth("POST /auth/v1/signup")
}

async fn token_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth("POST /auth/v1/token")
}

async fn user_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth(format!("{} /auth/v1/user", method))
}

async fn logout_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth("POST /auth/v1/logout")
}

async fn recover_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth("POST /auth/v1/recover")
}

async fn verify_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth(format!("{} /auth/v1/verify", method))
}

async fn otp_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth("POST /auth/v1/otp")
}

async fn magiclink_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth("POST /auth/v1/magiclink")
}

async fn authorize_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth("GET /auth/v1/authorize")
}

async fn callback_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth(format!("{} /auth/v1/callback", method))
}

async fn admin_users_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth(format!("{} /auth/v1/admin/users", method))
}

async fn admin_user_handler(method: Method, Path(user_id): Path<String>) -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth(format!("{} /auth/v1/admin/users/{}", method, user_id))
}

async fn admin_generate_link_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth("POST /auth/v1/admin/generate_link")
}

async fn admin_audit_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth(format!("{} /auth/v1/admin/audit", method))
}

async fn admin_fallback_handler(method: Method, uri: axum::http::Uri) -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth(format!("{} /auth/v1/admin{}", method, uri.path()))
}

async fn fallback_handler(method: Method, uri: axum::http::Uri) -> MegabaseNotImplemented {
    MegabaseNotImplemented::auth(format!("{} /auth/v1{}", method, uri.path()))
}
