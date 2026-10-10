// Ported from supabase/auth internal/conf/configuration.go and
// internal/api/settings.go (MIT), pin v2.197.0.
//
// Defaults match the judge reference stack (vendor/supabase/docker
// `.env.example` plus `judge/compose.override.yml`), not GoTrue's zero
// values. The judge starts Megabase with `JWT_SECRET` and `DATABASE_URL`
// only, so unset `GOTRUE_*` must still agree with that stack.

//! Auth HTTP configuration.

use crate::error::ConfigError;

/// Published GoTrue image version (`Makefile` `VERSION=v$(RELEASE_VERSION)`).
pub const AUTH_VERSION: &str = "v2.197.0";

/// GoTrue `defaultMinPasswordLength`. Values below this are raised to it.
pub const MIN_PASSWORD_LENGTH: usize = 6;

/// bcrypt cost. Go `bcrypt.DefaultCost` is 10; the Rust `bcrypt` crate
/// default is 12, so callers pass this explicitly.
pub const BCRYPT_COST: u32 = 10;

/// Flags and JWT claim defaults for `/auth/v1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthConfig {
    pub disable_signup: bool,
    pub mailer_autoconfirm: bool,
    pub phone_autoconfirm: bool,
    pub anonymous_users: bool,
    pub email_enabled: bool,
    pub phone_enabled: bool,
    pub external: ExternalProviders,
    pub sms_provider: String,
    pub saml_enabled: bool,
    pub saml_private_key_next_configured: bool,
    pub passkeys_enabled: bool,
    pub jwt_aud: String,
    pub jwt_default_group: String,
    pub jwt_exp_seconds: i64,
    pub jwt_issuer: String,
    pub password_min_length: usize,
    /// `GOTRUE_SITE_URL`, or `SITE_URL` when that is unset. Implicit verify
    /// redirects land here when `redirect_to` is missing or not allowed.
    pub site_url: String,
    /// `GOTRUE_MAILER_OTP_EXP` in seconds. `0` is raised to one day, matching
    /// GoTrue's startup default.
    pub mailer_otp_exp_seconds: u64,
    /// `GOTRUE_MAILER_SECURE_EMAIL_CHANGE_ENABLED`.
    pub secure_email_change: bool,
}

/// `external` object on `GET /settings`. Email and phone are separate fields
/// because signup branches on them; the rest are settings booleans only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalProviders {
    pub apple: bool,
    pub azure: bool,
    pub bitbucket: bool,
    pub discord: bool,
    pub facebook: bool,
    pub snapchat: bool,
    pub figma: bool,
    pub fly: bool,
    pub github: bool,
    pub gitlab: bool,
    pub google: bool,
    pub keycloak: bool,
    pub kakao: bool,
    pub linkedin: bool,
    pub linkedin_oidc: bool,
    pub notion: bool,
    pub spotify: bool,
    pub slack: bool,
    pub slack_oidc: bool,
    pub workos: bool,
    pub twitch: bool,
    pub twitter: bool,
    pub zoom: bool,
}

