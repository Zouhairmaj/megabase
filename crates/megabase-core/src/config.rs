use crate::Error;

/// Runtime configuration, read from environment variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// `MEGABASE_HOST`, default `0.0.0.0`.
    pub host: String,
    /// `MEGABASE_PORT`, default `8000` (the port the Supabase gateway uses).
    pub port: u16,
    /// `DATABASE_URL`. Not used yet; read so deployments can set it today.
    pub database_url: Option<String>,
    /// `JWT_SECRET`. Not used yet.
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
        format!("{}:{}", self.host, self.port)
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
}
