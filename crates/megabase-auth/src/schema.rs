//! Installs Auth SQL objects applications and RLS policies call.
//!
//! Ported from supabase/auth migrations (MIT), pin v2.197.0. HTTP `/auth/v1`
//! is unchanged; this module is the database compatibility surface.

use std::str::FromStr;
use std::time::Duration;
use tokio_postgres::config::SslMode;
use tokio_postgres::{Config, NoTls};

/// Whole-install bound (connect + SQL). Startup fails instead of hanging.
const INSTALL_DEADLINE: Duration = Duration::from_secs(30);

const SCHEMA: &str = include_str!("../sql/00_schema.sql");

// megabase:unit auth:sql-table:auth.users
const USERS: &str = include_str!("../sql/users.sql");

// megabase:unit auth:sql-table:auth.sessions
const SESSIONS: &str = include_str!("../sql/sessions.sql");

// megabase:unit auth:sql-table:auth.schema_migrations
const SCHEMA_MIGRATIONS: &str = include_str!("../sql/schema_migrations.sql");

// megabase:unit auth:sql-table:auth.sso_providers
const SSO_PROVIDERS: &str = include_str!("../sql/sso_providers.sql");

// megabase:unit auth:sql-table:auth.sso_domains
const SSO_DOMAINS: &str = include_str!("../sql/sso_domains.sql");

// megabase:unit auth:sql-table:auth.saml_providers
const SAML_PROVIDERS: &str = include_str!("../sql/saml_providers.sql");

// megabase:unit auth:sql-table:auth.saml_relay_states
const SAML_RELAY_STATES: &str = include_str!("../sql/saml_relay_states.sql");

// megabase:unit auth:sql-table:auth.sso_sessions
const SSO_SESSIONS: &str = include_str!("../sql/sso_sessions.sql");

// megabase:unit auth:sql-table:auth.scim_users
const SCIM_USERS: &str = include_str!("../sql/scim_users.sql");

// megabase:unit auth:sql-table:auth.scim_tokens
const SCIM_TOKENS: &str = include_str!("../sql/scim_tokens.sql");

const OBJECTS: &[&str] = &[
    SCHEMA,
    USERS,
    SESSIONS,
    SCHEMA_MIGRATIONS,
    SSO_PROVIDERS,
    SSO_DOMAINS,
    SAML_PROVIDERS,
    SAML_RELAY_STATES,
    SSO_SESSIONS,
    SCIM_USERS,
    SCIM_TOKENS,
];

/// SQL applied in one transaction when `DATABASE_URL` is set.
fn install_sql() -> String {
    let mut sql = String::from("BEGIN;\n");
    for part in OBJECTS {
        sql.push_str(part);
        if !part.ends_with('\n') {
            sql.push('\n');
        }
    }
    sql.push_str("COMMIT;\n");
    sql
}

/// Failure applying the Auth SQL objects to PostgreSQL.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error(
        "DATABASE_URL requests TLS (sslmode={mode}); Megabase does not use TLS for PostgreSQL yet"
    )]
    TlsRequired { mode: String },
    #[error("installing auth schema timed out after {timeout:?}")]
    TimedOut { timeout: Duration },
    #[error("installing auth schema: {0}")]
    Postgres(#[from] tokio_postgres::Error),
}

fn require_cleartext_postgres(database_url: &str) -> Result<(), SchemaError> {
    let config = Config::from_str(database_url)?;
    if config.get_ssl_mode() == SslMode::Require {
        return Err(SchemaError::TlsRequired {
            mode: "require".into(),
        });
    }
    Ok(())
}

