// Ported from supabase/realtime priv/repo/tenant_schema (Apache-2.0), pin v2.143.3.

//! Installs the `realtime` tenant schema subset that Postgres CDC
//! (`postgres_changes`) needs, plus the `supabase_realtime` publication.
//!
//! Each file under `sql/` starts from its upstream
//! `tenant_schema/realtime/<path>` file. `check_equality_op` and
//! `is_visible_through_filters` catch an evaluation error and return no
//! match for that subscription, so the shared poll continues for the
//! others. `subscription_check_filters` rejects an operator, column name,
//! or value the subscribe path does not accept. The upstream `create schema`,
//! `create type` and `create table` statements are not idempotent, so each
//! of those steps carries a catalog probe and is skipped when its object
//! already exists (a restart, or a database whose Realtime migrations
//! already ran). Functions use `create or replace` and run every time.
//! Broadcast, Presence authorization (`messages`, `send`, `topic`,
//! `authorize`) and the `list_changes_sync`/`settled_changes` helpers are
//! not installed.

use std::time::Duration;

/// Whole-install bound. Startup does not hang on a stuck database.
const INSTALL_DEADLINE: Duration = Duration::from_secs(30);

/// Serializes concurrent installs against one database.
const INSTALL_LOCK: &str =
    "SELECT pg_advisory_xact_lock(hashtext('megabase.realtime.install_schema'))";

/// One install step: an upstream SQL file and an optional probe that returns
/// `true` when the step's object already exists.
struct Step {
    name: &'static str,
    probe: Option<&'static str>,
    sql: &'static str,
}

const fn guarded(name: &'static str, probe: &'static str, sql: &'static str) -> Step {
    Step {
        name,
        probe: Some(probe),
        sql,
    }
}

const fn always(name: &'static str, sql: &'static str) -> Step {
    Step {
        name,
        probe: None,
        sql,
    }
}

/// Install order: the schema, then the types, then the functions the
/// `subscription` table needs (`to_regrole` for its generated column,
/// `subscription_check_filters` for its trigger), then the table, then the
/// `LANGUAGE sql` functions whose bodies are checked at creation time
/// (`list_changes` references `apply_rls` and `quote_wal2json`).
const STEPS: &[Step] = &[
    guarded(
        "schema",
        "SELECT to_regnamespace('realtime') IS NOT NULL",
        include_str!("../sql/schema.sql"),
    ),
    guarded(
        "types/action",
        "SELECT to_regtype('realtime.action') IS NOT NULL",
        include_str!("../sql/types/action.sql"),
    ),
    guarded(
        "types/equality_op",
        "SELECT to_regtype('realtime.equality_op') IS NOT NULL",
        include_str!("../sql/types/equality_op.sql"),
    ),
    guarded(
        "types/user_defined_filter",
        "SELECT to_regtype('realtime.user_defined_filter') IS NOT NULL",
        include_str!("../sql/types/user_defined_filter.sql"),
    ),
    guarded(
        "types/wal_column",
        "SELECT to_regtype('realtime.wal_column') IS NOT NULL",
        include_str!("../sql/types/wal_column.sql"),
    ),
    guarded(
        "types/wal_rls",
        "SELECT to_regtype('realtime.wal_rls') IS NOT NULL",
        include_str!("../sql/types/wal_rls.sql"),
    ),
    always(
        "functions/to_regrole",
        include_str!("../sql/functions/to_regrole.sql"),
    ),
    always("functions/cast", include_str!("../sql/functions/cast.sql")),
    always(
        "functions/check_equality_op",
        include_str!("../sql/functions/check_equality_op.sql"),
    ),
    always(
        "functions/build_prepared_statement_sql",
        include_str!("../sql/functions/build_prepared_statement_sql.sql"),
    ),
    always(
        "functions/is_visible_through_filters",
        include_str!("../sql/functions/is_visible_through_filters.sql"),
    ),
    always(
        "functions/wal2json_escape_identifier",
        include_str!("../sql/functions/wal2json_escape_identifier.sql"),
    ),
    always(
        "functions/quote_wal2json",
        include_str!("../sql/functions/quote_wal2json.sql"),
    ),
    always(
        "functions/subscription_check_filters",
        include_str!("../sql/functions/subscription_check_filters.sql"),
    ),
    guarded(
        "tables/subscription",
        "SELECT to_regclass('realtime.subscription') IS NOT NULL",
        include_str!("../sql/tables/subscription.sql"),
    ),
    always(
        "functions/apply_rls",
        include_str!("../sql/functions/apply_rls.sql"),
    ),
    always(
        "functions/list_changes",
        include_str!("../sql/functions/list_changes.sql"),
    ),
    always("publication", include_str!("../sql/publication.sql")),
];

