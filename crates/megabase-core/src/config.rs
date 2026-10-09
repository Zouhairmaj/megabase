use crate::Error;

/// Runtime configuration, read from environment variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// `MEGABASE_HOST`, default `0.0.0.0`.
    pub host: String,
    /// `MEGABASE_PORT`, default `8000` (the port the Supabase gateway uses).
    pub port: u16,
    /// `DATABASE_URL`. When set, Megabase installs implemented Auth SQL
    /// objects at startup. PostgreSQL stays external.
    pub database_url: Option<String>,
    /// `JWT_SECRET`. Raw HS256 secret; no default. Optional so the process can
    /// start for health checks; [`Self::jwt_hs256`] fails if it is missing.
    pub jwt_secret: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8000,
            database_url: None,
            jwt_secret: None,
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
        Ok(Self {
            host: lookup("MEGABASE_HOST").unwrap_or(defaults.host),
            port,
            database_url: lookup("DATABASE_URL"),
            jwt_secret: lookup("JWT_SECRET"),
        })
    }

    pub fn bind_address(&self) -> String {
        if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// HS256 verifier from `JWT_SECRET`. Missing or empty is an error; Megabase
    /// never invents a default secret.
    pub fn jwt_hs256(&self) -> crate::Result<crate::jwt::Hs256> {
        match self.jwt_secret.as_deref() {
            Some(secret) if !secret.is_empty() => Ok(crate::jwt::Hs256::new(secret.as_bytes())?),
            _ => Err(crate::jwt::JwtError::SecretMissing.into()),
        }
    }
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
        assert_eq!(config.jwt_secret.as_deref(), Some(secret));
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
            jwt_secret: Some(String::new()),
            ..Config::default()
        };
        assert!(matches!(
            empty.jwt_hs256().unwrap_err(),
            Error::Jwt(crate::jwt::JwtError::SecretMissing)
        ));
    }
}
