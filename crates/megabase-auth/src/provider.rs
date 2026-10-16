// Ported from supabase/auth internal/api/external.go (`API.Provider`),
// internal/conf/configuration.go (`ValidateOAuth`) and
// internal/api/provider/{provider,azure,bitbucket,discord,facebook,figma,fly,github,gitlab,
// kakao,keycloak,linkedin,notion,slack,slack_oidc,snapchat,spotify,twitch,workos,x,zoom}.go
// (MIT), pin v2.197.0. The authorize URL reproduces the output of
// golang.org/x/oauth2 v0.36.0 `Config.AuthCodeURL`, `GenerateVerifier` and
// `S256ChallengeOption` (the version in that pin's go.mod); no oauth2
// source is copied.

//! Built-in OAuth providers as far as the authorize redirect reads them.
//!
//! [`provider`] is `API.Provider`: it validates the client settings and,
//! for a provider whose constructor needs no network call, returns the
//! `oauth2.Config` the redirect builds its URL from. Apple, Google,
//! LinkedIn (OIDC) and Vercel Marketplace fetch an OIDC discovery document
//! in their constructor, Twitter (OAuth 1.0a) fetches a request token to
//! build its authorize URL, and the code exchange of every provider calls
//! the provider over HTTPS. None of these is ported.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::config::{AuthConfig, OAuthClient};
use crate::external::{encode, set, Values};

/// `oauth2.Config` fields `AuthCodeURL` reads.
#[derive(Debug)]
pub(crate) struct AuthCodeConfig {
    auth_url: String,
    client_id: String,
    redirect_uri: String,
    scopes: Vec<String>,
    /// `OAuthProvider.RequiresPKCE`: the redirect stores a code verifier in
    /// `auth.oauth_client_states` and sends its S256 challenge.
    pub(crate) requires_pkce: bool,
}

/// `oauth2.GenerateVerifier`: 32 random bytes, base64url without padding.
pub(crate) fn generate_verifier(random: &[u8; 32]) -> String {
    URL_SAFE_NO_PAD.encode(random)
}

/// `oauth2.S256ChallengeOption(verifier)`: `code_challenge_method=S256`
/// and the base64url (no padding) SHA-256 of the verifier.
pub(crate) fn s256_challenge(verifier: &str) -> [(&'static str, String); 2] {
    [
        ("code_challenge_method", "S256".to_string()),
        (
            "code_challenge",
            URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        ),
    ]
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
        Ok(Some(endpoint)) => Provider::Ready {
            config: endpoint,
            email_optional: client.email_optional,
        },
        Ok(None) => Provider::NotPorted,
        Err(reason) => Provider::Rejected(reason.into()),
    }
}

/// `OAuthProviderConfiguration.Enabled`, or `None` for an unknown name.
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
        "vercel_marketplace" => config.id_token.vercel_marketplace,
        "workos" => external.workos,
        "x" => config.x_enabled,
        "zoom" => external.zoom,
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

