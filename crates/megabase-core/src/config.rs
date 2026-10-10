use std::fmt;
use std::time::Duration;

use crate::ids::JwtSecret;
use crate::jwt::MIN_JWT_SECRET_BYTES;
use crate::Error;

/// Kong `functions-v1` `read_timeout` in
/// `vendor/supabase/docker/volumes/api/kong.yml` (milliseconds).
pub const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_millis(150_000);

/// `FILE_SIZE_LIMIT` in `vendor/supabase/docker/docker-compose.yml` (bytes).
pub const DEFAULT_REQUEST_BODY_LIMIT: usize = 52_428_800;

/// Runtime configuration, read from environment variables.
#[derive(Clone, PartialEq, Eq)]
pub struct Config {
    /// `MEGABASE_HOST`, default `0.0.0.0`.
    pub host: String,
    /// `MEGABASE_PORT`, default `8000` (the port the Supabase gateway uses).
    pub port: u16,
    /// `DATABASE_URL`. When set, Megabase installs implemented Auth SQL
    /// objects at startup. PostgreSQL stays external.
    pub database_url: Option<String>,
    /// `JWT_SECRET`. Raw HS256 secret; no default. Omitted so the process can
    /// start for health checks. A present value shorter than 32 bytes
    /// (including empty) is a configuration error. [`Self::jwt_hs256`] fails
    /// if it is missing.
    pub jwt_secret: Option<JwtSecret>,
    /// `MEGABASE_HTTP_TIMEOUT_MS`. Whole-request deadline. Default is Kong's
    /// documented functions `read_timeout` ([`DEFAULT_HTTP_TIMEOUT`]).
    pub http_timeout: Duration,
    /// `MEGABASE_REQUEST_BODY_LIMIT_BYTES`. Default is the self-hosted
    /// `FILE_SIZE_LIMIT` ([`DEFAULT_REQUEST_BODY_LIMIT`]).
    pub request_body_limit: usize,
    /// `PGRST_DB_AGGREGATES_ENABLED`, default `false` (PostgREST's default,
    /// and the pinned Supabase compose file does not set it).
    pub db_aggregates_enabled: bool,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("host", &self.host)
            .field("port", &self.port)
            .field(
                "database_url",
                &self.database_url.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "jwt_secret",
                &self.jwt_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("http_timeout", &self.http_timeout)
            .field("request_body_limit", &self.request_body_limit)
            .field("db_aggregates_enabled", &self.db_aggregates_enabled)
            .finish()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8000,
            database_url: None,
            jwt_secret: None,
            http_timeout: DEFAULT_HTTP_TIMEOUT,
            request_body_limit: DEFAULT_REQUEST_BODY_LIMIT,
            db_aggregates_enabled: false,
        }
    }
}

impl Config {
    pub fn from_env() -> crate::Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> crate::Result<Self> {
        let defaults = Self::default();
        let port = match lookup("MEGABASE_PORT") {
            Some(raw) => raw
                .parse()
                .map_err(|_| Error::Config(format!("MEGABASE_PORT is not a valid port: {raw}")))?,
            None => defaults.port,
        };
        let jwt_secret = match lookup("JWT_SECRET") {
            Some(secret) => {
                // `str::len` is the UTF-8 byte length, which is the HMAC key size.
                let len = secret.len();
                if len < MIN_JWT_SECRET_BYTES {
                    return Err(Error::Config(format!(
                        "JWT_SECRET is {len} bytes; HMAC-SHA-256 keys shorter than {MIN_JWT_SECRET_BYTES} bytes are disabled"
                    )));
                }
                Some(JwtSecret::new(secret))
            }
            None => None,
        };
        let http_timeout = match lookup("MEGABASE_HTTP_TIMEOUT_MS") {
            Some(raw) => parse_millis(&raw)?,
            None => defaults.http_timeout,
        };
        let request_body_limit = match lookup("MEGABASE_REQUEST_BODY_LIMIT_BYTES") {
            Some(raw) => raw.parse().map_err(|_| {
                Error::Config(format!(
                    "MEGABASE_REQUEST_BODY_LIMIT_BYTES is not a valid byte count: {raw}"
                ))
            })?,
            None => defaults.request_body_limit,
        };
        Ok(Self {
            host: lookup("MEGABASE_HOST").unwrap_or(defaults.host),
            port,
            database_url: lookup("DATABASE_URL"),
            jwt_secret,
            http_timeout,
            request_body_limit,
            db_aggregates_enabled: lookup("PGRST_DB_AGGREGATES_ENABLED")
                .is_some_and(|raw| raw.trim().eq_ignore_ascii_case("true")),
        })
    }

    pub fn bind_address(&self) -> String {
        if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// HS256 verifier from `JWT_SECRET`. Missing or empty is
    /// [`crate::jwt::JwtError::SecretMissing`]. A shorter non-empty secret is
    /// [`crate::jwt::JwtError::SecretTooShort`]. Megabase never invents a
    /// default secret. [`Self::from_env`] rejects a short secret before listen.
    pub fn jwt_hs256(&self) -> crate::Result<crate::jwt::Hs256> {
        match self.jwt_secret.as_ref() {
            Some(secret) if !secret.is_empty() => Ok(crate::jwt::Hs256::new(secret.as_bytes())?),
            _ => Err(crate::jwt::JwtError::SecretMissing.into()),
        }
    }
}

fn parse_millis(raw: &str) -> crate::Result<Duration> {
    let millis: u64 = raw.parse().map_err(|_| {
        Error::Config(format!(
            "MEGABASE_HTTP_TIMEOUT_MS is not a valid millisecond count: {raw}"
        ))
    })?;
    if millis == 0 {
        return Err(Error::Config(
            "MEGABASE_HTTP_TIMEOUT_MS must be greater than 0".into(),
        ));
    }
    Ok(Duration::from_millis(millis))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_without_env() {
        let config = Config::from_lookup(|_| None).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.bind_address(), "0.0.0.0:8000");
    }

