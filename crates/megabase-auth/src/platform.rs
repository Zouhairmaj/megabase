//! Creates the per-database objects a fresh Supabase project ships.
//!
//! Dumps from `supabase db dump` and application migrations reference the
//! `extensions` schema, `supabase_functions.http_request` (database
//! webhooks), `vault` and the `supabase_realtime` publication. The official
//! image creates them; a stock PostgreSQL has none, so restoring such a dump
//! fails. The SQL is `sql/platform.sql`, ported from the vendored webhook
//! setup. Spec: `specs/core/platform-schemas.md`.

use std::time::Duration;

use crate::schema::{connect, reject_tls, SchemaError};

/// Whole-install bound (connect + SQL). Startup fails instead of hanging.
const INSTALL_DEADLINE: Duration = Duration::from_secs(30);

const PLATFORM: &str = include_str!("../sql/platform.sql");

/// Failure creating the platform schemas or the `supabase_realtime`
/// publication.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("installing platform schemas timed out after {timeout:?}")]
    TimedOut { timeout: Duration },
    #[error(
        "installing platform schemas (extensions, supabase_functions, vault, publication supabase_realtime): {0}; \
         DATABASE_URL must connect as a superuser or as the owner of the database"
    )]
    Postgres(#[from] tokio_postgres::Error),
    #[error(transparent)]
    Connect(SchemaError),
}

impl From<SchemaError> for PlatformError {
    fn from(error: SchemaError) -> Self {
        match error {
            SchemaError::Postgres(error) => Self::Postgres(error),
            other => Self::Connect(other),
        }
    }
}

/// Create the `extensions` and `supabase_functions` schemas, the extensions
/// a fresh Supabase project has, `vault` when `supabase_vault` is available,
/// and the empty `supabase_realtime` publication, when they are missing.
///
/// Idempotent. Run after [`crate::install_roles`], whose roles the grants
/// name. Runs in one transaction under
/// `pg_advisory_xact_lock(hashtext('megabase.core.install_platform'))`.
/// An optional extension that cannot be created is logged by PostgreSQL as
/// a `WARNING` and skipped.
///
/// # Errors
///
/// `sslmode=require` fails before a socket opens, the same as
/// [`crate::install_schema`]. A connection or statement failure returns
/// [`PlatformError::Postgres`] (for example `permission denied for database`
/// when the connecting role may not create schemas or publications).
/// Connect plus SQL are bounded by 30 seconds.
pub async fn install_platform(database_url: &str) -> Result<(), PlatformError> {
    install_platform_within(database_url, INSTALL_DEADLINE).await
}

async fn install_platform_within(
    database_url: &str,
    deadline: Duration,
) -> Result<(), PlatformError> {
    reject_tls(database_url)?;
    match tokio::time::timeout(deadline, install_platform_inner(database_url)).await {
        Ok(result) => result,
        Err(_) => Err(PlatformError::TimedOut { timeout: deadline }),
    }
}