/// The ported `New<Provider>Provider` constructors after `ValidateOAuth`:
/// `Ok(None)` for a constructor that is not ported, `Err` for a
/// constructor's own error.
fn endpoint(
    name: &str,
    client: &OAuthClient,
    scopes: &str,
) -> Result<Option<AuthCodeConfig>, &'static str> {
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
        // megabase:unit auth:provider:kakao
        "kakao" => (
            choose_host(url, "kauth.kakao.com") + "/oauth/authorize",
            &["account_email", "profile_image", "profile_nickname"],
            true,
        ),
        // megabase:unit auth:provider:keycloak
        "keycloak" => {
            // No default host: an empty `URL` is the constructor's error,
            // checked after `ValidateOAuth`.
            if url.is_empty() {
                return Err("unable to find URL for the Keycloak provider");
            }
            (
                choose_host(url, "") + "/protocol/openid-connect/auth",
                &["profile", "email"],
                true,
            )
        }
        // megabase:unit auth:provider:linkedin
        "linkedin" => (
            choose_host(url, "api.linkedin.com") + "/oauth/v2/authorization",
            &["r_emailaddress", "r_liteprofile"],
            true,
        ),
        // megabase:unit auth:provider:notion
        "notion" => (
            choose_host(url, "api.notion.com") + "/v1/oauth/authorize",
            &[],
            false,
        ),
        // megabase:unit auth:provider:slack
        "slack" => (
            choose_host(url, "slack.com") + "/oauth/authorize",
            &["profile", "email", "openid"],
            true,
        ),
        // megabase:unit auth:provider:slack_oidc
        "slack_oidc" => (
            choose_host(url, "slack.com") + "/openid/connect/authorize",
            &["profile", "email", "openid"],
            true,
        ),
        // megabase:unit auth:provider:snapchat
        "snapchat" => (
            choose_host(url, "accounts.snapchat.com") + "/accounts/oauth2/auth",
            &[
                "https://auth.snapchat.com/oauth2/api/user.external_id",
                "https://auth.snapchat.com/oauth2/api/user.display_name",
                "https://auth.snapchat.com/oauth2/api/user.bitmoji.avatar",
            ],
            true,
        ),
        // megabase:unit auth:provider:spotify
        "spotify" => (
            choose_host(url, "accounts.spotify.com") + "/authorize",
            &["user-read-email"],
            true,
        ),
        // megabase:unit auth:provider:twitch
        "twitch" => (
            choose_host(url, "id.twitch.tv") + "/oauth2/authorize",
            &["user:read:email"],
            true,
        ),
        // megabase:unit auth:provider:workos
        "workos" => (
            choose_host(url, "api.workos.com") + "/sso/authorize",
            &[],
            false,
        ),
        // megabase:unit auth:provider:x
        "x" => (
            choose_host(url, "x.com") + "/i/oauth2/authorize",
            &["users.email", "tweet.read", "users.read", "offline.access"],
            true,
        ),
        // megabase:unit auth:provider:zoom
        "zoom" => (choose_host(url, "zoom.us") + "/oauth/authorize", &[], false),
        _ => return Ok(None),
    };
    let mut scope_list: Vec<String> = default_scopes.iter().map(|s| (*s).to_string()).collect();
    // `append(oauthScopes, strings.Split(scopes, ",")...)`: empty parts stay.
    if takes_scopes && !scopes.is_empty() {
        scope_list.extend(scopes.split(',').map(str::to_string));
    }
    // `ValidateOAuth` already rejected an empty client id list.
    let Some(client_id) = client.client_id.first() else {
        return Ok(None);
    };
    Ok(Some(AuthCodeConfig {
        auth_url,
        client_id: client_id.clone(),
        redirect_uri: client.redirect_uri.clone(),
        scopes: scope_list,
        requires_pkce: name == "x",
    }))
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

    #[test]
    fn second_batch_constructors_match_upstream() {
        let mut base = AuthConfig::reference_defaults();
        let external = &mut base.external;
        for flag in [
            &mut external.google,
            &mut external.kakao,
            &mut external.keycloak,
            &mut external.linkedin,
            &mut external.linkedin_oidc,
            &mut external.notion,
            &mut external.slack,
            &mut external.slack_oidc,
            &mut external.snapchat,
            &mut external.spotify,
        ] {
            *flag = true;
        }
        let client = |url: &str| OAuthClient {
            client_id: vec!["cid".into()],
            secret: "s".into(),
            redirect_uri: "http://localhost:8000/auth/v1/callback".into(),
            url: url.into(),
            email_optional: false,
        };
        let build = |name: &str, url: &str, scopes: &str| {
            let mut config = AuthConfig::reference_defaults();
            config.external = base.external.clone();
            config.oauth_clients.set(name, client(url));
            provider(&config, name, scopes)
        };
        let cases: [(&str, &str, &[&str]); 8] = [
            (
                "kakao",
                "https://kauth.kakao.com/oauth/authorize",
                &["account_email", "profile_image", "profile_nickname", "x"],
            ),
            (
                "linkedin",
                "https://api.linkedin.com/oauth/v2/authorization",
                &["r_emailaddress", "r_liteprofile", "x"],
            ),
            ("notion", "https://api.notion.com/v1/oauth/authorize", &[]),
            (
                "slack",
                "https://slack.com/oauth/authorize",
                &["profile", "email", "openid", "x"],
            ),
            (
                "slack_oidc",
                "https://slack.com/openid/connect/authorize",
                &["profile", "email", "openid", "x"],
            ),
            (
                "snapchat",
                "https://accounts.snapchat.com/accounts/oauth2/auth",
                &[
                    "https://auth.snapchat.com/oauth2/api/user.external_id",
                    "https://auth.snapchat.com/oauth2/api/user.display_name",
                    "https://auth.snapchat.com/oauth2/api/user.bitmoji.avatar",
                    "x",
                ],
            ),
            (
                "spotify",
                "https://accounts.spotify.com/authorize",
                &["user-read-email", "x"],
            ),
            (
                "keycloak",
                "https://kc.example.com/realms/r/protocol/openid-connect/auth",
                &["profile", "email", "x"],
            ),
        ];
        for (name, auth_url, scopes) in cases {
            let url = if name == "keycloak" {
                "https://kc.example.com/realms/r/"
            } else {
                ""
            };
            let config = ready(build(name, url, "x"));
            assert_eq!(config.auth_url, auth_url, "{name}");
            assert_eq!(config.scopes, scopes, "{name}");
        }
        // Notion sends no `scope` at all.
        let notion = ready(build("notion", "", "x"));
        assert!(!notion.auth_code_url("st", &Vec::new()).contains("scope="));
        // Keycloak has no default host.
        assert!(matches!(
            build("keycloak", "", ""),
            Provider::Rejected(reason) if reason == "unable to find URL for the Keycloak provider"
        ));
        // Google and LinkedIn (OIDC) fetch a discovery document.
        for name in ["google", "linkedin_oidc"] {
            assert!(matches!(build(name, "", ""), Provider::NotPorted), "{name}");
        }
    }

    #[test]
    fn third_batch_constructors_match_upstream() {
        let mut config = AuthConfig::reference_defaults();
        config.external.twitch = true;
        config.external.twitter = true;
        config.external.workos = true;
        config.external.zoom = true;
        config.x_enabled = true;
        config.id_token.vercel_marketplace = true;
        let build = |config: &AuthConfig, name: &str, url: &str| {
            let mut config = config.clone();
            config.oauth_clients.set(
                name,
                OAuthClient {
                    client_id: vec!["cid".into()],
                    secret: "s".into(),
                    redirect_uri: "http://localhost:8000/auth/v1/callback".into(),
                    url: url.into(),
                    email_optional: false,
                },
            );
            provider(&config, name, "a,b")
        };
        let cases: [(&str, &str, &[&str], bool); 4] = [
            (
                "twitch",
                "https://id.twitch.tv/oauth2/authorize",
                &["user:read:email", "a", "b"],
                false,
            ),
            ("workos", "https://api.workos.com/sso/authorize", &[], false),
            (
                "x",
                "https://x.com/i/oauth2/authorize",
                &[
                    "users.email",
                    "tweet.read",
                    "users.read",
                    "offline.access",
                    "a",
                    "b",
                ],
                true,
            ),
            ("zoom", "https://zoom.us/oauth/authorize", &[], false),
        ];
        for (name, auth_url, scopes, pkce) in cases {
            let ready = ready(build(&config, name, ""));
            assert_eq!(ready.auth_url, auth_url, "{name}");
            assert_eq!(ready.scopes, scopes, "{name}");
            assert_eq!(ready.requires_pkce, pkce, "{name}");
        }
        let zoom = ready(build(&config, "zoom", "https://zoom.example/"));
        assert_eq!(zoom.auth_url, "https://zoom.example/oauth/authorize");
        // Twitter fetches an OAuth 1.0a request token and Vercel Marketplace
        // an OIDC discovery document, both after `ValidateOAuth`.
        for name in ["twitter", "vercel_marketplace"] {
            assert!(
                matches!(build(&config, name, ""), Provider::NotPorted),
                "{name}"
            );
        }
        config.x_enabled = false;
        assert!(matches!(
            build(&config, "x", ""),
            Provider::Rejected(reason) if reason == "provider is not enabled"
        ));
    }

    #[test]
    fn s256_challenge_matches_rfc_7636() {
        // RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let [(method_key, method), (challenge_key, challenge)] = s256_challenge(verifier);
        assert_eq!(
            (method_key, method.as_str()),
            ("code_challenge_method", "S256")
        );
        assert_eq!(challenge_key, "code_challenge");
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(generate_verifier(&[0; 32]).len(), 43);
    }
}
