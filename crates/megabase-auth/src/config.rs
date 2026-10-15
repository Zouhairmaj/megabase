// Ported from supabase/auth internal/conf/configuration.go and
// internal/api/settings.go (MIT), pin v2.197.0.
//
// Defaults match the judge reference stack (vendor/supabase/docker
// `.env.example` plus `judge/compose.override.yml`), not GoTrue's zero
// values. The judge starts Megabase with `JWT_SECRET` and `DATABASE_URL`
// only, so unset `GOTRUE_*` must still agree with that stack.

//! Auth HTTP configuration.

use crate::error::ConfigError;
use crate::hooks::AccessTokenHook;

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
    /// `GOTRUE_EXTERNAL_<PROVIDER>_*` client settings `API.Provider` reads.
    pub oauth_clients: OAuthClients,
    pub sms_provider: String,
    /// `GOTRUE_SMS_OTP_EXP` in seconds. `0` is raised to 60, matching
    /// GoTrue's startup default.
    pub sms_otp_exp_seconds: u64,
    /// `GOTRUE_SMS_TEST_OTP` is set. Phone verify does not port the test
    /// OTP map, so it answers 501 instead.
    pub sms_test_otp_configured: bool,
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
    /// `GOTRUE_EXTERNAL_REDIRECT_URL`. When set, OAuth callbacks redirect
    /// here instead of the flow state referrer or the site URL.
    pub external_redirect_url: String,
    /// `GOTRUE_EXTERNAL_EMAIL_MAGIC_LINK_ENABLED`. GoTrue default is true.
    pub magic_link_enabled: bool,
    /// `GOTRUE_MAILER_OTP_EXP` in seconds. `0` is raised to one day, matching
    /// GoTrue's startup default.
    pub mailer_otp_exp_seconds: u64,
    /// `GOTRUE_MAILER_SECURE_EMAIL_CHANGE_ENABLED`.
    pub secure_email_change: bool,
    /// `GOTRUE_HOOK_CUSTOM_ACCESS_TOKEN_*`. `None` when the hook is disabled.
    pub(crate) custom_access_token_hook: Option<AccessTokenHook>,
    /// `GOTRUE_MFA_*` enroll and verify switches.
    pub mfa: MfaConfig,
    /// Provider settings only the `id_token` grant reads.
    pub id_token: IdTokenConfig,
    /// `GOTRUE_EXTERNAL_WEB3_SOLANA_ENABLED`. Off on the reference stack.
    pub web3_solana: bool,
    /// `GOTRUE_EXTERNAL_WEB3_ETHEREUM_ENABLED`. Off on the reference stack.
    pub web3_ethereum: bool,
}

/// `ExternalProviderConfiguration` fields the `id_token` grant reads and
/// `GET /settings` does not publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdTokenConfig {
    /// `GOTRUE_EXTERNAL_VERCEL_MARKETPLACE_ENABLED`.
    pub vercel_marketplace: bool,
    /// `GOTRUE_EXTERNAL_KEYCLOAK_URL`: the Keycloak issuer.
    pub keycloak_url: String,
    /// `GOTRUE_EXTERNAL_ALLOWED_ID_TOKEN_ISSUERS`, comma-separated. Empty
    /// is raised to the Apple and Google issuers, as `ApplyDefaults` does.
    pub allowed_issuers: Vec<String>,
}

impl Default for IdTokenConfig {
    fn default() -> Self {
        Self {
            vercel_marketplace: false,
            keycloak_url: String::new(),
            allowed_issuers: default_id_token_issuers(),
        }
    }
}

fn default_id_token_issuers() -> Vec<String> {
    vec![
        "https://appleid.apple.com".into(),
        "https://accounts.google.com".into(),
    ]
}

/// envconfig `[]string`: split on `,`; an empty list takes the default.
fn id_token_issuers(raw: Option<String>) -> Vec<String> {
    match raw {
        Some(raw) if !raw.is_empty() => raw.split(',').map(str::to_string).collect(),
        _ => default_id_token_issuers(),
    }
}