    #[test]
    fn rejects_invalid_port() {
        let err = Config::from_lookup(|k| (k == "MEGABASE_PORT").then(|| "x".into()));
        assert!(err.is_err());
    }

    #[test]
    fn bind_address_brackets_ipv6() {
        let v6 = Config {
            host: "::1".into(),
            ..Config::default()
        };
        assert_eq!(v6.bind_address(), "[::1]:8000");
        let already = Config {
            host: "[::1]".into(),
            ..Config::default()
        };
        assert_eq!(already.bind_address(), "[::1]:8000");
    }

    #[test]
    fn jwt_secret_from_env() {
        let secret = "your-super-secret-jwt-token-with-at-least-32-characters-long";
        let config = Config::from_lookup(|k| (k == "JWT_SECRET").then(|| secret.into())).unwrap();
        assert_eq!(
            config.jwt_secret.as_ref().map(JwtSecret::as_str),
            Some(secret)
        );
        assert_eq!(config.http_timeout, DEFAULT_HTTP_TIMEOUT);
        assert_eq!(config.request_body_limit, DEFAULT_REQUEST_BODY_LIMIT);
        assert!(config.jwt_hs256().is_ok());
    }

    #[test]
    fn jwt_hs256_fails_without_secret() {
        let err = Config::default().jwt_hs256().unwrap_err();
        assert!(matches!(
            err,
            Error::Jwt(crate::jwt::JwtError::SecretMissing)
        ));
        let empty = Config {
            jwt_secret: Some(JwtSecret::new(String::new())),
            ..Config::default()
        };
        assert!(matches!(
            empty.jwt_hs256().unwrap_err(),
            Error::Jwt(crate::jwt::JwtError::SecretMissing)
        ));
    }

    #[test]
    fn jwt_secret_shorter_than_32_bytes_aborts_startup() {
        let empty = Config::from_lookup(|k| (k == "JWT_SECRET").then(String::new)).unwrap_err();
        assert!(
            empty.to_string().contains("JWT_SECRET is 0 bytes"),
            "{empty}"
        );
        assert!(
            empty.to_string().contains("shorter than 32 bytes"),
            "{empty}"
        );

        let short = "a".repeat(MIN_JWT_SECRET_BYTES - 1);
        let err = Config::from_lookup(|k| (k == "JWT_SECRET").then(|| short.clone())).unwrap_err();
        assert!(matches!(err, Error::Config(_)));
        assert!(
            err.to_string()
                .contains(&format!("JWT_SECRET is {} bytes", MIN_JWT_SECRET_BYTES - 1)),
            "{err}"
        );

        let ok = "b".repeat(MIN_JWT_SECRET_BYTES);
        let config = Config::from_lookup(|k| (k == "JWT_SECRET").then(|| ok.clone())).unwrap();
        assert!(config.jwt_hs256().is_ok());
    }

    #[test]
    fn hand_built_short_secret_cannot_verify() {
        let short = Config {
            jwt_secret: Some(JwtSecret::new("a".repeat(31))),
            ..Config::default()
        };
        assert!(matches!(
            short.jwt_hs256().unwrap_err(),
            Error::Jwt(crate::jwt::JwtError::SecretTooShort { got: 31 })
        ));
    }

    #[test]
    fn debug_redacts_secrets() {
        let config = Config {
            database_url: Some("postgres://user:password@localhost/db".into()),
            jwt_secret: Some(JwtSecret::new("b".repeat(MIN_JWT_SECRET_BYTES))),
            ..Config::default()
        };
        let rendered = format!("{config:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("password"));
        assert!(!rendered.contains(&"b".repeat(MIN_JWT_SECRET_BYTES)));
        let secret = format!("{:?}", config.jwt_secret.as_ref().unwrap());
        assert_eq!(secret, "JwtSecret(<redacted>)");
    }

    #[test]
    fn http_limits_parse_and_reject_bad_values() {
        let config = Config::from_lookup(|k| match k {
            "MEGABASE_HTTP_TIMEOUT_MS" => Some("60000".into()),
            "MEGABASE_REQUEST_BODY_LIMIT_BYTES" => Some("1024".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(config.http_timeout, Duration::from_millis(60_000));
        assert_eq!(config.request_body_limit, 1024);

        let timeout =
            Config::from_lookup(|k| (k == "MEGABASE_HTTP_TIMEOUT_MS").then(|| "0".into()))
                .unwrap_err();
        assert!(timeout.to_string().contains("greater than 0"), "{timeout}");
        let bad = Config::from_lookup(|k| (k == "MEGABASE_HTTP_TIMEOUT_MS").then(|| "nope".into()))
            .unwrap_err();
        assert!(bad.to_string().contains("millisecond"), "{bad}");
        let body = Config::from_lookup(|k| {
            (k == "MEGABASE_REQUEST_BODY_LIMIT_BYTES").then(|| "-1".into())
        })
        .unwrap_err();
        assert!(body.to_string().contains("byte count"), "{body}");
    }
}
