//! Installs the `storage` schema subset Megabase serves.
//!
//! The SQL is ported from the storage-api tenant migrations; each file under
//! `sql/` names its upstream migration. The full `storage` schema is not
//! installed (`specs/storage/database.md`), so the
//! `storage:sql-table:storage.buckets` unit carries no marker.
//! `storage.objects` and the S3 multipart tables are installed in their
//! final shape.

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

// megabase:unit storage:sql-function:storage.operation()
const OPERATION: &str = include_str!("../sql/operation.sql");

// megabase:unit storage:sql-function:storage.allow_only_operation()
const ALLOW_ONLY_OPERATION: &str = include_str!("../sql/allow_only_operation.sql");

// megabase:unit storage:sql-function:storage.allow_any_operation()
const ALLOW_ANY_OPERATION: &str = include_str!("../sql/allow_any_operation.sql");

// megabase:unit storage:sql-function:storage.enforce_bucket_name_length()
const ENFORCE_BUCKET_NAME_LENGTH: &str = include_str!("../sql/enforce_bucket_name_length.sql");

// megabase:unit storage:sql-function:storage.protect_bucket_control_columns()
const PROTECT_BUCKET_CONTROL_COLUMNS: &str =
    include_str!("../sql/protect_bucket_control_columns.sql");

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

// megabase:unit storage:sql-function:storage.can_insert_object()
const CAN_INSERT_OBJECT: &str = include_str!("../sql/can_insert_object.sql");

// megabase:unit storage:sql-table:storage.s3_multipart_uploads
// megabase:unit storage:sql-table:storage.s3_multipart_uploads_parts
const S3_MULTIPART: &str = include_str!("../sql/s3_multipart.sql");

// megabase:unit storage:sql-function:storage.list_multipart_uploads_with_delimiter()
const LIST_MULTIPART: &str = include_str!("../sql/list_multipart_uploads_with_delimiter.sql");

// megabase:unit storage:sql-function:storage.get_common_prefix()
// megabase:unit storage:sql-function:storage.search()
const SEARCH: &str = include_str!("../sql/search.sql");

// megabase:unit storage:sql-function:storage.list_objects_with_delimiter()
const LIST_OBJECTS: &str = include_str!("../sql/list_objects_with_delimiter.sql");

// megabase:unit storage:sql-function:storage.search_by_timestamp()
const SEARCH_BY_TIMESTAMP: &str = include_str!("../sql/search_by_timestamp.sql");

// megabase:unit storage:sql-function:storage.search_v2()
const SEARCH_V2: &str = include_str!("../sql/search_v2.sql");

// megabase:unit storage:sql-function:storage.get_size_by_bucket()
const GET_SIZE_BY_BUCKET: &str = include_str!("../sql/get_size_by_bucket.sql");

/// Assigns owners when `supabase_storage_admin` (or `storage.super_user`)
/// already exists and the connecting role can replace the objects later.
/// Issue #493 creates that role; this file does not.
const FUNCTION_OWNERS: &str = include_str!("../sql/function_owners.sql");

