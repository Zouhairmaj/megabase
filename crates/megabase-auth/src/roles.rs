//! Creates the standard API roles and Supabase admin roles a stock
//! PostgreSQL lacks.
//!
//! `anon`, `authenticated`, `service_role` and `authenticator` are cluster
//! objects applications depend on: migrations say `to authenticated` and
//! `grant ... to anon`, and REST `SET`s the JWT role for every statement.
//! `supabase_admin`, `supabase_auth_admin`, `supabase_storage_admin` and
//! `supabase_realtime_admin` are what dumps, policies and the realtime
//! schema name. The official image creates them at `initdb`; a stock
//! PostgreSQL has none. The SQL is `sql/roles.sql`. Spec:
//! `specs/core/roles.md`.

use std::time::Duration;

use crate::schema::{reject_tls, SchemaError};

/// Whole-install bound (connect + SQL). Startup fails instead of hanging.
const INSTALL_DEADLINE: Duration = Duration::from_secs(30);

const ROLES: &str = include_str!("../sql/roles.sql");

/// Failure creating the standard roles or their grants.
#[derive(Debug, thiserror::Error)]
pub enum RolesError {
    #[error("installing standard roles timed out after {timeout:?}")]
    TimedOut { timeout: Duration },
    #[error(
        "installing standard roles (anon, authenticated, service_role, authenticator, \
         supabase_admin, supabase_auth_admin, supabase_storage_admin, supabase_realtime_admin): {0}; \
         DATABASE_URL must connect as a superuser, or as a role with CREATEROLE that owns the public schema"
    )]
    Postgres(#[from] tokio_postgres::Error),
    #[error(transparent)]
    Connect(SchemaError),
}

impl From<SchemaError> for RolesError {
    fn from(error: SchemaError) -> Self {
        match error {
            SchemaError::Postgres(error) => Self::Postgres(error),
            other => Self::Connect(other),
        }
    }
}

/// Create the standard roles, admin roles, memberships, `public` usage and
/// default privileges when they are missing.
///
/// Idempotent. A role that already exists keeps its attributes: a cluster the
/// official stack initialised is not altered. Memberships and grants are
/// added only when absent. Runs in one transaction under
/// `pg_advisory_xact_lock(hashtext('megabase.core.install_roles'))`.
///
/// An attribute the connecting role is not allowed to grant (for example
/// `SUPERUSER` when it is not a superuser) is skipped. PostgreSQL reports
/// that as a `WARNING`, and this function logs it at `warn`. The role is
/// still created, and startup continues.
///
/// # Errors
///
/// `sslmode=require` fails before a socket opens, the same as
/// [`crate::install_schema`]. A connection or statement failure returns
/// [`RolesError::Postgres`] (for example `permission denied to create role`
/// when the connecting role lacks `CREATEROLE`). Connect plus SQL are bounded
/// by 30 seconds.
pub async fn install_roles(database_url: &str) -> Result<(), RolesError> {
    install_roles_within(database_url, INSTALL_DEADLINE).await
}

async fn install_roles_within(database_url: &str, deadline: Duration) -> Result<(), RolesError> {
    reject_tls(database_url)?;
    match tokio::time::timeout(deadline, install_roles_inner(database_url)).await {
        Ok(result) => result,
        Err(_) => Err(RolesError::TimedOut { timeout: deadline }),
    }
}

async fn install_roles_inner(database_url: &str) -> Result<(), RolesError> {
    let (client, connection) = tokio_postgres::connect(database_url, tokio_postgres::NoTls).await?;
    // Notices (the attribute WARNINGs) are only visible while this task
    // polls the connection. `schema::connect` awaits the connection future,
    // which drops them.
    let logged = tokio::spawn(log_role_notices(connection));
    // A failed batch leaves the session in an aborted transaction; the client
    // is dropped on return, which rolls back. Do not reuse it after an error.
    let executed = client.batch_execute(ROLES).await;
    drop(client);
    let _ = logged.await;
    executed?;
    Ok(())
}

