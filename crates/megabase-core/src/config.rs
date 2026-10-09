// Megabase Core - Configuration
// Ported from Supabase components (Apache-2.0, MIT licenses - see NOTICE)

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub jwt_secret: String,
    pub anon_key: Option<String>,
    pub service_role_key: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8000,
            database_url: "postgres://postgres:postgres@localhost:5432/postgres".to_string(),
            jwt_secret: "your-super-secret-jwt-token-with-at-least-32-characters".to_string(),
            anon_key: None,
            service_role_key: None,
        }
    }
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            host: std::env::var("MEGABASE_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: std::env::var("MEGABASE_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8000),
            database_url: std::env::var("DATABASE_URL").unwrap_or_else(|_| {
                "postgres://postgres:postgres@localhost:5432/postgres".to_string()
            }),
            jwt_secret: std::env::var("JWT_SECRET").unwrap_or_else(|_| {
                "your-super-secret-jwt-token-with-at-least-32-characters".to_string()
            }),
            anon_key: std::env::var("ANON_KEY").ok(),
            service_role_key: std::env::var("SERVICE_ROLE_KEY").ok(),
        }
    }

    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}