/// Every `realtime.user_defined_filter` attribute and its type
/// (`types/user_defined_filter.sql`).
const FILTER_ATTRIBUTES: &[(&str, &str)] = &[
    ("column_name", "text"),
    ("op", "realtime.equality_op"),
    ("value", "text"),
    ("negate", "boolean"),
];

/// Every `realtime.subscription` column and its type
/// (`tables/subscription.sql`): the insert writes them, `apply_rls` reads
/// them, and `claims_role` is the role it runs under.
const SUBSCRIPTION_COLUMNS: &[(&str, &str)] = &[
    ("id", "bigint"),
    ("subscription_id", "uuid"),
    ("entity", "regclass"),
    ("claims", "jsonb"),
    ("created_at", "timestamp without time zone"),
    ("action_filter", "text"),
    ("selected_columns", "text[]"),
    ("filters", "realtime.user_defined_filter[]"),
    ("claims_role", "regrole"),
];

/// The unique index the insert's `on conflict` targets.
const SUBSCRIPTION_UNIQUE_INDEX: &str =
    "realtime.subscription_subscription_id_entity_filters_action_filter_selec";

/// `SELECT count(*) = <n>` of the attributes of `relid` (a SQL `oid`
/// expression) that carry the listed name and exact type.
fn attributes_probe(relid: &str, attributes: &[(&str, &str)]) -> String {
    let values = attributes
        .iter()
        .map(|(name, typ)| format!("('{name}', '{typ}')"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "(SELECT count(*) = {n} FROM pg_attribute a \
           JOIN (VALUES {values}) AS e(name, typ) \
             ON a.attname = e.name AND a.atttypid = e.typ::regtype \
           WHERE a.attrelid = {relid} AND a.attnum > 0 AND NOT a.attisdropped)",
        n = attributes.len(),
    )
}

/// `true` when the objects a guarded step may have skipped have the shape
/// the subscription SQL uses: every attribute with its type, the
/// `equality_op` values, the unique index the insert's `on conflict` names,
/// and the filter-check trigger.
fn shape_probe() -> String {
    let filter = attributes_probe(
        "(SELECT typrelid FROM pg_type WHERE oid = 'realtime.user_defined_filter'::regtype)",
        FILTER_ATTRIBUTES,
    );
    let subscription = attributes_probe("'realtime.subscription'::regclass", SUBSCRIPTION_COLUMNS);
    format!(
        "SELECT {filter} \
           AND {subscription} \
           AND (SELECT count(*) = 2 FROM pg_enum \
                  WHERE enumtypid = 'realtime.equality_op'::regtype \
                    AND enumlabel IN ('isdistinct', 'match')) \
           AND to_regclass('{SUBSCRIPTION_UNIQUE_INDEX}') IS NOT NULL \
           AND EXISTS (SELECT 1 FROM pg_trigger \
                  WHERE tgrelid = 'realtime.subscription'::regclass \
                    AND tgname = 'tr_check_filters' AND NOT tgisinternal)"
    )
}

/// Failure installing the realtime schema. Display text does not include the URL.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    /// The install did not finish before the deadline.
    #[error("installing the realtime schema timed out after {timeout:?}")]
    TimedOut {
        /// The deadline that passed.
        timeout: Duration,
    },
    /// A skipped object exists, but without the attributes the current
    /// subscription SQL writes (a database from an older Realtime).
    #[error("an existing realtime type or table has an outdated shape")]
    OutdatedShape,
    /// PostgreSQL rejected a step (or the surrounding transaction).
    #[error("installing the realtime schema failed at {step}: {source}")]
    Sql {
        /// The install step that failed, such as `tables/subscription`.
        step: &'static str,
        /// The database error.
        #[source]
        source: sqlx::Error,
    },
}

