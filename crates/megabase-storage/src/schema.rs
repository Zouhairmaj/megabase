//! Installs the `storage.buckets` subset the bucket routes read.
//!
//! The SQL is ported from the storage-api tenant migrations; see the header
//! of `sql/buckets.sql`. The full `storage` schema is not installed, so the
//! `storage:sql-table:storage.buckets` unit carries no marker.

use std::time::Duration;

/// Whole-install bound. Startup fails instead of hanging.
const INSTALL_DEADLINE: Duration = Duration::from_secs(30);

const BUCKETS: &str = include_str!("../sql/buckets.sql");

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

/// Create `storage`, `storage.buckettype` and `storage.buckets` when missing.
///
/// Idempotent: every statement is guarded, so a database whose storage-api
/// migrations already ran is left as it is apart from re-applied grants.
///
/// # Errors
///
/// [`SchemaError::Sql`] when PostgreSQL rejects a statement (for example the
/// API roles are missing), [`SchemaError::TimedOut`] after 30 seconds.
pub async fn install_schema(pool: &sqlx::PgPool) -> Result<(), SchemaError> {
    match tokio::time::timeout(INSTALL_DEADLINE, sqlx::raw_sql(BUCKETS).execute(pool)).await {
        Ok(result) => result.map(|_| ()).map_err(SchemaError::from),
        Err(_) => Err(SchemaError::TimedOut {
            timeout: INSTALL_DEADLINE,
        }),
    }
}