/// `MFAConfiguration` enroll and verify switches for each factor type.
///
/// GoTrue turns TOTP on by default and leaves phone, WebAuthn, and recovery
/// codes opt-in. The reference stack sets none of these variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MfaConfig {
    pub totp_enroll: bool,
    pub totp_verify: bool,
    pub phone_enroll: bool,
    pub phone_verify: bool,
    pub webauthn_enroll: bool,
    pub webauthn_verify: bool,
    pub recovery_codes_enroll: bool,
    pub recovery_codes_verify: bool,
}

impl Default for MfaConfig {
    fn default() -> Self {
        Self {
            totp_enroll: true,
            totp_verify: true,
            phone_enroll: false,
            phone_verify: false,
            webauthn_enroll: false,
            webauthn_verify: false,
            recovery_codes_enroll: false,
            recovery_codes_verify: false,
        }
    }
}

impl MfaConfig {
    fn from_lookup(lookup: &impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let defaults = Self::default();
        let read = |key: &str, default: bool| -> Result<bool, ConfigError> {
            Ok(optional_bool(lookup, key)?.unwrap_or(default))
        };
        Ok(Self {
            totp_enroll: read("GOTRUE_MFA_TOTP_ENROLL_ENABLED", defaults.totp_enroll)?,
            totp_verify: read("GOTRUE_MFA_TOTP_VERIFY_ENABLED", defaults.totp_verify)?,
            phone_enroll: read("GOTRUE_MFA_PHONE_ENROLL_ENABLED", defaults.phone_enroll)?,
            phone_verify: read("GOTRUE_MFA_PHONE_VERIFY_ENABLED", defaults.phone_verify)?,
            webauthn_enroll: read(
                "GOTRUE_MFA_WEB_AUTHN_ENROLL_ENABLED",
                defaults.webauthn_enroll,
            )?,
            webauthn_verify: read(
                "GOTRUE_MFA_WEB_AUTHN_VERIFY_ENABLED",
                defaults.webauthn_verify,
            )?,
            recovery_codes_enroll: read(
                "GOTRUE_MFA_RECOVERY_CODES_ENROLL_ENABLED",
                defaults.recovery_codes_enroll,
            )?,
            recovery_codes_verify: read(
                "GOTRUE_MFA_RECOVERY_CODES_VERIFY_ENABLED",
                defaults.recovery_codes_verify,
            )?,
        })
    }
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

/// `OAuthProviderConfiguration` fields other than `enabled`, for one
/// provider. `Debug` leaves the secret out.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct OAuthClient {
    /// `CLIENT_ID`, split on `,` like envconfig does for a `[]string`.
    pub client_id: Vec<String>,
    pub secret: String,
    pub redirect_uri: String,
    /// `URL`: the provider host override `chooseHost` reads.
    pub url: String,
    pub email_optional: bool,
}

impl std::fmt::Debug for OAuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthClient")
            .field("client_id", &self.client_id)
            .field("secret_set", &!self.secret.is_empty())
            .field("redirect_uri", &self.redirect_uri)
            .field("url", &self.url)
            .field("email_optional", &self.email_optional)
            .finish()
    }
}

/// Every built-in provider's [`OAuthClient`], keyed by provider name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OAuthClients(std::collections::BTreeMap<&'static str, OAuthClient>);

/// `ProviderConfiguration` OAuth fields. The env segment is the name in
/// upper case (`LINKEDIN_OIDC`, `SLACK_OIDC`, `VERCEL_MARKETPLACE` and `X`
/// come from envconfig tags or `split_words`).
const OAUTH_PROVIDER_NAMES: &[&str] = &[
    "apple",
    "azure",
    "bitbucket",
    "discord",
    "facebook",
    "snapchat",
    "figma",
    "fly",
    "github",
    "gitlab",
    "google",
    "kakao",
    "notion",
    "keycloak",
    "linkedin",
    "linkedin_oidc",
    "spotify",
    "slack",
    "slack_oidc",
    "twitter",
    "twitch",
    "vercel_marketplace",
    "workos",
    "x",
    "zoom",
];

impl OAuthClients {
    /// The settings of provider `name`, if it is a built-in provider.
    pub fn get(&self, name: &str) -> Option<&OAuthClient> {
        self.0.get(name)
    }