async fn install_platform_inner(database_url: &str) -> Result<(), PlatformError> {
    let client = connect(database_url).await?;
    // A failed batch leaves the session in an aborted transaction; the client
    // is dropped on return, which rolls back. Do not reuse it after an error.
    client.batch_execute(PLATFORM).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_sql_is_one_locked_transaction() {
        let begin = PLATFORM.find("BEGIN;").expect("begin");
        let lock = PLATFORM
            .find("SELECT pg_advisory_xact_lock(hashtext('megabase.core.install_platform'));")
            .expect("lock");
        let first_create = PLATFORM.find("CREATE SCHEMA").expect("create");
        assert!(begin < lock && lock < first_create, "{PLATFORM}");
        assert!(PLATFORM.trim_end().ends_with("COMMIT;"), "{PLATFORM}");
    }

    #[test]
    fn extensions_schema_is_usable_by_the_api_roles() {
        assert!(PLATFORM.contains("CREATE SCHEMA IF NOT EXISTS extensions;"));
        assert!(PLATFORM.contains("EXECUTE 'GRANT USAGE ON SCHEMA extensions TO ' || grantees;"));
        assert!(PLATFORM.contains("ARRAY['uuid-ossp', 'pgcrypto', 'pg_stat_statements', 'pg_net']"));
        assert!(
            PLATFORM.contains("EXECUTE format('CREATE EXTENSION %I WITH SCHEMA extensions', ext);")
        );
        assert!(
            PLATFORM.contains("SELECT 1 FROM pg_available_extensions WHERE name = ext"),
            "an extension the server does not ship is skipped"
        );
        assert!(
            PLATFORM.contains("ALTER DATABASE %I SET search_path TO \"$user\", public, extensions"),
            "extensions is on the default search_path"
        );
        assert!(
            PLATFORM.contains("s.setrole = 0"),
            "a database-level search_path an operator set is kept"
        );
    }

    #[test]
    fn vault_is_created_only_when_available() {
        let guard = PLATFORM
            .find("SELECT 1 FROM pg_available_extensions WHERE name = 'supabase_vault'")
            .expect("availability guard");
        let schema = PLATFORM
            .find("CREATE SCHEMA IF NOT EXISTS vault;")
            .expect("vault schema is created first");
        let create = PLATFORM
            .find("CREATE EXTENSION supabase_vault WITH SCHEMA vault;")
            .expect("create");
        let guard_end = guard
            + PLATFORM[guard..]
                .find("END IF;")
                .expect("availability guard closes");
        assert!(guard < schema && schema < create);
        assert!(
            create < guard_end,
            "the vault schema and extension stay inside the availability guard"
        );
    }

    #[test]
    fn supabase_functions_matches_the_webhooks_setup() {
        let guard = PLATFORM
            .find(
                "IF EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = 'supabase_functions') THEN",
            )
            .expect("guard");
        let create = PLATFORM
            .find("CREATE SCHEMA supabase_functions;")
            .expect("schema");
        assert!(guard < create, "an existing schema is left alone");
        for kind in ["TABLES", "FUNCTIONS", "SEQUENCES"] {
            let statement = format!(
                "ALTER DEFAULT PRIVILEGES IN SCHEMA supabase_functions GRANT ALL ON {kind} TO ' || grantees;"
            );
            let at = PLATFORM
                .find(&statement)
                .unwrap_or_else(|| panic!("{kind}"));
            assert!(
                at < PLATFORM
                    .find("CREATE TABLE supabase_functions.hooks")
                    .expect("hooks"),
                "default privileges apply to the tables created after them"
            );
        }
        assert!(PLATFORM.contains("CREATE FUNCTION supabase_functions.http_request()"));
        assert!(PLATFORM.contains("RETURNS trigger"));
        assert!(PLATFORM.contains("net.http_post("));
        assert!(
            PLATFORM.contains("ALTER FUNCTION supabase_functions.http_request() SECURITY DEFINER;")
        );
        assert!(PLATFORM.contains(
            "ALTER FUNCTION supabase_functions.http_request() SET search_path = supabase_functions;"
        ));
        assert!(PLATFORM
            .contains("REVOKE ALL ON FUNCTION supabase_functions.http_request() FROM PUBLIC;"));
        assert!(PLATFORM.contains(
            "EXECUTE 'GRANT EXECUTE ON FUNCTION supabase_functions.http_request() TO ' || grantees;"
        ));
        assert!(PLATFORM.contains("VALUES ('initial');"));
        assert!(PLATFORM.contains("VALUES ('20210809183423_update_grants');"));
    }

    #[test]
    fn realtime_publication_is_created_empty_when_missing() {
        let guard = PLATFORM
            .find("IF NOT EXISTS (SELECT 1 FROM pg_publication WHERE pubname = 'supabase_realtime') THEN")
            .expect("guard");
        let create = PLATFORM
            .find("CREATE PUBLICATION supabase_realtime;")
            .expect("create");
        assert!(guard < create);
        assert!(
            !PLATFORM.contains("FOR ALL TABLES"),
            "the publication starts empty"
        );
    }

    #[test]
    fn grants_include_postgres_only_when_it_exists() {
        assert_eq!(
            PLATFORM
                .matches("IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN")
                .count(),
            2
        );
        assert_eq!(
            PLATFORM
                .matches("grantees text := 'anon, authenticated, service_role';")
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn sslmode_require_is_rejected_before_connecting() {
        let err = install_platform("postgres://u:secret@127.0.0.1:1/db?sslmode=require")
            .await
            .unwrap_err();
        assert!(
            matches!(err, PlatformError::Connect(SchemaError::TlsRequired { .. })),
            "{err}"
        );
        assert!(!err.to_string().contains("secret"), "{err}");
    }

    #[tokio::test]
    async fn an_unreachable_server_is_a_postgres_error() {
        let err = install_platform("postgres://u:secret@127.0.0.1:1/db?connect_timeout=1")
            .await
            .unwrap_err();
        assert!(matches!(err, PlatformError::Postgres(_)), "{err}");
        let text = err.to_string();
        assert!(text.contains("installing platform schemas"), "{text}");
        assert!(!text.contains("secret"), "{text}");
    }

    #[tokio::test]
    async fn a_stalled_install_times_out() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let hold = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
            drop(socket);
        });
        let url = format!("postgres://u:p@127.0.0.1:{port}/db");
        let err = install_platform_within(&url, Duration::from_millis(200))
            .await
            .unwrap_err();
        assert!(matches!(err, PlatformError::TimedOut { .. }), "{err}");
        hold.abort();
    }
}
