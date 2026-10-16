// Ported from supabase/auth internal/api/external.go (`API.Provider`),
// internal/conf/configuration.go (`ValidateOAuth`) and
// internal/api/provider/{provider,azure,bitbucket,discord,facebook,figma,fly,github,gitlab}.go
// (MIT), pin v2.197.0. The authorize URL reproduces the output of
// golang.org/x/oauth2 v0.36.0 `Config.AuthCodeURL` (the version in that
// pin's go.mod); no oauth2 source is copied.

//! Built-in OAuth providers as far as the authorize redirect reads them.
//!
//! [`provider`] is `API.Provider`: it validates the client settings and,
//! for a provider whose constructor needs no network call, returns the
//! `oauth2.Config` the redirect builds its URL from. Apple, Google,
//! LinkedIn (OIDC) and Vercel Marketplace fetch an OIDC discovery document
//! in their constructor, and the code exchange of every provider calls the
//! provider over HTTPS. Neither is ported.

use crate::config::{AuthConfig, OAuthClient};
use crate::external::{encode, set, Values};

/// `oauth2.Config` fields `AuthCodeURL` reads.
#[derive(Debug)]
pub(crate) struct AuthCodeConfig {
    auth_url: String,
    client_id: String,
    redirect_uri: String,
    scopes: Vec<String>,
}

impl AuthCodeConfig {
    /// `oauth2.Config.AuthCodeURL(state, opts...)`: `response_type`,
    /// `client_id`, `redirect_uri` and `scope` (when set) and `state`, then
    /// every `SetAuthURLParam` in `params`, `url.Values.Encode`d.
    pub(crate) fn auth_code_url(&self, state: &str, params: &Values) -> String {
        let mut values: Values = vec![
            ("response_type".into(), "code".into()),
            ("client_id".into(), self.client_id.clone()),
        ];
        if !self.redirect_uri.is_empty() {
            set(&mut values, "redirect_uri", &self.redirect_uri);
        }
        if !self.scopes.is_empty() {
            set(&mut values, "scope", &self.scopes.join(" "));
        }
        if !state.is_empty() {
            set(&mut values, "state", state);
        }
        for (key, value) in params {
            set(&mut values, key, value);
        }
        let separator = if self.auth_url.contains('?') {
            '&'
        } else {
            '?'
        };
        format!("{}{separator}{}", self.auth_url, encode(&values))
    }
}

/// What `API.Provider` returns for a built-in provider name.
pub(crate) enum Provider {
    /// The constructor's error: `ValidateOAuth`, or an unknown name.
    Rejected(String),
    /// Valid settings and a ported constructor.
    Ready {
        config: AuthCodeConfig,
        email_optional: bool,
    },
    /// Valid settings; the constructor or the provider type is not ported.
    NotPorted,
}

/// `API.Provider` for a built-in (not `custom:`) provider. `name` is
/// already lower case.
pub(crate) fn provider(config: &AuthConfig, name: &str, scopes: &str) -> Provider {
    let Some(enabled) = enabled(config, name) else {
        return Provider::Rejected(format!("Provider {name} could not be found"));
    };
    let default = OAuthClient::default();
    let client = config.oauth_clients.get(name).unwrap_or(&default);
    if let Err(reason) = validate_oauth(enabled, client) {
        return Provider::Rejected(reason.into());
    }
    match endpoint(name, client, scopes) {
        Some(endpoint) => Provider::Ready {
            config: endpoint,
            email_optional: client.email_optional,
        },
        None => Provider::NotPorted,
    }
}

/// `OAuthProviderConfiguration.Enabled`, or `None` for an unknown name.
/// X and Vercel Marketplace are not read from the environment yet, so they
/// count as disabled.
fn enabled(config: &AuthConfig, name: &str) -> Option<bool> {
    let external = &config.external;
    Some(match name {
        "apple" => external.apple,
        "azure" => external.azure,
        "bitbucket" => external.bitbucket,
        "discord" => external.discord,
        "facebook" => external.facebook,
        "figma" => external.figma,
        "fly" => external.fly,
        "github" => external.github,
        "gitlab" => external.gitlab,
        "google" => external.google,
        "kakao" => external.kakao,
        "keycloak" => external.keycloak,
        "linkedin" => external.linkedin,
        "linkedin_oidc" => external.linkedin_oidc,
        "notion" => external.notion,
        "snapchat" => external.snapchat,
        "spotify" => external.spotify,
        "slack" => external.slack,
        "slack_oidc" => external.slack_oidc,
        "twitch" => external.twitch,
        "twitter" => external.twitter,
        "workos" => external.workos,
        "zoom" => external.zoom,
        "x" | "vercel_marketplace" => false,
        _ => return None,
    })
}

/// `OAuthProviderConfiguration.ValidateOAuth`.
fn validate_oauth(enabled: bool, client: &OAuthClient) -> Result<(), &'static str> {
    if !enabled {
        return Err("provider is not enabled");
    }
    if client.client_id.is_empty() {
        return Err("missing OAuth client ID");
    }
    if client.secret.is_empty() {
        return Err("missing OAuth secret");
    }
    if client.redirect_uri.is_empty() {
        return Err("missing redirect URI");
    }
    Ok(())
}

