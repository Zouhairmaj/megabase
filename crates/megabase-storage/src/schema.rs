//! Installs the `storage` schema subset Megabase serves.
//!
//! The SQL is ported from the storage-api tenant migrations; each file under
//! `sql/` names its upstream migration. The full `storage` schema is not
//! installed (`specs/storage/database.md`), so the
//! `storage:sql-table:storage.buckets` unit carries no marker;
//! `storage.objects` is installed in its final shape.

use std::time::Duration;

/// Whole-install bound. Startup fails instead of hanging.
const INSTALL_DEADLINE: Duration = Duration::from_secs(30);

/// Serializes concurrent installs against one database.
const INSTALL_LOCK: &str =
    "SELECT pg_advisory_xact_lock(hashtext('megabase.storage.install_schema'))";

const BUCKETS: &str = include_str!("../sql/buckets.sql");

// megabase:unit storage:sql-table:storage.objects
// megabase:unit storage:sql-function:storage.protect_delete()
const OBJECTS_TABLE: &str = include_str!("../sql/objects.sql");

// megabase:unit storage:sql-function:storage.foldername()
// megabase:unit storage:sql-function:storage.filename()
const PATH_FUNCTIONS: &str = include_str!("../sql/path_functions.sql");

/// Dependency of the operation helpers; the unit itself is not claimed yet.
const OPERATION: &str = include_str!("../sql/operation.sql");

// megabase:unit storage:sql-function:storage.allow_only_operation()
const ALLOW_ONLY_OPERATION: &str = include_str!("../sql/allow_only_operation.sql");

// megabase:unit storage:sql-function:storage.allow_any_operation()
const ALLOW_ANY_OPERATION: &str = include_str!("../sql/allow_any_operation.sql");

// megabase:unit storage:sql-function:storage.enforce_bucket_name_length()
const ENFORCE_BUCKET_NAME_LENGTH: &str = include_str!("../sql/enforce_bucket_name_length.sql");

// megabase:unit storage:sql-function:storage.enforce_bucket_lifecycle_service_role()
const ENFORCE_BUCKET_LIFECYCLE_SERVICE_ROLE: &str =
    include_str!("../sql/enforce_bucket_lifecycle_service_role.sql");

// megabase:unit storage:sql-function:storage.extension()
const EXTENSION: &str = include_str!("../sql/extension.sql");

// Dropped upstream by 0052 and 0059; served by making sure they are absent.
// megabase:unit storage:sql-function:storage.add_prefixes()
// megabase:unit storage:sql-function:storage.delete_prefix()
// megabase:unit storage:sql-function:storage.delete_prefix_hierarchy_trigger()
// megabase:unit storage:sql-function:storage.delete_leaf_prefixes()
const DROPPED_PREFIX_FUNCTIONS: &str = include_str!("../sql/dropped_prefix_functions.sql");

// megabase:unit storage:sql-function:storage.get_common_prefix()
// megabase:unit storage:sql-function:storage.search()
const SEARCH: &str = include_str!("../sql/search.sql");

/// `list-v2` calls these. The sql-function units are not claimed.
const LIST_V2: &str = include_str!("../sql/list_v2.sql");

/// Install order: the table before its triggers, `storage.operation()`
/// before the `LANGUAGE sql` helpers that call it.
const OBJECTS: &[&str] = &[
    BUCKETS,
    OBJECTS_TABLE,
    PATH_FUNCTIONS,
    OPERATION,
    ALLOW_ONLY_OPERATION,
    ALLOW_ANY_OPERATION,
    ENFORCE_BUCKET_NAME_LENGTH,
    ENFORCE_BUCKET_LIFECYCLE_SERVICE_ROLE,
    EXTENSION,
    DROPPED_PREFIX_FUNCTIONS,
    SEARCH,
    LIST_V2,
];

/// Failure installing the storage schema. Display text does not include the URL.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    /// The install did not finish before the deadline.
    #[error("installing the storage schema timed out after {timeout:?}")]
    TimedOut { timeout: Duration },
    /// PostgreSQL rejected the install.
    #[error("installing the storage schema failed: {0}")]
    Sql(#[from] sqlx::Error),
}

/// Every statement after the advisory lock, as one script.
fn install_sql() -> String {
    let mut sql = String::new();
    for part in OBJECTS {
        sql.push_str(part);
        if !part.ends_with('\n') {
            sql.push('\n');
        }
    }
    sql
}

/// Create or replace the `storage` objects this crate implements.
///
/// Runs in one transaction under an advisory lock. Idempotent: every
/// statement is guarded or `CREATE OR REPLACE`, so a database whose
/// storage-api migrations already ran keeps the same definitions.
///
/// # Errors
///
/// [`SchemaError::Sql`] when PostgreSQL rejects a statement (for example the
/// API roles are missing), [`SchemaError::TimedOut`] after 30 seconds.
pub async fn install_schema(pool: &sqlx::PgPool) -> Result<(), SchemaError> {
    match tokio::time::timeout(INSTALL_DEADLINE, install(pool)).await {
        Ok(result) => result,
        Err(_) => Err(SchemaError::TimedOut {
            timeout: INSTALL_DEADLINE,
        }),
    }
}

async fn install(pool: &sqlx::PgPool) -> Result<(), SchemaError> {
    let mut tx = pool.begin().await?;
    sqlx::query(INSTALL_LOCK).execute(&mut *tx).await?;
    sqlx::raw_sql(&install_sql()).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(sql: &str, needle: &str) -> usize {
        sql.find(needle)
            .unwrap_or_else(|| panic!("`{needle}` missing from the install SQL"))
    }

    #[test]
    fn tables_and_dependencies_come_before_their_users() {
        let sql = install_sql();
        let buckets = position(&sql, "CREATE TABLE IF NOT EXISTS storage.buckets");
        let objects = position(&sql, "CREATE TABLE IF NOT EXISTS storage.objects");
        let operation = position(&sql, "CREATE OR REPLACE FUNCTION storage.operation()");
        assert!(buckets < objects, "objects references buckets");
        assert!(objects < position(&sql, "TRIGGER protect_objects_delete"));
        assert!(buckets < operation);
        assert!(
            position(
                &sql,
                "CREATE OR REPLACE FUNCTION storage.get_common_prefix("
            ) < position(
                &sql,
                "CREATE OR REPLACE FUNCTION storage.list_objects_with_delimiter("
            )
        );
        for helper in [
            "storage.allow_only_operation(",
            "storage.allow_any_operation(",
        ] {
            assert!(operation < position(&sql, helper), "{helper}");
        }
        for trigger in [
            "trigger enforce_bucket_name_length_trigger",
            "TRIGGER protect_bucket_control_update_role",
        ] {
            assert!(buckets < position(&sql, trigger), "{trigger}");
        }
    }

    #[test]
    fn dropped_prefix_functions_are_removed_not_created() {
        let sql = install_sql();
        let lower = sql.to_lowercase();
        for function in [
            "storage.add_prefixes(",
            "storage.delete_prefix(",
            "storage.delete_prefix_hierarchy_trigger(",
            "storage.delete_leaf_prefixes(",
        ] {
            assert!(
                sql.contains(&format!("DROP FUNCTION IF EXISTS {function}")),
                "{function}"
            );
            assert!(
                !lower.contains(&format!("create or replace function {function}"))
                    && !lower.contains(&format!("create function {function}")),
                "{function} is created"
            );
        }
    }
}