    /// Replace the settings of built-in provider `name`; other names are
    /// ignored.
    pub fn set(&mut self, name: &str, client: OAuthClient) {
        if let Some(known) = OAUTH_PROVIDER_NAMES.iter().find(|known| **known == name) {
            self.0.insert(known, client);
        }
    }

    fn from_lookup(lookup: &impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let mut clients = Self::default();
        for name in OAUTH_PROVIDER_NAMES {
            let prefix = format!("GOTRUE_EXTERNAL_{}", name.to_ascii_uppercase());
            let text = |field: &str| lookup(&format!("{prefix}_{field}")).unwrap_or_default();
            let client_id = text("CLIENT_ID");
            // envconfig leaves a `[]string` empty for a blank value and does
            // not trim the parts.
            let client_id = if client_id.trim().is_empty() {
                Vec::new()
            } else {
                client_id.split(',').map(str::to_string).collect()
            };
            let client = OAuthClient {
                client_id,
                secret: text("SECRET"),
                redirect_uri: text("REDIRECT_URI"),
                url: text("URL"),
                email_optional: flag(lookup, &format!("{prefix}_EMAIL_OPTIONAL"))?,
            };
            clients.0.insert(name, client);
        }
        Ok(clients)
    }
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
            oauth_clients: OAuthClients::default(),
            sms_provider: String::new(),
            sms_otp_exp_seconds: 60,
            sms_test_otp_configured: false,
            saml_enabled: false,
            // With SAML off, GoTrue reports `true`: see `from_lookup`.
            saml_private_key_next_configured: true,
            passkeys_enabled: false,
            jwt_aud: "authenticated".into(),
            jwt_default_group: "authenticated".into(),
            jwt_exp_seconds: 3600,
            jwt_issuer: "http://localhost:8000/auth/v1".into(),
            password_min_length: MIN_PASSWORD_LENGTH,
            // `vendor/supabase/docker/.env.example` `SITE_URL`.
            site_url: "http://localhost:3000".into(),
            external_redirect_url: String::new(),
            magic_link_enabled: true,
            mailer_otp_exp_seconds: 86_400,
            secure_email_change: true,
            custom_access_token_hook: None,
            mfa: MfaConfig::default(),
            id_token: IdTokenConfig::default(),
            web3_solana: false,
            web3_ethereum: false,
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
        let sms_otp_exp = match optional_u64(&lookup, "GOTRUE_SMS_OTP_EXP")? {
            Some(0) | None => defaults.sms_otp_exp_seconds,
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
        let saml_enabled =
            optional_bool(&lookup, "GOTRUE_SAML_ENABLED")?.unwrap_or(defaults.saml_enabled);
        // `settings.go` reports `SAML.CertificateNext != nil`. envconfig
        // allocates a zero-value certificate for that pointer, and only
        // `SAMLConfiguration.PopulateFields` resets it to nil when
        // `GOTRUE_SAML_PRIVATE_KEY_NEXT` is empty. `ApplyDefaults` calls it
        // only when SAML is enabled, so with SAML off the flag is `true`
        // (`internal/conf/configuration.go`, `internal/conf/saml.go`).
        let saml_private_key_next_configured = !saml_enabled
            || lookup("GOTRUE_SAML_PRIVATE_KEY_NEXT").is_some_and(|value| !value.is_empty());
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
            oauth_clients: OAuthClients::from_lookup(&lookup)?,
            sms_provider: lookup("GOTRUE_SMS_PROVIDER").unwrap_or(defaults.sms_provider),
            sms_otp_exp_seconds: sms_otp_exp,
            sms_test_otp_configured: lookup("GOTRUE_SMS_TEST_OTP")
                .is_some_and(|value| !value.is_empty()),
            saml_enabled,
            saml_private_key_next_configured,
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
            external_redirect_url: lookup("GOTRUE_EXTERNAL_REDIRECT_URL")
                .unwrap_or(defaults.external_redirect_url),
            magic_link_enabled: optional_bool(&lookup, "GOTRUE_EXTERNAL_EMAIL_MAGIC_LINK_ENABLED")?
                .unwrap_or(defaults.magic_link_enabled),
            mailer_otp_exp_seconds: otp_exp,
            secure_email_change: optional_bool(
                &lookup,
                "GOTRUE_MAILER_SECURE_EMAIL_CHANGE_ENABLED",
            )?
            .unwrap_or(defaults.secure_email_change),
            custom_access_token_hook: AccessTokenHook::from_lookup(&lookup)?,
            mfa: MfaConfig::from_lookup(&lookup)?,
            id_token: IdTokenConfig {
                vercel_marketplace: flag(&lookup, "GOTRUE_EXTERNAL_VERCEL_MARKETPLACE_ENABLED")?,
                keycloak_url: lookup("GOTRUE_EXTERNAL_KEYCLOAK_URL").unwrap_or_default(),
                allowed_issuers: id_token_issuers(lookup(
                    "GOTRUE_EXTERNAL_ALLOWED_ID_TOKEN_ISSUERS",
                )),
            },
            web3_solana: flag(&lookup, "GOTRUE_EXTERNAL_WEB3_SOLANA_ENABLED")?,
            web3_ethereum: flag(&lookup, "GOTRUE_EXTERNAL_WEB3_ETHEREUM_ENABLED")?,
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
        assert_eq!(config.sms_otp_exp_seconds, 60);
        assert!(!config.sms_test_otp_configured);
        assert!(!config.saml_enabled);
        assert!(config.saml_private_key_next_configured);
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
            "GOTRUE_SMS_OTP_EXP" => Some("300".into()),
            "GOTRUE_SMS_TEST_OTP" => Some("15550001111:123456".into()),
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
        assert_eq!(config.sms_otp_exp_seconds, 300);
        assert!(config.sms_test_otp_configured);
        assert!(config.saml_private_key_next_configured);
    }

