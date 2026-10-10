// Ported from supabase/auth internal/conf/configuration.go (MIT), pin v2.197.0.

//! Runtime flags the admin routes read. Names match GoTrue's `GOTRUE_*` env.

use megabase_core::jwt::Hs256;

use crate::config::{optional_bool, AuthConfig};
use crate::error::ConfigError;
use crate::store::Backend;

const DEFAULT_ADMIN_ROLES: &[&str] = &["service_role", "supabase_admin"];

#[derive(Clone)]
pub struct AuthState {
    pub jwt: Option<Hs256>,
    pub database_url: Option<String>,
    pub oauth_server_enabled: bool,
    pub custom_oauth_enabled: bool,
    /// `GOTRUE_SECURITY_MANUAL_LINKING_ENABLED`. GoTrue default is false.
    pub manual_linking_enabled: bool,
    /// Password-update gates. Both default off, matching unset GoTrue.
    pub password_require_reauthentication: bool,
    pub password_require_current: bool,
    /// `GOTRUE_JWT_ADMIN_GROUP_NAME`. GoTrue turns an empty value into `admin`.
    pub admin_group_name: String,
    pub admin_roles: Vec<String>,
    /// `GOTRUE_URI_ALLOW_LIST`, or `ADDITIONAL_REDIRECT_URLS` when that is unset.
    /// Comma-separated redirect globs. Empty when neither variable is set.
    pub uri_allow_list: Vec<String>,
    /// `API_EXTERNAL_URL`. Reference stack default `http://localhost:8000/auth/v1`.
    pub api_external_url: String,
    pub mailer_confirmation_path: String,
    pub mailer_invite_path: String,
    pub mailer_recovery_path: String,
    pub mailer_email_change_path: String,
    pub otp_length: usize,
    pub cursor_pagination: bool,
    /// Signup and settings flags. Unset `GOTRUE_*` follows the reference stack.
    pub config: AuthConfig,
    /// Shared Auth pool. `try_from_env` leaves this empty. `megabase_server::run`
    /// replaces it with `Backend::connect` before `router_with_state`, and
    /// closes it on shutdown. Signup, logout, verify, the password and
    /// refresh-token grants, and admin routes use that pool. `router()` does
    /// not connect.
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
            oauth_server_enabled: env_bool(lookup("GOTRUE_OAUTH_SERVER_ENABLED").as_deref(), false),
            custom_oauth_enabled: env_bool(lookup("GOTRUE_CUSTOM_OAUTH_ENABLED").as_deref(), true),
            manual_linking_enabled: security_flag(
                &lookup,
                "GOTRUE_SECURITY_MANUAL_LINKING_ENABLED",
            )?,
            password_require_reauthentication: security_flag(
                &lookup,
                "GOTRUE_SECURITY_UPDATE_PASSWORD_REQUIRE_REAUTHENTICATION",
            )?,
            password_require_current: security_flag(
                &lookup,
                "GOTRUE_SECURITY_UPDATE_PASSWORD_REQUIRE_CURRENT_PASSWORD",
            )?,
            admin_group_name: admin_group_name(&lookup),
            admin_roles: admin_roles(&lookup),
            uri_allow_list: uri_allow_list(&lookup),
            api_external_url: first_nonempty(
                &lookup,
                &["API_EXTERNAL_URL", "GOTRUE_API_EXTERNAL_URL"],
                "http://localhost:8000/auth/v1",
            ),
            mailer_confirmation_path: mailer_path(&lookup, "CONFIRMATION"),
            mailer_invite_path: mailer_path(&lookup, "INVITE"),
            mailer_recovery_path: mailer_path(&lookup, "RECOVERY"),
            mailer_email_change_path: mailer_path(&lookup, "EMAIL_CHANGE"),
            otp_length: otp_length(lookup("GOTRUE_MAILER_OTP_LENGTH").as_deref()),
            cursor_pagination: env_bool(
                lookup("GOTRUE_EXPERIMENTAL_CURSOR_PAGINATION_ENABLED").as_deref(),
                false,
            ),
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
            manual_linking_enabled: defaults.manual_linking_enabled,
            password_require_reauthentication: defaults.password_require_reauthentication,
            password_require_current: defaults.password_require_current,
            admin_group_name: defaults.admin_group_name,
            admin_roles: defaults.admin_roles,
            uri_allow_list: defaults.uri_allow_list,
            api_external_url: defaults.api_external_url,
            mailer_confirmation_path: defaults.mailer_confirmation_path,
            mailer_invite_path: defaults.mailer_invite_path,
            mailer_recovery_path: defaults.mailer_recovery_path,
            mailer_email_change_path: defaults.mailer_email_change_path,
            otp_length: defaults.otp_length,
            cursor_pagination: defaults.cursor_pagination,
            config,
            backend,
        }
    }

    pub fn is_admin_role(&self, role: Option<&str>) -> bool {
        role.is_some_and(|role| self.admin_roles.iter().any(|allowed| allowed == role))
    }
}

fn security_flag(lookup: &impl Fn(&str) -> Option<String>, key: &str) -> Result<bool, ConfigError> {
    Ok(optional_bool(lookup, key)?.unwrap_or(false))
}