/// Install order: the table before its triggers, `storage.operation()`
/// before the `LANGUAGE sql` helpers that call it, and
/// `storage.s3_multipart_uploads` before the `LANGUAGE sql` list function
/// that reads it. `storage.search_v2` is installed after the two functions
/// it calls.
const OBJECTS: &[&str] = &[
    BUCKETS,
    OBJECTS_TABLE,
    PATH_FUNCTIONS,
    OPERATION,
    ALLOW_ONLY_OPERATION,
    ALLOW_ANY_OPERATION,
    ENFORCE_BUCKET_NAME_LENGTH,
    PROTECT_BUCKET_CONTROL_COLUMNS,
    ENFORCE_BUCKET_LIFECYCLE_SERVICE_ROLE,
    EXTENSION,
    DROPPED_PREFIX_FUNCTIONS,
    CAN_INSERT_OBJECT,
    S3_MULTIPART,
    LIST_MULTIPART,
    SEARCH,
    LIST_OBJECTS,
    SEARCH_BY_TIMESTAMP,
    SEARCH_V2,
    GET_SIZE_BY_BUCKET,
    FUNCTION_OWNERS,
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
            "TRIGGER protect_bucket_control_insert",
            "TRIGGER protect_bucket_control_update ",
            "TRIGGER protect_bucket_control_update_role",
        ] {
            assert!(buckets < position(&sql, trigger), "{trigger}");
        }
        let uploads = position(
            &sql,
            "CREATE TABLE IF NOT EXISTS storage.s3_multipart_uploads",
        );
        let parts = position(
            &sql,
            "CREATE TABLE IF NOT EXISTS storage.s3_multipart_uploads_parts",
        );
        assert!(buckets < uploads, "multipart uploads reference buckets");
        assert!(uploads < parts, "parts reference uploads");
        assert!(
            parts
                < position(
                    &sql,
                    "CREATE OR REPLACE FUNCTION storage.list_multipart_uploads_with_delimiter("
                ),
            "the list function is LANGUAGE sql and needs the table at CREATE"
        );
        let common = position(
            &sql,
            "CREATE OR REPLACE FUNCTION storage.get_common_prefix(",
        );
        let list_objects = position(
            &sql,
            "CREATE OR REPLACE FUNCTION storage.list_objects_with_delimiter(",
        );
        let by_timestamp = position(
            &sql,
            "CREATE OR REPLACE FUNCTION storage.search_by_timestamp(",
        );
        let search_v2 = position(&sql, "CREATE OR REPLACE FUNCTION storage.search_v2(");
        assert!(common < list_objects);
        assert!(list_objects < by_timestamp);
        assert!(by_timestamp < search_v2);
        assert!(
            search_v2
                < position(
                    &sql,
                    "ALTER FUNCTION storage.search_v2(text, text, integer, integer, text, text, text, text, text, text, timestamptz, text, boolean)"
                )
        );
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

    #[test]
    fn route_and_rls_functions_match_the_pinned_migrations() {
        let sql = install_sql();
        // CI Build does not check out vendor/, so this compares the
        // install script to the ported files rather than the pin.
        for part in [
            OPERATION,
            CAN_INSERT_OBJECT,
            GET_SIZE_BY_BUCKET,
            PROTECT_BUCKET_CONTROL_COLUMNS,
            LIST_MULTIPART,
            LIST_OBJECTS,
            SEARCH_BY_TIMESTAMP,
            SEARCH_V2,
            S3_MULTIPART,
            FUNCTION_OWNERS,
        ] {
            assert!(
                sql.contains(part.trim()),
                "install SQL is missing a ported file"
            );
        }
        for older in [
            "DROP FUNCTION IF EXISTS storage.list_objects_with_delimiter(text, text, text, integer, text, text);",
            "DROP FUNCTION IF EXISTS storage.search_v2(text, text, integer, integer, text);",
            "DROP FUNCTION IF EXISTS storage.get_size_by_bucket();",
        ] {
            assert!(sql.contains(older), "{older}");
        }
        assert!(sql.contains("noncurrent_versions := COALESCE(noncurrent_versions, 'include');"));
        assert!(sql.contains("delete_markers := COALESCE(delete_markers, 'include');"));
        assert!(
            sql.contains("IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = super_user) THEN")
        );
        assert!(sql.contains("AND current_user IS DISTINCT FROM super_user THEN"));
        assert!(sql.contains("in_progress_size bigint NOT NULL DEFAULT 0"));
        assert!(sql.contains("size bigint NOT NULL DEFAULT 0"));
        assert!(sql.contains("GRANT SELECT ON TABLE storage.s3_multipart_uploads TO "));
        assert!(sql.contains("GRANT ALL ON TABLE storage.s3_multipart_uploads_parts TO "));
        assert!(!sql
            .to_lowercase()
            .contains("create role supabase_storage_admin"));
        assert!(!sql
            .to_lowercase()
            .contains("create user supabase_storage_admin"));
    }
}
