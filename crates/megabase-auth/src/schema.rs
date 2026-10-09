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

// megabase:unit auth:sql-function:auth.uid()
const AUTH_UID: &str = include_str!("../sql/auth.uid.sql");

// megabase:unit auth:sql-function:auth.role()
const AUTH_ROLE: &str = include_str!("../sql/auth.role.sql");

// megabase:unit auth:sql-function:auth.email()
const AUTH_EMAIL: &str = include_str!("../sql/auth.email.sql");

// megabase:unit auth:sql-function:auth.jwt()
const AUTH_JWT: &str = include_str!("../sql/auth.jwt.sql");

// megabase:unit auth:sql-table:auth.instances
const INSTANCES: &str = include_str!("../sql/instances.sql");

// megabase:unit auth:sql-table:auth.audit_log_entries
const AUDIT_LOG_ENTRIES: &str = include_str!("../sql/audit_log_entries.sql");

// megabase:unit auth:sql-table:auth.identities
const IDENTITIES: &str = include_str!("../sql/identities.sql");

// megabase:unit auth:sql-table:auth.flow_state
const FLOW_STATE: &str = include_str!("../sql/flow_state.sql");

// megabase:unit auth:sql-table:auth.mfa_amr_claims
const MFA_AMR_CLAIMS: &str = include_str!("../sql/mfa_amr_claims.sql");

// megabase:unit auth:sql-table:auth.custom_oauth_providers
const CUSTOM_OAUTH_PROVIDERS: &str = include_str!("../sql/custom_oauth_providers.sql");

// megabase:unit auth:sql-table:auth.mfa_factors
const MFA_FACTORS: &str = include_str!("../sql/mfa_factors.sql");

// megabase:unit auth:sql-table:auth.mfa_challenges
const MFA_CHALLENGES: &str = include_str!("../sql/mfa_challenges.sql");

// megabase:unit auth:sql-table:auth.mfa_recovery_code_sets
const MFA_RECOVERY_CODE_SETS: &str = include_str!("../sql/mfa_recovery_code_sets.sql");

// megabase:unit auth:sql-table:auth.mfa_recovery_codes
const MFA_RECOVERY_CODES: &str = include_str!("../sql/mfa_recovery_codes.sql");

// megabase:unit auth:sql-table:auth.oauth_clients
const OAUTH_CLIENTS: &str = include_str!("../sql/oauth_clients.sql");

// megabase:unit auth:sql-table:auth.oauth_client_states
const OAUTH_CLIENT_STATES: &str = include_str!("../sql/oauth_client_states.sql");

// megabase:unit auth:sql-table:auth.oauth_authorizations
const OAUTH_AUTHORIZATIONS: &str = include_str!("../sql/oauth_authorizations.sql");

// megabase:unit auth:sql-table:auth.oauth_consents
const OAUTH_CONSENTS: &str = include_str!("../sql/oauth_consents.sql");

// megabase:unit auth:sql-table:auth.one_time_tokens
const ONE_TIME_TOKENS: &str = include_str!("../sql/one_time_tokens.sql");

// megabase:unit auth:sql-table:auth.refresh_tokens
const REFRESH_TOKENS: &str = include_str!("../sql/refresh_tokens.sql");