fn admin_group_name(lookup: &impl Fn(&str) -> Option<String>) -> String {
    lookup("GOTRUE_JWT_ADMIN_GROUP_NAME")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "admin".to_string())
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

fn first_nonempty(
    lookup: &impl Fn(&str) -> Option<String>,
    keys: &[&str],
    default: &str,
) -> String {
    keys.iter()
        .find_map(|key| lookup(key).filter(|value| !value.is_empty()))
        .unwrap_or_else(|| default.to_string())
}

fn mailer_path(lookup: &impl Fn(&str) -> Option<String>, name: &str) -> String {
    let gotrue = format!("GOTRUE_MAILER_URLPATHS_{name}");
    let docker = format!("MAILER_URLPATHS_{name}");
    first_nonempty(
        lookup,
        &[gotrue.as_str(), docker.as_str()],
        "/auth/v1/verify",
    )
}

fn uri_allow_list(lookup: &impl Fn(&str) -> Option<String>) -> Vec<String> {
    let raw = lookup("GOTRUE_URI_ALLOW_LIST")
        .filter(|value| !value.trim().is_empty())
        .or_else(|| lookup("ADDITIONAL_REDIRECT_URLS").filter(|value| !value.trim().is_empty()));
    raw.map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|pattern| !pattern.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

fn otp_length(raw: Option<&str>) -> usize {
    match raw.and_then(|value| value.parse::<usize>().ok()) {
        Some(length) if (6..=10).contains(&length) => length,
        _ => 6,
    }
}

fn env_bool(raw: Option<&str>, default: bool) -> bool {
    match raw.map(str::trim) {
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
        assert!(!state.manual_linking_enabled);
        assert!(!state.password_require_reauthentication);
        assert!(!state.password_require_current);
        assert_eq!(state.admin_group_name, "admin");
        assert_eq!(
            state.admin_roles,
            ["service_role", "supabase_admin"].map(str::to_string)
        );
        assert!(state.is_admin_role(Some("service_role")));
        assert!(!state.is_admin_role(Some("anon")));
        assert!(state.config.mailer_autoconfirm);
        assert!(state.config.email_enabled);
        assert_eq!(state.config.site_url, "http://localhost:3000");
        assert!(state.uri_allow_list.is_empty());
        assert_eq!(state.api_external_url, "http://localhost:8000/auth/v1");
        assert_eq!(state.mailer_confirmation_path, "/auth/v1/verify");
        assert_eq!(state.mailer_invite_path, "/auth/v1/verify");
        assert_eq!(state.mailer_recovery_path, "/auth/v1/verify");
        assert_eq!(state.mailer_email_change_path, "/auth/v1/verify");
        assert_eq!(state.otp_length, 6);
        assert!(state.config.secure_email_change);
        assert!(!state.cursor_pagination);
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
        let shared = AuthState::from_lookup(|key| match key {
            "SITE_URL" => Some("https://app.example".into()),
            "GOTRUE_MAILER_SECURE_EMAIL_CHANGE_ENABLED" => Some("f".into()),
            _ => None,
        });
        assert_eq!(shared.config.site_url, "https://app.example");
        assert!(!shared.config.secure_email_change);
    }

    #[test]
    fn security_flags_follow_go_bool_rules() {
        let state = AuthState::try_from_lookup(|key| match key {
            "GOTRUE_SECURITY_MANUAL_LINKING_ENABLED" => Some("t".into()),
            "GOTRUE_SECURITY_UPDATE_PASSWORD_REQUIRE_REAUTHENTICATION" => Some("T".into()),
            "GOTRUE_SECURITY_UPDATE_PASSWORD_REQUIRE_CURRENT_PASSWORD" => Some("F".into()),
            _ => None,
        })
        .expect("t/T/F");
        assert!(state.manual_linking_enabled);
        assert!(state.password_require_reauthentication);
        assert!(!state.password_require_current);

        let Err(error) = AuthState::try_from_lookup(|key| {
            (key == "GOTRUE_SECURITY_UPDATE_PASSWORD_REQUIRE_REAUTHENTICATION")
                .then(|| "yes".into())
        }) else {
            panic!("yes must be rejected");
        };
        assert_eq!(
            error.to_string(),
            "GOTRUE_SECURITY_UPDATE_PASSWORD_REQUIRE_REAUTHENTICATION is not a valid value: yes"
        );
    }

    #[test]
    fn uri_allow_list_prefers_gotrue_name() {
        let from_gotrue = AuthState::from_lookup(|key| match key {
            "GOTRUE_URI_ALLOW_LIST" => {
                Some(" https://app.example/** , http://localhost:3000 ".into())
            }
            "ADDITIONAL_REDIRECT_URLS" => Some("https://other.example/**".into()),
            _ => None,
        });
        assert_eq!(
            from_gotrue.uri_allow_list,
            ["https://app.example/**", "http://localhost:3000"]
        );
        let from_docker = AuthState::from_lookup(|key| match key {
            "ADDITIONAL_REDIRECT_URLS" => Some("https://preview.example/**".into()),
            _ => None,
        });
        assert_eq!(from_docker.uri_allow_list, ["https://preview.example/**"]);
    }
}
