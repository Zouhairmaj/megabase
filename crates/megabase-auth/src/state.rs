// Ported from supabase/auth internal/conf/configuration.go (MIT), pin v2.197.0.

//! Runtime flags the admin routes read. Names match GoTrue's `GOTRUE_*` env.

use megabase_core::jwt::Hs256;

use crate::config::AuthConfig;
use crate::error::ConfigError;
use crate::store::Backend;

const DEFAULT_ADMIN_ROLES: &[&str] = &["service_role", "supabase_admin"];

#[derive(Clone)]
pub struct AuthState {
    pub jwt: Option<Hs256>,
    pub database_url: Option<String>,
    pub oauth_server_enabled: bool,
    pub custom_oauth_enabled: bool,
    pub admin_roles: Vec<String>,
    /// Signup and settings flags. Unset `GOTRUE_*` follows the reference stack.
    pub config: AuthConfig,
    /// Long-lived client for signup and logout. Admin routes open their own.
    pub backend: Backend,
}

impl AuthState {
    pub fn from_env() -> Self {
        Self::try_from_env().expect("invalid Auth configuration")
    }

    pub fn try_from_env() -> Result<Self, ConfigError> {
        Self::try_from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        Self::try_from_lookup(lookup).expect("invalid Auth configuration")
    }

    pub fn try_from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let config = AuthConfig::from_lookup(&lookup)?;
        let jwt = lookup("JWT_SECRET")
            .filter(|secret| !secret.is_empty())
            .and_then(|secret| Hs256::new(secret.into_bytes()).ok());
        Ok(Self {
            jwt,
            database_url: lookup("DATABASE_URL").filter(|url| !url.is_empty()),
            oauth_server_enabled: env_bool(lookup("GOTRUE_OAUTH_SERVER_ENABLED"), false),
            custom_oauth_enabled: env_bool(lookup("GOTRUE_CUSTOM_OAUTH_ENABLED"), true),
            admin_roles: admin_roles(&lookup),
            config,
            backend: Backend::none(),
        })
    }

    /// Reference-stack signup flags, GoTrue admin defaults, no key, no database.
    /// Gateway tests and `create_router` use this.
    pub fn reference() -> Self {
        Self::from_lookup(|_| None)
    }

    /// Signup tests build a custom config. Admin flags stay at the unset defaults.
    pub fn new(config: AuthConfig, jwt: Option<Hs256>, backend: Backend) -> Self {
        let defaults = Self::from_lookup(|_| None);
        Self {
            jwt,
            database_url: None,
            oauth_server_enabled: defaults.oauth_server_enabled,
            custom_oauth_enabled: defaults.custom_oauth_enabled,
            admin_roles: defaults.admin_roles,
            config,
            backend,
        }
    }

    pub fn is_admin_role(&self, role: Option<&str>) -> bool {
        role.is_some_and(|role| self.admin_roles.iter().any(|allowed| allowed == role))
    }
}

fn admin_roles(lookup: &impl Fn(&str) -> Option<String>) -> Vec<String> {
    lookup("GOTRUE_JWT_ADMIN_ROLES")
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|role| !role.is_empty())
                .map(str::to_string)
                .collect()
        })
        .filter(|roles: &Vec<String>| !roles.is_empty())
        .unwrap_or_else(|| {
            DEFAULT_ADMIN_ROLES
                .iter()
                .map(|s| (*s).to_string())
                .collect()
        })
}

fn env_bool(raw: Option<String>, default: bool) -> bool {
    match raw.as_deref().map(str::trim) {
        Some(value) if value.eq_ignore_ascii_case("true") || value == "1" => true,
        Some(value) if value.eq_ignore_ascii_case("false") || value == "0" => false,
        _ => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_gotrue() {
        let state = AuthState::from_lookup(|_| None);
        assert!(state.jwt.is_none());
        assert!(state.database_url.is_none());
        assert!(!state.oauth_server_enabled);
        assert!(state.custom_oauth_enabled);
        assert_eq!(
            state.admin_roles,
            ["service_role", "supabase_admin"].map(str::to_string)
        );
        assert!(state.is_admin_role(Some("service_role")));
        assert!(!state.is_admin_role(Some("anon")));
        assert!(state.config.mailer_autoconfirm);
        assert!(state.config.email_enabled);
    }

    #[test]
    fn parses_gotrue_flags() {
        let state = AuthState::from_lookup(|key| match key {
            "GOTRUE_OAUTH_SERVER_ENABLED" => Some("true".into()),
            "GOTRUE_CUSTOM_OAUTH_ENABLED" => Some("false".into()),
            "GOTRUE_JWT_ADMIN_ROLES" => Some("service_role, supabase_admin ".into()),
            _ => None,
        });
        assert!(state.oauth_server_enabled);
        assert!(!state.custom_oauth_enabled);
        assert_eq!(state.admin_roles.len(), 2);
    }
}