/// Forward PostgreSQL notices from the role install to `tracing`.
///
/// `RAISE WARNING` in `roles.sql` is how a missing privilege degrades
/// without aborting the transaction. The message is the warning text.
async fn log_role_notices<S, T>(mut connection: tokio_postgres::Connection<S, T>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    T: tokio_postgres::tls::TlsStream + Unpin + Send + 'static,
{
    loop {
        match std::future::poll_fn(|cx| connection.poll_message(cx)).await {
            Some(Ok(tokio_postgres::AsyncMessage::Notice(notice))) => {
                tracing::warn!(severity = notice.severity(), "{}", notice.message());
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => {
                tracing::error!(%error, "roles postgres connection closed");
                break;
            }
            None => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const API_ROLES: [&str; 3] = ["anon", "authenticated", "service_role"];

    #[test]
    fn roles_sql_is_one_locked_transaction() {
        assert!(ROLES.contains("BEGIN;"), "{ROLES}");
        assert!(ROLES.trim_end().ends_with("COMMIT;"), "{ROLES}");
        let begin = ROLES.find("BEGIN;").expect("begin");
        let lock = ROLES
            .find("SELECT pg_advisory_xact_lock(hashtext('megabase.core.install_roles'));")
            .expect("lock");
        let first_create = ROLES.find("CREATE ROLE").expect("create");
        assert!(begin < lock && lock < first_create, "{ROLES}");
    }

    #[test]
    fn roles_match_the_official_image() {
        assert!(ROLES.contains("CREATE ROLE anon NOLOGIN NOINHERIT;"));
        assert!(ROLES.contains("CREATE ROLE authenticated NOLOGIN NOINHERIT;"));
        assert!(ROLES.contains("CREATE ROLE service_role NOLOGIN NOINHERIT BYPASSRLS;"));
        assert!(ROLES.contains("CREATE ROLE authenticator LOGIN NOINHERIT;"));
        assert_eq!(
            ROLES.matches("BYPASSRLS").count(),
            3,
            "service_role, the supabase_admin CREATE, and its attribute fallback"
        );
        // Four literal API-role statements, plus five admin-role format
        // strings (base and full for supabase_admin, base and full for the
        // auth/storage pair, and the realtime base).
        assert_eq!(ROLES.matches("CREATE ROLE").count(), 9);
        let lower = ROLES.to_ascii_lowercase();
        assert!(
            !lower.contains("password '"),
            "no password is set or embedded"
        );
        assert!(
            ROLES.contains("NOLOGIN SUPERUSER CREATEDB CREATEROLE INHERIT REPLICATION BYPASSRLS"),
            "supabase_admin is superuser-like"
        );
        assert!(
            !lower.contains("superuser;"),
            "no role statement ends by making an API role a superuser"
        );
        assert!(
            !lower.contains("grant postgres to authenticator"),
            "the Storage dev-only grant is not ported"
        );
    }

    #[test]
    fn every_role_is_created_only_when_missing() {
        for role in API_ROLES.iter().chain(["authenticator"].iter()) {
            let guard = format!("IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{role}')");
            let guard_at = ROLES
                .find(&guard)
                .unwrap_or_else(|| panic!("guard for {role}"));
            let create_at = ROLES
                .find(&format!("CREATE ROLE {role} "))
                .unwrap_or_else(|| panic!("create for {role}"));
            assert!(guard_at < create_at, "{role} is created inside its guard");
        }
        assert_eq!(
            ROLES
                .matches("EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;")
                .count(),
            4,
            "a concurrent CREATE ROLE on another database is tolerated"
        );
        assert!(
            !ROLES[..ROLES.find("admin_role").expect("admin block")].contains("ALTER ROLE"),
            "an existing API role keeps its attributes"
        );
    }

    #[test]
    fn admin_roles_are_nologin_and_created_only_when_missing() {
        for role in [
            "supabase_admin",
            "supabase_auth_admin",
            "supabase_storage_admin",
            "supabase_realtime_admin",
        ] {
            assert!(
                ROLES.contains(&format!("'{role}'")),
                "{role} is in the admin-role list"
            );
        }
        let guard = ROLES
            .find("IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = admin_role) THEN")
            .expect("skip when the role exists");
        let create = ROLES
            .find("CREATE ROLE %I NOLOGIN SUPERUSER")
            .expect("supabase_admin attributes");
        assert!(guard < create, "a role that already exists is not altered");
        assert!(ROLES.contains("NOLOGIN NOINHERIT CREATEROLE NOREPLICATION"));
        assert!(ROLES.contains("NOLOGIN NOINHERIT NOREPLICATION"));
        assert_eq!(
            ROLES
                .matches("WHEN duplicate_object OR unique_violation THEN")
                .count(),
            6,
            "admin creates tolerate duplicate_object and unique_violation too"
        );
        let degraded = ROLES
            .find("WHEN insufficient_privilege THEN")
            .expect("degrade");
        let alter = ROLES.find("ALTER ROLE").expect("attribute grant");
        assert!(
            degraded < alter,
            "ALTER ROLE runs only after the full CREATE was denied"
        );
        assert_eq!(ROLES.matches("ALTER ROLE").count(), 1);
        assert!(ROLES.contains(
            "RAISE WARNING 'megabase: connecting role cannot grant % to %; role created without it'"
        ));
        assert!(
            !ROLES.to_ascii_lowercase().contains("password '"),
            "admin roles get no password"
        );
    }

    #[test]
    fn memberships_are_granted_only_when_absent() {
        assert!(ROLES.contains("ARRAY['anon', 'authenticated', 'service_role']"));
        assert!(ROLES.contains("IF NOT pg_has_role('authenticator', api_role, 'MEMBER')"));
        assert!(ROLES.contains("EXECUTE format('GRANT %I TO authenticator', api_role);"));
        assert!(ROLES.contains("SELECT rolsuper INTO connect_is_super"));
        assert!(ROLES.contains("IF NOT connect_is_super"));
        assert!(ROLES.contains("NOT pg_has_role(connect_role, api_role, 'MEMBER')"));
        assert!(ROLES.contains("EXECUTE format('GRANT %I TO %I', api_role, connect_role);"));
    }

    #[test]
    fn public_schema_grants_match_the_official_image() {
        assert!(
            ROLES.contains("IF NOT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = 'public')")
        );
        assert!(ROLES.contains("grantees text := 'anon, authenticated, service_role';"));
        assert!(ROLES.contains("grantees := 'postgres, ' || grantees;"));
        assert!(ROLES.contains("EXECUTE 'GRANT USAGE ON SCHEMA public TO ' || grantees;"));
        for kind in ["TABLES", "FUNCTIONS", "SEQUENCES"] {
            let statement = format!(
                "EXECUTE 'ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT ALL ON {kind} TO ' || grantees;"
            );
            assert!(ROLES.contains(&statement), "{kind}");
        }
        assert!(
            !ROLES.contains("REVOKE"),
            "bootstrap never removes a privilege an operator granted"
        );
    }

    #[tokio::test]
    async fn sslmode_require_is_rejected_before_connecting() {
        let err = install_roles("postgres://u:secret@127.0.0.1:1/db?sslmode=require")
            .await
            .unwrap_err();
        assert!(
            matches!(err, RolesError::Connect(SchemaError::TlsRequired { .. })),
            "{err}"
        );
        assert!(!err.to_string().contains("secret"), "{err}");
    }

    #[tokio::test]
    async fn an_unreachable_server_is_a_postgres_error() {
        let err = install_roles("postgres://u:secret@127.0.0.1:1/db?connect_timeout=1")
            .await
            .unwrap_err();
        assert!(matches!(err, RolesError::Postgres(_)), "{err}");
        let text = err.to_string();
        assert!(text.contains("installing standard roles"), "{text}");
        assert!(text.contains("CREATEROLE"), "{text}");
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
        let err = install_roles_within(&url, Duration::from_millis(200))
            .await
            .unwrap_err();
        assert!(matches!(err, RolesError::TimedOut { .. }), "{err}");
        hold.abort();
    }
}