/// Create or replace the Auth SQL objects this crate implements.
///
/// Idempotent. Safe to run against a database the official Auth migrations
/// already applied. Aborts startup on error so a half-installed schema is
/// not served as if it were complete. Connects without TLS; `sslmode=require`
/// fails instead of sending credentials in the clear. `verify-*` is not a
/// valid `tokio-postgres` sslmode and fails at parse. The connect and SQL
/// run under a 30s deadline so a stalled PostgreSQL cannot hang startup.
pub async fn install_schema(database_url: &str) -> Result<(), SchemaError> {
    install_schema_within(database_url, INSTALL_DEADLINE).await
}

async fn install_schema_within(database_url: &str, deadline: Duration) -> Result<(), SchemaError> {
    require_cleartext_postgres(database_url)?;
    match tokio::time::timeout(deadline, install_schema_inner(database_url)).await {
        Ok(result) => result,
        Err(_) => Err(SchemaError::TimedOut { timeout: deadline }),
    }
}

async fn install_schema_inner(database_url: &str) -> Result<(), SchemaError> {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls).await?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            tracing::error!(%error, "postgres connection closed during auth schema install");
        }
    });
    client.batch_execute(&install_sql()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_sql_is_one_transaction() {
        let sql = install_sql();
        assert!(sql.starts_with("BEGIN;"), "{sql}");
        assert!(sql.trim_end().ends_with("COMMIT;"), "{sql}");
    }

    #[test]
    fn install_sql_has_no_go_templates() {
        assert!(
            !install_sql().contains("{{"),
            "leftover Go template in install SQL"
        );
    }

    #[test]
    fn issue_12_tables_are_installed() {
        let sql = install_sql();
        for name in [
            "auth.users",
            "auth.sessions",
            "auth.schema_migrations",
            "auth.sso_providers",
            "auth.sso_domains",
            "auth.saml_providers",
            "auth.saml_relay_states",
            "auth.sso_sessions",
            "auth.scim_users",
            "auth.scim_tokens",
        ] {
            assert!(
                sql.contains(&format!("CREATE TABLE IF NOT EXISTS {name}")),
                "missing {name}"
            );
        }
    }

    #[test]
    fn tables_declare_final_columns() {
        assert!(USERS.contains("encrypted_password"));
        assert!(USERS.contains(
            "GENERATED ALWAYS AS (LEAST (users.email_confirmed_at, users.phone_confirmed_at)) STORED"
        ));
        assert!(USERS.contains("users_email_partial_key"));
        assert!(USERS.contains("DROP CONSTRAINT IF EXISTS users_email_key"));
        assert!(USERS.contains("is_anonymous"));
        assert!(USERS.contains("is_sso_user"));
        assert!(USERS.contains("ALTER COLUMN phone TYPE text"));
        assert!(SESSIONS.contains("auth.aal_level"));
        assert!(SESSIONS.contains("timestamp WITHOUT TIME ZONE"));
        assert!(SESSIONS.contains("refresh_token_hmac_key"));
        assert!(SESSIONS.contains("sessions_scopes_length"));
        assert!(SESSIONS.contains("REFERENCES auth.oauth_clients(id) ON DELETE CASCADE"));
        assert!(SSO_PROVIDERS.contains("disabled"));
        assert!(SSO_PROVIDERS.contains("sso_providers_resource_id_pattern_idx"));
        assert!(SAML_PROVIDERS.contains("name_id_format"));
        assert!(SAML_RELAY_STATES.contains("flow_state_id"));
        assert!(SAML_RELAY_STATES.contains("DROP COLUMN from_ip_address"));
        assert!(!SAML_RELAY_STATES.contains("from_ip_address inet"));
        assert!(SCIM_USERS.contains("GENERATED ALWAYS AS (lower(resource->>'userName')) STORED"));
        assert!(SCIM_USERS.contains("COLLATE \"C\""));
        assert!(SCIM_TOKENS.contains("token_hash ~ '^[0-9a-f]{64}$'"));
        assert!(SCHEMA_MIGRATIONS.contains("schema_migrations_pkey"));
    }

    #[test]
    fn sso_sessions_is_dropped_after_create() {
        let create = SSO_SESSIONS
            .find("CREATE TABLE IF NOT EXISTS auth.sso_sessions")
            .expect("create");
        let drop = SSO_SESSIONS
            .find("DROP TABLE IF EXISTS auth.sso_sessions")
            .expect("drop");
        assert!(
            create < drop,
            "drop must follow create so the final shape matches the pin"
        );
    }

    #[test]
    fn parent_keys_are_stubs_not_claimed_units() {
        assert!(SCHEMA.contains("CREATE TABLE IF NOT EXISTS auth.flow_state"));
        assert!(SCHEMA.contains("CREATE TABLE IF NOT EXISTS auth.oauth_clients"));
        assert!(
            !SCHEMA.contains("encrypted_password"),
            "do not ship the full auth.users column list as a stub"
        );
        assert!(
            !SCHEMA.contains("provider_type"),
            "do not ship the full auth.flow_state column list in this issue"
        );
    }

    #[test]
    fn does_not_claim_sibling_units() {
        let sql = install_sql();
        assert!(
            !sql.contains("CREATE OR REPLACE FUNCTION auth.uid"),
            "auth.uid() is issue 10"
        );
        assert!(
            !sql.contains("CREATE TABLE IF NOT EXISTS auth.instances"),
            "auth.instances is issue 10"
        );
        assert!(
            !sql.contains("CREATE TABLE IF NOT EXISTS auth.mfa_factors"),
            "auth.mfa_factors is issue 11"
        );
        assert!(
            !sql.contains("CREATE TABLE IF NOT EXISTS auth.refresh_tokens"),
            "auth.refresh_tokens is issue 11"
        );
    }

    #[test]
    fn rejects_tls_required_urls() {
        assert!(require_cleartext_postgres("postgres://u@h/db").is_ok());
        assert!(require_cleartext_postgres("postgres://u@h/db?sslmode=disable").is_ok());
        assert!(require_cleartext_postgres("postgres://u@h/db?sslmode=prefer").is_ok());
        assert!(require_cleartext_postgres("host=h user=u dbname=db").is_ok());
        assert!(
            require_cleartext_postgres("postgres://u@h/db?application_name=sslmode=require")
                .is_ok()
        );
        assert!(require_cleartext_postgres("postgres://u:sslmode=require@h/db").is_ok());
        assert!(require_cleartext_postgres("application_name=sslmode=require host=h").is_ok());
        match require_cleartext_postgres("postgres://u@h/db?sslmode=require") {
            Err(SchemaError::TlsRequired { mode }) => assert_eq!(mode, "require"),
            other => panic!("{other:?}"),
        }
        assert!(require_cleartext_postgres("postgres://u@h/db?foo=1&sslmode=require").is_err());
        assert!(require_cleartext_postgres("postgres://u@h/db?sslmode=verify-full").is_err());
        assert!(require_cleartext_postgres("host=h sslmode=verify-ca user=u").is_err());
        let err = require_cleartext_postgres("postgres://u@h/db?sslmode=require").unwrap_err();
        assert!(err.to_string().contains("sslmode=require"), "{err}");
    }

    #[tokio::test]
    async fn install_schema_rejects_tls_before_connect() {
        let err = install_schema("postgres://u@h/db?sslmode=require")
            .await
            .expect_err("must not connect");
        match err {
            SchemaError::TlsRequired { mode } => assert_eq!(mode, "require"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn install_schema_times_out_when_postgres_does_not_answer() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let deadline = Duration::from_millis(200);
        let started = std::time::Instant::now();
        let err = install_schema_within(&format!("postgres://megabase@{addr}/db"), deadline)
            .await
            .expect_err("must time out");
        match err {
            SchemaError::TimedOut { timeout } => assert_eq!(timeout, deadline),
            other => panic!("{other:?}"),
        }
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "deadline must cancel a stalled handshake"
        );
        drop(listener);
    }
}