impl AuthConfig {
    /// Reference-stack defaults used when `GOTRUE_*` is unset.
    pub fn reference_defaults() -> Self {
        Self {
            disable_signup: false,
            // `judge/compose.override.yml` sets `GOTRUE_MAILER_AUTOCONFIRM=true`
            // because the reference stack has no mail server.
            mailer_autoconfirm: true,
            phone_autoconfirm: true,
            anonymous_users: false,
            email_enabled: true,
            phone_enabled: true,
            external: ExternalProviders::default(),
            sms_provider: String::new(),
            saml_enabled: false,
            saml_private_key_next_configured: false,
            passkeys_enabled: false,
            jwt_aud: "authenticated".into(),
            jwt_default_group: "authenticated".into(),
            jwt_exp_seconds: 3600,
            jwt_issuer: "http://localhost:8000/auth/v1".into(),
            password_min_length: MIN_PASSWORD_LENGTH,
            // `vendor/supabase/docker/.env.example` `SITE_URL`.
            site_url: "http://localhost:3000".into(),
            mailer_otp_exp_seconds: 86_400,
            secure_email_change: true,
        }
    }

    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let defaults = Self::reference_defaults();
        let password_min = match optional_u64(&lookup, "GOTRUE_PASSWORD_MIN_LENGTH")? {
            Some(value) => value.max(MIN_PASSWORD_LENGTH as u64) as usize,
            None => defaults.password_min_length,
        };
        let otp_exp = match optional_u64(&lookup, "GOTRUE_MAILER_OTP_EXP")? {
            Some(0) | None => defaults.mailer_otp_exp_seconds,
            Some(value) => value,
        };
        let jwt_exp = match optional_i64(&lookup, "GOTRUE_JWT_EXP")? {
            Some(value) if value > 0 => value,
            Some(value) => {
                return Err(ConfigError::Invalid {
                    key: "GOTRUE_JWT_EXP".into(),
                    value: value.to_string(),
                })
            }
            None => defaults.jwt_exp_seconds,
        };
        Ok(Self {
            disable_signup: optional_bool(&lookup, "GOTRUE_DISABLE_SIGNUP")?
                .unwrap_or(defaults.disable_signup),
            mailer_autoconfirm: optional_bool(&lookup, "GOTRUE_MAILER_AUTOCONFIRM")?
                .unwrap_or(defaults.mailer_autoconfirm),
            phone_autoconfirm: optional_bool(&lookup, "GOTRUE_SMS_AUTOCONFIRM")?
                .unwrap_or(defaults.phone_autoconfirm),
            anonymous_users: optional_bool(&lookup, "GOTRUE_EXTERNAL_ANONYMOUS_USERS_ENABLED")?
                .unwrap_or(defaults.anonymous_users),
            email_enabled: optional_bool(&lookup, "GOTRUE_EXTERNAL_EMAIL_ENABLED")?
                .unwrap_or(defaults.email_enabled),
            phone_enabled: optional_bool(&lookup, "GOTRUE_EXTERNAL_PHONE_ENABLED")?
                .unwrap_or(defaults.phone_enabled),
            external: ExternalProviders {
                apple: flag(&lookup, "GOTRUE_EXTERNAL_APPLE_ENABLED")?,
                azure: flag(&lookup, "GOTRUE_EXTERNAL_AZURE_ENABLED")?,
                bitbucket: flag(&lookup, "GOTRUE_EXTERNAL_BITBUCKET_ENABLED")?,
                discord: flag(&lookup, "GOTRUE_EXTERNAL_DISCORD_ENABLED")?,
                facebook: flag(&lookup, "GOTRUE_EXTERNAL_FACEBOOK_ENABLED")?,
                snapchat: flag(&lookup, "GOTRUE_EXTERNAL_SNAPCHAT_ENABLED")?,
                figma: flag(&lookup, "GOTRUE_EXTERNAL_FIGMA_ENABLED")?,
                fly: flag(&lookup, "GOTRUE_EXTERNAL_FLY_ENABLED")?,
                github: flag(&lookup, "GOTRUE_EXTERNAL_GITHUB_ENABLED")?,
                gitlab: flag(&lookup, "GOTRUE_EXTERNAL_GITLAB_ENABLED")?,
                google: flag(&lookup, "GOTRUE_EXTERNAL_GOOGLE_ENABLED")?,
                keycloak: flag(&lookup, "GOTRUE_EXTERNAL_KEYCLOAK_ENABLED")?,
                kakao: flag(&lookup, "GOTRUE_EXTERNAL_KAKAO_ENABLED")?,
                linkedin: flag(&lookup, "GOTRUE_EXTERNAL_LINKEDIN_ENABLED")?,
                linkedin_oidc: flag(&lookup, "GOTRUE_EXTERNAL_LINKEDIN_OIDC_ENABLED")?,
                notion: flag(&lookup, "GOTRUE_EXTERNAL_NOTION_ENABLED")?,
                spotify: flag(&lookup, "GOTRUE_EXTERNAL_SPOTIFY_ENABLED")?,
                slack: flag(&lookup, "GOTRUE_EXTERNAL_SLACK_ENABLED")?,
                slack_oidc: flag(&lookup, "GOTRUE_EXTERNAL_SLACK_OIDC_ENABLED")?,
                workos: flag(&lookup, "GOTRUE_EXTERNAL_WORKOS_ENABLED")?,
                twitch: flag(&lookup, "GOTRUE_EXTERNAL_TWITCH_ENABLED")?,
                twitter: flag(&lookup, "GOTRUE_EXTERNAL_TWITTER_ENABLED")?,
                zoom: flag(&lookup, "GOTRUE_EXTERNAL_ZOOM_ENABLED")?,
            },
            sms_provider: lookup("GOTRUE_SMS_PROVIDER").unwrap_or(defaults.sms_provider),
            saml_enabled: optional_bool(&lookup, "GOTRUE_SAML_ENABLED")?
                .unwrap_or(defaults.saml_enabled),
            saml_private_key_next_configured: lookup("GOTRUE_SAML_CERTIFICATE_NEXT")
                .is_some_and(|value| !value.is_empty()),
            passkeys_enabled: optional_bool(&lookup, "GOTRUE_PASSKEY_ENABLED")?
                .unwrap_or(defaults.passkeys_enabled),
            jwt_aud: lookup("GOTRUE_JWT_AUD").unwrap_or(defaults.jwt_aud),
            jwt_default_group: lookup("GOTRUE_JWT_DEFAULT_GROUP_NAME")
                .unwrap_or(defaults.jwt_default_group),
            jwt_exp_seconds: jwt_exp,
            jwt_issuer: lookup("GOTRUE_JWT_ISSUER").unwrap_or(defaults.jwt_issuer),
            password_min_length: password_min,
            site_url: lookup("GOTRUE_SITE_URL")
                .filter(|value| !value.is_empty())
                .or_else(|| lookup("SITE_URL").filter(|value| !value.is_empty()))
                .unwrap_or(defaults.site_url),
            mailer_otp_exp_seconds: otp_exp,
            secure_email_change: optional_bool(
                &lookup,
                "GOTRUE_MAILER_SECURE_EMAIL_CHANGE_ENABLED",
            )?
            .unwrap_or(defaults.secure_email_change),
        })
    }
}