    #[test]
    fn saml_next_key_flag_follows_gotrue_apply_defaults() {
        let enabled_without_next =
            AuthConfig::from_lookup(|key| (key == "GOTRUE_SAML_ENABLED").then(|| "true".into()))
                .unwrap();
        assert!(!enabled_without_next.saml_private_key_next_configured);
        let enabled_with_next = AuthConfig::from_lookup(|key| match key {
            "GOTRUE_SAML_ENABLED" => Some("true".into()),
            "GOTRUE_SAML_PRIVATE_KEY_NEXT" => Some("key".into()),
            _ => None,
        })
        .unwrap();
        assert!(enabled_with_next.saml_private_key_next_configured);
        let disabled =
            AuthConfig::from_lookup(|key| (key == "GOTRUE_SAML_ENABLED").then(|| "false".into()))
                .unwrap();
        assert!(disabled.saml_private_key_next_configured);
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
    fn id_token_and_web3_settings() {
        let defaults = AuthConfig::reference_defaults();
        assert!(!defaults.web3_solana && !defaults.web3_ethereum);
        assert_eq!(
            defaults.id_token.allowed_issuers,
            ["https://appleid.apple.com", "https://accounts.google.com"]
        );
        let config = AuthConfig::from_lookup(|key| match key {
            "GOTRUE_EXTERNAL_WEB3_SOLANA_ENABLED" => Some("true".into()),
            "GOTRUE_EXTERNAL_VERCEL_MARKETPLACE_ENABLED" => Some("1".into()),
            "GOTRUE_EXTERNAL_KEYCLOAK_URL" => Some("https://kc.example/realms/a".into()),
            "GOTRUE_EXTERNAL_ALLOWED_ID_TOKEN_ISSUERS" => Some("https://a,https://b".into()),
            _ => None,
        })
        .unwrap();
        assert!(config.web3_solana && !config.web3_ethereum);
        assert!(config.id_token.vercel_marketplace);
        assert_eq!(config.id_token.keycloak_url, "https://kc.example/realms/a");
        assert_eq!(config.id_token.allowed_issuers, ["https://a", "https://b"]);
    }

    #[test]
    fn invalid_bool_is_an_error() {
        let error =
            AuthConfig::from_lookup(|key| (key == "GOTRUE_DISABLE_SIGNUP").then(|| "maybe".into()))
                .unwrap_err();
        assert!(error.to_string().contains("GOTRUE_DISABLE_SIGNUP"));
    }
}