const OBJECTS: &[&str] = &[
    SCHEMA,
    INSTANCES,
    AUDIT_LOG_ENTRIES,
    IDENTITIES,
    FLOW_STATE,
    MFA_AMR_CLAIMS,
    CUSTOM_OAUTH_PROVIDERS,
    MFA_FACTORS,
    MFA_CHALLENGES,
    MFA_RECOVERY_CODE_SETS,
    MFA_RECOVERY_CODES,
    OAUTH_CLIENTS,
    OAUTH_CLIENT_STATES,
    OAUTH_AUTHORIZATIONS,
    OAUTH_CONSENTS,
    ONE_TIME_TOKENS,
    REFRESH_TOKENS,
    AUTH_UID,
    AUTH_ROLE,
    AUTH_EMAIL,
    AUTH_JWT,
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
    fn function_bodies_match_upstream_gucs() {
        assert!(AUTH_UID.contains("request.jwt.claim.sub"));
        assert!(AUTH_UID.contains("request.jwt.claims"));
        assert!(AUTH_UID.contains("language sql stable"));
        assert!(AUTH_ROLE.contains("request.jwt.claim.role"));
        assert!(AUTH_EMAIL.contains("request.jwt.claim.email"));
        assert!(AUTH_JWT.contains("current_setting('request.jwt.claim', true)"));
        assert!(AUTH_JWT.contains("current_setting('request.jwt.claims', true)"));
        assert!(
            !AUTH_JWT.contains("request.jwt.claim.sub"),
            "auth.jwt() reads the whole claim document, not .sub"
        );
    }

    #[test]
    fn identities_does_not_rename_id() {
        let lower = IDENTITIES.to_ascii_lowercase();
        assert!(
            !lower.contains("rename column"),
            "renaming id breaks an already-migrated UUID primary key"
        );
        assert!(IDENTITIES.contains("provider_id"));
        assert!(IDENTITIES.contains("GENERATED ALWAYS AS (lower(identity_data->>'email')) STORED"));
        assert!(IDENTITIES.contains("REFERENCES auth.users(id) ON DELETE CASCADE"));
    }

    #[test]
    fn tables_declare_final_columns() {
        assert!(INSTANCES.contains("raw_base_config"));
        assert!(AUDIT_LOG_ENTRIES.contains("ip_address"));
        assert!(FLOW_STATE.contains("code_challenge_method"));
        assert!(FLOW_STATE.contains("authentication_method"));
        assert!(FLOW_STATE.contains("email_optional"));
        assert!(FLOW_STATE.contains("ALTER COLUMN provider_type SET NOT NULL"));
        assert!(FLOW_STATE.contains("ALTER COLUMN authentication_method SET NOT NULL"));
        assert!(FLOW_STATE.contains("Stores metadata for all OAuth/SSO login flows"));
        assert!(IDENTITIES.contains("ALTER COLUMN provider_id SET NOT NULL"));
        assert!(MFA_AMR_CLAIMS.contains("ALTER COLUMN session_id SET NOT NULL"));
        assert!(MFA_AMR_CLAIMS.contains("amr_id_pk"));
        assert!(MFA_AMR_CLAIMS.contains("REFERENCES auth.sessions(id) ON DELETE CASCADE"));
        assert!(CUSTOM_OAUTH_PROVIDERS.contains("custom_claims_allowlist"));
        assert!(CUSTOM_OAUTH_PROVIDERS.contains("provider_type IN ('oauth2', 'oidc')"));
        assert!(MFA_FACTORS.contains("last_webauthn_challenge_data"));
        assert!(MFA_FACTORS.contains("'recovery_code'"));
        assert!(MFA_FACTORS.contains("unique_phone_factor_per_user"));
        assert!(MFA_FACTORS.contains("DROP CONSTRAINT IF EXISTS mfa_factors_phone_key"));
        assert!(MFA_FACTORS.contains("ALTER COLUMN factor_type SET NOT NULL"));
        assert!(MFA_CHALLENGES.contains("otp_code"));
        assert!(MFA_CHALLENGES.contains("web_authn_session_data"));
        assert!(MFA_CHALLENGES.contains("mfa_challenge_created_at_idx"));
        assert!(MFA_CHALLENGES.contains("REFERENCES auth.mfa_factors(id) ON DELETE CASCADE"));
        assert!(MFA_RECOVERY_CODE_SETS.contains("failed_verification_count"));
        assert!(
            MFA_RECOVERY_CODE_SETS.contains("REFERENCES auth.mfa_factors (id) ON DELETE CASCADE")
        );
        assert!(MFA_RECOVERY_CODES.contains("code_hash"));
        assert!(MFA_RECOVERY_CODES.contains("mfa_recovery_codes_set_id_idx"));
        assert!(OAUTH_CLIENTS.contains("token_endpoint_auth_method"));
        assert!(OAUTH_CLIENTS.contains("DROP COLUMN IF EXISTS client_id"));
        assert!(!OAUTH_CLIENTS.contains("constraint oauth_clients_client_id_key unique"));
        assert!(OAUTH_CLIENTS.contains("ALTER COLUMN client_secret_hash DROP NOT NULL"));
        assert!(OAUTH_CLIENT_STATES.contains("provider_type"));
        assert!(OAUTH_CLIENT_STATES.contains(
            "Stores OAuth states for third-party provider authentication flows where Supabase acts as the OAuth client."
        ));
        assert!(OAUTH_AUTHORIZATIONS.contains("oauth_auth_pending_exp_idx"));
        assert!(OAUTH_AUTHORIZATIONS.contains("nonce"));
        assert!(OAUTH_AUTHORIZATIONS.contains("code_challenge_method"));
        assert!(OAUTH_CONSENTS.contains("oauth_consents_user_client_unique"));
        assert!(OAUTH_CONSENTS.contains("revoked_at IS NULL OR revoked_at >= granted_at"));
        assert!(ONE_TIME_TOKENS.contains("timestamp WITHOUT TIME ZONE"));
        assert!(ONE_TIME_TOKENS.contains("expires_at"));
        assert!(ONE_TIME_TOKENS.contains("USING hash (token_hash)"));
        assert!(ONE_TIME_TOKENS.contains("WHEN undefined_object OR feature_not_supported"));
        assert!(
            !ONE_TIME_TOKENS.contains("WHEN OTHERS"),
            "hash fallback must not swallow lock/permission errors"
        );
        let add_user_id = ONE_TIME_TOKENS
            .find("ADD COLUMN IF NOT EXISTS user_id")
            .expect("user_id repair");
        let unique_idx = ONE_TIME_TOKENS
            .find("one_time_tokens_user_id_token_type_key")
            .expect("unique index");
        let hash_idx = ONE_TIME_TOKENS
            .find("USING hash (token_hash)")
            .expect("hash index");
        assert!(
            add_user_id < unique_idx,
            "unique index must follow column repairs"
        );
        assert!(
            add_user_id < hash_idx,
            "hash indexes must follow column repairs"
        );
        assert!(REFRESH_TOKENS.contains("user_id varchar(255)"));
        assert!(REFRESH_TOKENS.contains("DROP CONSTRAINT IF EXISTS refresh_tokens_parent_fkey"));
        assert!(REFRESH_TOKENS.contains("DROP INDEX IF EXISTS auth.refresh_tokens_token_idx"));
        assert!(REFRESH_TOKENS.contains("refresh_tokens_session_id_fkey"));
        assert!(REFRESH_TOKENS.contains("parent varchar(255)"));
    }

    #[test]
    fn issue_11_tables_are_installed() {
        let sql = install_sql();
        for name in [
            "auth.mfa_factors",
            "auth.mfa_challenges",
            "auth.mfa_recovery_code_sets",
            "auth.mfa_recovery_codes",
            "auth.oauth_clients",
            "auth.oauth_client_states",
            "auth.oauth_authorizations",
            "auth.oauth_consents",
            "auth.one_time_tokens",
            "auth.refresh_tokens",
        ] {
            assert!(
                sql.contains(&format!("CREATE TABLE IF NOT EXISTS {name}")),
                "missing {name}"
            );
        }
    }

    #[test]
    fn parent_keys_are_stubs_not_claimed_units() {
        assert!(SCHEMA.contains("CREATE TABLE IF NOT EXISTS auth.users"));
        assert!(SCHEMA.contains("CREATE TABLE IF NOT EXISTS auth.sessions"));
        assert!(
            !SCHEMA.contains("encrypted_password"),
            "do not ship the full auth.users column list in this issue"
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