impl SchemaError {
    fn at(step: &'static str) -> impl FnOnce(sqlx::Error) -> Self {
        move |source| Self::Sql { step, source }
    }
}

/// Create the `realtime` objects Postgres CDC needs and the `supabase_realtime` publication.
///
/// Runs in one transaction under an advisory lock. Idempotent: objects that
/// cannot be replaced are skipped when they already exist, and functions are
/// `create or replace`, so restarts and databases whose Realtime migrations
/// already ran keep working.
///
/// # Errors
///
/// [`SchemaError::Sql`] when PostgreSQL rejects a step (for example the
/// `supabase_admin` or `supabase_realtime_admin` role is missing), and
/// [`SchemaError::TimedOut`] after 30 seconds.
pub async fn install_schema(pool: &sqlx::PgPool) -> Result<(), SchemaError> {
    match tokio::time::timeout(INSTALL_DEADLINE, install(pool)).await {
        Ok(result) => result,
        Err(_) => Err(SchemaError::TimedOut {
            timeout: INSTALL_DEADLINE,
        }),
    }
}

async fn install(pool: &sqlx::PgPool) -> Result<(), SchemaError> {
    let mut tx = pool.begin().await.map_err(SchemaError::at("begin"))?;
    sqlx::query(INSTALL_LOCK)
        .execute(&mut *tx)
        .await
        .map_err(SchemaError::at("lock"))?;
    for step in STEPS {
        if let Some(probe) = step.probe {
            let exists: bool = sqlx::query_scalar(probe)
                .fetch_one(&mut *tx)
                .await
                .map_err(SchemaError::at(step.name))?;
            if exists {
                continue;
            }
        }
        sqlx::raw_sql(step.sql)
            .execute(&mut *tx)
            .await
            .map_err(SchemaError::at(step.name))?;
    }
    let current: bool = sqlx::query_scalar(&shape_probe())
        .fetch_one(&mut *tx)
        .await
        .map_err(SchemaError::at("shape"))?;
    if !current {
        return Err(SchemaError::OutdatedShape);
    }
    tx.commit().await.map_err(SchemaError::at("commit"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "-- Ported from supabase/realtime priv/repo/tenant_schema/realtime/";

    fn index(name: &str) -> usize {
        STEPS
            .iter()
            .position(|step| step.name == name)
            .unwrap_or_else(|| panic!("step `{name}` missing"))
    }

    #[test]
    fn every_upstream_file_names_its_source() {
        for step in STEPS.iter().filter(|step| step.name != "publication") {
            let first = step.sql.lines().next().unwrap_or_default();
            assert!(
                first.starts_with(&format!("{HEADER}{}.sql (Apache-2.0)", step.name)),
                "{}: {first}",
                step.name
            );
            assert!(first.ends_with("pin v2.143.3."), "{}", step.name);
            assert!(
                step.sql.lines().skip(1).any(|line| !line.trim().is_empty()),
                "{} is empty",
                step.name
            );
        }
    }

    #[test]
    fn non_replaceable_objects_are_guarded() {
        for step in STEPS {
            let lower = step.sql.to_lowercase();
            let replaceable_only = !lower.contains("create schema")
                && !lower.contains("create type")
                && !lower.contains("create table");
            assert!(
                replaceable_only || step.probe.is_some(),
                "{} creates an object without a probe",
                step.name
            );
        }
    }

    #[test]
    fn dependencies_come_before_their_users() {
        let schema = index("schema");
        let table = index("tables/subscription");
        for ty in [
            "types/action",
            "types/equality_op",
            "types/user_defined_filter",
            "types/wal_column",
            "types/wal_rls",
        ] {
            assert!(schema < index(ty), "{ty}");
            assert!(index(ty) < index("functions/to_regrole"), "{ty}");
        }
        assert!(index("types/equality_op") < index("types/user_defined_filter"));
        assert!(index("functions/to_regrole") < table);
        assert!(index("functions/subscription_check_filters") < table);
        assert!(index("functions/cast") < index("functions/subscription_check_filters"));
        let check_equality_op = index("functions/check_equality_op");
        assert!(check_equality_op < index("functions/is_visible_through_filters"));
        let escape = index("functions/wal2json_escape_identifier");
        assert!(escape < index("functions/quote_wal2json"));
        let list_changes = index("functions/list_changes");
        assert!(table < index("functions/apply_rls"));
        assert!(index("functions/apply_rls") < list_changes);
        assert!(index("functions/quote_wal2json") < list_changes);
    }

    /// Each `(name, type)` is declared as `"name" type` in `sql`, and `sql`
    /// declares no other quoted attribute.
    fn declares_exactly(sql: &str, attributes: &[(&str, &str)]) {
        for (name, typ) in attributes {
            let quoted = format!("\"{name}\"");
            let declared = sql.lines().any(|line| {
                line.split_once(&quoted)
                    .is_some_and(|(_, rest)| rest.trim_start().starts_with(typ))
            });
            assert!(declared, "{name} {typ} is not declared");
        }
        let declared = sql
            .lines()
            .filter(|line| {
                let line = line.trim_start();
                line.starts_with('"') || line.starts_with("add column \"")
            })
            .count();
        assert_eq!(declared, attributes.len());
    }

    #[test]
    fn shape_probe_checks_every_attribute_and_its_type() {
        declares_exactly(
            STEPS[index("types/user_defined_filter")].sql,
            FILTER_ATTRIBUTES,
        );
        let table = STEPS[index("tables/subscription")].sql;
        declares_exactly(table, SUBSCRIPTION_COLUMNS);
        let (_, index_name) = SUBSCRIPTION_UNIQUE_INDEX.split_once('.').unwrap();
        assert!(table.contains(&format!("create unique index {index_name} on")));
        assert!(table.contains("create trigger tr_check_filters"));

        let probe = shape_probe();
        for (name, typ) in FILTER_ATTRIBUTES.iter().chain(SUBSCRIPTION_COLUMNS) {
            assert!(probe.contains(&format!("('{name}', '{typ}')")), "{name}");
        }
        assert!(probe.contains("count(*) = 4 FROM pg_attribute"));
        assert!(probe.contains("count(*) = 9 FROM pg_attribute"));
        assert!(probe.contains("a.atttypid = e.typ::regtype"));
        assert!(probe.contains(SUBSCRIPTION_UNIQUE_INDEX));
    }

    #[test]
    fn filter_evaluation_errors_stay_on_one_subscription() {
        let op = STEPS[index("functions/check_equality_op")].sql;
        assert!(op.contains("when others then"));
        assert!(op.contains("return null"));
        let visible = STEPS[index("functions/is_visible_through_filters")].sql;
        assert!(visible.contains("when others then"));
        assert!(visible.contains("return false"));
        let check = STEPS[index("functions/subscription_check_filters")].sql;
        assert!(check.contains("filter value exceeds the maximum length"));
        assert!(check.contains("'eq'::realtime.equality_op"));
        assert!(check.contains("'in'::realtime.equality_op"));
    }

    #[test]
    fn publication_is_created_only_when_missing() {
        let sql = STEPS[index("publication")].sql;
        let guard = "if not exists (select 1 from pg_publication \
                     where pubname = 'supabase_realtime')";
        assert!(sql.contains(guard));
        assert!(sql.contains("create publication supabase_realtime;"));
    }
}