/// The ported `New<Provider>Provider` constructors after `ValidateOAuth`.
fn endpoint(name: &str, client: &OAuthClient, scopes: &str) -> Option<AuthCodeConfig> {
    let url = client.url.as_str();
    let (auth_url, default_scopes, takes_scopes): (String, &[&str], bool) = match name {
        // megabase:unit auth:provider:azure
        "azure" => (
            choose_host(url, "login.microsoftonline.com/common") + "/oauth2/v2.0/authorize",
            &["openid"],
            true,
        ),
        // megabase:unit auth:provider:bitbucket
        "bitbucket" => (
            choose_host(url, "bitbucket.org") + "/site/oauth2/authorize",
            &["account", "email"],
            false,
        ),
        // megabase:unit auth:provider:discord
        "discord" => (
            choose_host(url, "discord.com") + "/api/oauth2/authorize",
            &["email", "identify"],
            true,
        ),
        // megabase:unit auth:provider:facebook
        "facebook" => (
            choose_host(url, "www.facebook.com") + "/dialog/oauth",
            &["email"],
            true,
        ),
        // megabase:unit auth:provider:figma
        "figma" => (
            choose_host(url, "www.figma.com") + "/oauth",
            &["current_user:read"],
            true,
        ),
        // megabase:unit auth:provider:fly
        "fly" => (
            choose_host(url, "api.fly.io") + "/oauth/authorize",
            &["read"],
            true,
        ),
        // megabase:unit auth:provider:github
        "github" => (
            choose_host(url, "github.com") + "/login/oauth/authorize",
            &["user:email"],
            true,
        ),
        // megabase:unit auth:provider:gitlab
        "gitlab" => (
            choose_host(url, "gitlab.com") + "/oauth/authorize",
            &["read_user"],
            true,
        ),
        _ => return None,
    };
    let mut scope_list: Vec<String> = default_scopes.iter().map(|s| (*s).to_string()).collect();
    // `append(oauthScopes, strings.Split(scopes, ",")...)`: empty parts stay.
    if takes_scopes && !scopes.is_empty() {
        scope_list.extend(scopes.split(',').map(str::to_string));
    }
    Some(AuthCodeConfig {
        auth_url,
        client_id: client.client_id.first()?.clone(),
        redirect_uri: client.redirect_uri.clone(),
        scopes: scope_list,
    })
}

/// `chooseHost`: `https://` plus the default host, or `base` without one
/// trailing slash.
fn choose_host(base: &str, default_host: &str) -> String {
    if base.is_empty() {
        return format!("https://{default_host}");
    }
    base.strip_suffix('/').unwrap_or(base).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(name: &str, url: &str) -> AuthConfig {
        let mut config = AuthConfig::reference_defaults();
        config.external.github = true;
        config.external.gitlab = true;
        config.external.apple = true;
        config.external.bitbucket = true;
        config.oauth_clients.set(
            name,
            OAuthClient {
                client_id: vec!["cid".into(), "other".into()],
                secret: "s".into(),
                redirect_uri: "http://localhost:8000/auth/v1/callback".into(),
                url: url.into(),
                email_optional: false,
            },
        );
        config
    }

    fn ready(provider: Provider) -> AuthCodeConfig {
        match provider {
            Provider::Ready { config, .. } => config,
            Provider::Rejected(reason) => panic!("rejected: {reason}"),
            Provider::NotPorted => panic!("not ported"),
        }
    }

    #[test]
    fn validate_oauth_reports_the_first_missing_setting() {
        let mut config = AuthConfig::reference_defaults();
        let reject = |config: &AuthConfig| match provider(config, "github", "") {
            Provider::Rejected(reason) => reason,
            _ => panic!("expected a rejection"),
        };
        assert_eq!(reject(&config), "provider is not enabled");
        config.external.github = true;
        assert_eq!(reject(&config), "missing OAuth client ID");
        let mut client = OAuthClient {
            client_id: vec!["cid".into()],
            ..OAuthClient::default()
        };
        config.oauth_clients.set("github", client.clone());
        assert_eq!(reject(&config), "missing OAuth secret");
        client.secret = "s".into();
        config.oauth_clients.set("github", client);
        assert_eq!(reject(&config), "missing redirect URI");
        assert!(matches!(
            provider(&config, "nope", ""),
            Provider::Rejected(reason) if reason == "Provider nope could not be found"
        ));
    }

    #[test]
    fn auth_code_url_matches_oauth2() {
        let config = ready(provider(
            &configured("github", ""),
            "github",
            "repo,,read:org",
        ));
        let params = vec![(
            "redirect_to".to_string(),
            "http://localhost:3000".to_string(),
        )];
        assert_eq!(
            config.auth_code_url("st", &params),
            "https://github.com/login/oauth/authorize?client_id=cid&redirect_to=http%3A%2F%2Flocalhost%3A3000&redirect_uri=http%3A%2F%2Flocalhost%3A8000%2Fauth%2Fv1%2Fcallback&response_type=code&scope=user%3Aemail+repo++read%3Aorg&state=st"
        );
        // A caller parameter overrides the defaults, as `SetAuthURLParam` does.
        let params = vec![("scope".to_string(), "x".to_string())];
        assert!(config.auth_code_url("st", &params).contains("&scope=x&"));
    }

    #[test]
    fn hosts_follow_choose_host_and_bitbucket_ignores_scopes() {
        let config = ready(provider(
            &configured("gitlab", "https://git.example.com/"),
            "gitlab",
            "",
        ));
        assert!(config
            .auth_code_url("st", &Vec::new())
            .starts_with("https://git.example.com/oauth/authorize?"));
        let config = ready(provider(&configured("bitbucket", ""), "bitbucket", "extra"));
        assert_eq!(config.scopes, ["account", "email"]);
        assert!(matches!(
            provider(&configured("apple", ""), "apple", ""),
            Provider::NotPorted
        ));
    }
}