fn flag(lookup: &impl Fn(&str) -> Option<String>, key: &str) -> Result<bool, ConfigError> {
    Ok(optional_bool(lookup, key)?.unwrap_or(false))
}

/// Go `strconv.ParseBool` for one env value.
///
/// Unset and empty are `None`. `1`, `t`, `T`, `true`, `0`, `f`, `F`, and
/// `false` parse in any case. Every other value is [`ConfigError::Invalid`].
pub(crate) fn optional_bool(
    lookup: &impl Fn(&str) -> Option<String>,
    key: &str,
) -> Result<Option<bool>, ConfigError> {
    match lookup(key) {
        None => Ok(None),
        Some(raw) if raw.trim().is_empty() => Ok(None),
        Some(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "t" | "true" => Ok(Some(true)),
            "0" | "f" | "false" => Ok(Some(false)),
            _ => Err(ConfigError::Invalid {
                key: key.into(),
                value: raw,
            }),
        },
    }
}

fn optional_i64(
    lookup: &impl Fn(&str) -> Option<String>,
    key: &str,
) -> Result<Option<i64>, ConfigError> {
    match lookup(key) {
        None => Ok(None),
        Some(raw) if raw.trim().is_empty() => Ok(None),
        Some(raw) => raw
            .trim()
            .parse()
            .map(Some)
            .map_err(|_| ConfigError::Invalid {
                key: key.into(),
                value: raw,
            }),
    }
}

