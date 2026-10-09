//! Installs Auth SQL objects applications and RLS policies call.
//!
//! Ported from supabase/auth migrations (MIT), pin v2.197.0. HTTP `/auth/v1`
//! is unchanged; this module is the database compatibility surface.

use tokio_postgres::NoTls;

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

const OBJECTS: &[&str] = &[
    SCHEMA,
    INSTANCES,
    AUDIT_LOG_ENTRIES,
    IDENTITIES,
    FLOW_STATE,
    MFA_AMR_CLAIMS,
    CUSTOM_OAUTH_PROVIDERS,
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
    #[error("installing auth schema: {0}")]
    Postgres(#[from] tokio_postgres::Error),
}

/// Create or replace the Auth SQL objects this crate implements.
///
/// Idempotent. Safe to run against a database the official Auth migrations
/// already applied. Aborts startup on error so a half-installed schema is
/// not served as if it were complete.
pub async fn install_schema(database_url: &str) -> Result<(), SchemaError> {
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
        assert!(FLOW_STATE.contains("Stores metadata for all OAuth/SSO login flows"));
        assert!(MFA_AMR_CLAIMS.contains("amr_id_pk"));
        assert!(MFA_AMR_CLAIMS.contains("REFERENCES auth.sessions(id) ON DELETE CASCADE"));
        assert!(CUSTOM_OAUTH_PROVIDERS.contains("custom_claims_allowlist"));
        assert!(CUSTOM_OAUTH_PROVIDERS.contains("provider_type IN ('oauth2', 'oidc')"));
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
}
