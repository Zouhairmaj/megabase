//! PostgreSQL pool for REST reads.
//!
//! Connects in the clear, same rule as Auth schema install: `sslmode=require`
//! is rejected before a socket opens so the password is not sent unprotected.

use std::str::FromStr;
use std::time::Duration;

use tokio_postgres::config::SslMode;
use tokio_postgres::Config;

const CONNECT_DEADLINE: Duration = Duration::from_secs(30);

/// Failure opening the REST pool. Display text does not include the URL.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// `sslmode=require` (or a mode this client cannot negotiate).
    #[error("sslmode=require is disabled; refusing to send the database password in the clear")]
    TlsRequired,
    /// The URL could not be parsed. The raw string is omitted on purpose.
    #[error("DATABASE_URL could not be parsed")]
    Url,
    /// The server did not accept a connection before [`CONNECT_DEADLINE`].
    #[error("connecting to postgres timed out")]
    TimedOut,
    /// The server rejected the connection. Details stay in the process log.
    #[error("connecting to postgres failed")]
    Connect,
}

/// Open a pool of 10 connections.
///
/// # Errors
///
/// Rejects `sslmode=require`, an unparseable URL, a timeout, or a refused
/// connection. The error text does not contain the URL.
pub async fn connect(database_url: &str) -> Result<sqlx::PgPool, DbError> {
    reject_tls(database_url)?;
    let connect = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(CONNECT_DEADLINE)
        .connect(database_url);
    match tokio::time::timeout(CONNECT_DEADLINE, connect).await {
        Ok(Ok(pool)) => Ok(pool),
        Ok(Err(error)) => {
            tracing::error!(error = %sanitize_connect_error(&error), "rest pool connect failed");
            Err(DbError::Connect)
        }
        Err(_) => Err(DbError::TimedOut),
    }
}

fn reject_tls(database_url: &str) -> Result<(), DbError> {
    let config = Config::from_str(database_url).map_err(|_| DbError::Url)?;
    if config.get_ssl_mode() == SslMode::Require {
        return Err(DbError::TlsRequired);
    }
    Ok(())
}

/// Drop anything that looks like a URL before the error reaches the log.
fn sanitize_connect_error(error: &sqlx::Error) -> String {
    let text = error.to_string();
    if text.contains("://") || text.to_ascii_lowercase().contains("password") {
        "connection failed".to_string()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_tls_is_rejected() {
        let err = reject_tls("postgres://user:secret@localhost/db?sslmode=require").unwrap_err();
        assert!(matches!(err, DbError::TlsRequired));
        assert!(!err.to_string().contains("secret"));
    }

    #[test]
    fn disable_tls_is_allowed() {
        assert!(reject_tls("postgres://user:secret@localhost/db?sslmode=disable").is_ok());
    }
}