fn optional_u64(
    lookup: &impl Fn(&str) -> Option<String>,
    key: &str,
) -> Result<Option<u64>, ConfigError> {
    match lookup(key) {
        None => Ok(None),
        Some(raw) if raw.trim().is_empty() => Ok(None),
        Some(raw) => raw
            .trim()
            .parse()
            .map(Some)
            .map_err(|_| ConfigError::Invalid {
                key: key.into(),
                value: raw,
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_defaults_match_the_judge_stack() {
        let config = AuthConfig::reference_defaults();
        assert!(!config.disable_signup);
        assert!(config.mailer_autoconfirm);
        assert!(config.phone_autoconfirm);
        assert!(!config.anonymous_users);
        assert!(config.email_enabled);
        assert!(config.phone_enabled);
        assert!(!config.external.github);
        assert_eq!(config.sms_provider, "");
        assert!(!config.saml_enabled);
        assert!(!config.passkeys_enabled);
        assert_eq!(config.jwt_aud, "authenticated");
        assert_eq!(config.jwt_default_group, "authenticated");
        assert_eq!(config.jwt_exp_seconds, 3600);
        assert_eq!(config.jwt_issuer, "http://localhost:8000/auth/v1");
        assert_eq!(config.password_min_length, 6);
        assert_eq!(config.site_url, "http://localhost:3000");
        assert_eq!(config.mailer_otp_exp_seconds, 86_400);
        assert!(config.secure_email_change);
    }

    #[test]
    fn env_overrides_and_password_floor() {
        let config = AuthConfig::from_lookup(|key| match key {
            "GOTRUE_DISABLE_SIGNUP" => Some("true".into()),
            "GOTRUE_MAILER_AUTOCONFIRM" => Some("0".into()),
            "GOTRUE_EXTERNAL_EMAIL_ENABLED" => Some("false".into()),
            "GOTRUE_EXTERNAL_GITHUB_ENABLED" => Some("T".into()),
            "GOTRUE_PASSWORD_MIN_LENGTH" => Some("4".into()),
            "GOTRUE_JWT_EXP" => Some("120".into()),
            "GOTRUE_SMS_PROVIDER" => Some("twilio".into()),
            "GOTRUE_SAML_CERTIFICATE_NEXT" => Some("cert".into()),
            _ => None,
        })
        .unwrap();
        assert!(config.disable_signup);
        assert!(!config.mailer_autoconfirm);
        assert!(!config.email_enabled);
        assert!(config.external.github);
        assert_eq!(config.password_min_length, 6);
        assert_eq!(config.jwt_exp_seconds, 120);
        assert_eq!(config.sms_provider, "twilio");
        assert!(config.saml_private_key_next_configured);
    }

    #[test]
    fn site_url_prefers_gotrue_name_and_secure_change_accepts_f() {
        let from_site = AuthConfig::from_lookup(|key| {
            (key == "SITE_URL").then(|| "https://app.example".into())
        })
        .unwrap();
        assert_eq!(from_site.site_url, "https://app.example");
        let both = AuthConfig::from_lookup(|key| match key {
            "GOTRUE_SITE_URL" => Some("https://gotrue.example".into()),
            "SITE_URL" => Some("https://site.example".into()),
            "GOTRUE_MAILER_SECURE_EMAIL_CHANGE_ENABLED" => Some("f".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(both.site_url, "https://gotrue.example");
        assert!(!both.secure_email_change);
    }

    #[test]
    fn invalid_bool_is_an_error() {
        let error =
            AuthConfig::from_lookup(|key| (key == "GOTRUE_DISABLE_SIGNUP").then(|| "maybe".into()))
                .unwrap_err();
        assert!(error.to_string().contains("GOTRUE_DISABLE_SIGNUP"));
    }
}
