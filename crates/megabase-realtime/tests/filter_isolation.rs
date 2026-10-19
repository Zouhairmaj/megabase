//! `list_changes` and `apply_rls` with two subscriptions on one table.
//!
//! One filter matches the inserted row. The other is stored, then
//! `check_equality_op` cannot evaluate it. Both functions still return
//! the matching subscription.
//!
//! Needs a superuser URL and `wal2json` (`wal_level=logical`):
//!
//! ```text
//! MEGABASE_REALTIME_PG_URL=postgres://postgres@127.0.0.1:54329/postgres \
//!   cargo test -p megabase-realtime --test filter_isolation -- --ignored --nocapture
//! ```

use megabase_realtime::install_schema;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

/// Not a valid `bytea` literal. `realtime.cast` stores it unchanged;
/// `check_equality_op` casts it and does not evaluate the comparison.
const UNEVALUATED_BYTEA: &str = r"\xzz";

#[tokio::test]
#[ignore = "needs MEGABASE_REALTIME_PG_URL and wal2json"]
async fn one_filter_error_leaves_the_other_subscriber() {
    let url = std::env::var("MEGABASE_REALTIME_PG_URL").expect("MEGABASE_REALTIME_PG_URL");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect");

    sqlx::raw_sql(
        "do $$ begin
            if not exists (select 1 from pg_roles where rolname = 'anon') then
                create role anon nologin noinherit;
            end if;
            if not exists (select 1 from pg_roles where rolname = 'authenticated') then
                create role authenticated nologin noinherit;
            end if;
            if not exists (select 1 from pg_roles where rolname = 'service_role') then
                create role service_role nologin noinherit bypassrls;
            end if;
            if not exists (select 1 from pg_roles where rolname = 'supabase_admin') then
                create role supabase_admin nologin noinherit;
            end if;
            if not exists (select 1 from pg_roles where rolname = 'supabase_realtime_admin') then
                create role supabase_realtime_admin nologin noinherit;
            end if;
        end $$;",
    )
    .execute(&pool)
    .await
    .expect("roles");

    install_schema(&pool).await.expect("schema");

    let suffix = Uuid::new_v4().simple().to_string();
    let table = format!("iso_{suffix}");
    let slot = format!("mb_{suffix}");
    let good = Uuid::new_v4();
    let bad = Uuid::new_v4();
    let result = exercise(&pool, &table, &slot, good, bad).await;
    cleanup(&pool, &table, &slot, &[good, bad]).await;
    result.expect("filter isolation");
}

async fn exercise(
    pool: &PgPool,
    table: &str,
    slot: &str,
    good: Uuid,
    bad: Uuid,
) -> Result<(), String> {
    let table = sql_ident(table);
    let slot = sql_ident(slot);
    sqlx::raw_sql(&format!(
        "create table public.{table} (
            id int primary key,
            body text not null,
            blob bytea
         );
         grant select on public.{table} to anon, authenticated, service_role;
         alter publication supabase_realtime add table public.{table};"
    ))
    .execute(pool)
    .await
    .map_err(|err| format!("table: {err}"))?;
    // A logical slot cannot be created in a transaction that already wrote.
    sqlx::query("select pg_create_logical_replication_slot($1, 'wal2json')")
        .bind(slot)
        .execute(pool)
        .await
        .map_err(|err| format!("create slot: {err}"))?;

    let cast_failed = sqlx::query("select $1::bytea")
        .bind(UNEVALUATED_BYTEA)
        .execute(pool)
        .await;
    if cast_failed.is_ok() {
        return Err("the stored filter value must not be a bytea literal".to_owned());
    }

    let claims = serde_json::json!({"role": "anon"});
    sqlx::query(
        "insert into realtime.subscription (subscription_id, entity, filters, claims, action_filter)
         values
           ($1, format('public.%I', $4::text)::regclass,
            array[row('body', 'eq', 'hello', false)::realtime.user_defined_filter],
            $3, '*'),
           ($2, format('public.%I', $4::text)::regclass,
            array[row('blob', 'eq', $5, false)::realtime.user_defined_filter],
            $3, '*')",
    )
    .bind(good)
    .bind(bad)
    .bind(&claims)
    .bind(table)
    .bind(UNEVALUATED_BYTEA)
    .execute(pool)
    .await
    .map_err(|err| format!("subscriptions: {err}"))?;

    let evaluated: Option<bool> = sqlx::query_scalar(
        "select realtime.check_equality_op('eq', 'bytea'::regtype, '\\x01', $1, false)",
    )
    .bind(UNEVALUATED_BYTEA)
    .fetch_one(pool)
    .await
    .map_err(|err| format!("check_equality_op: {err}"))?;
    if evaluated.is_some() {
        return Err("an unevaluable filter returns no match".to_owned());
    }

    sqlx::raw_sql(&format!(
        "insert into public.{table} (id, body, blob) values (1, 'hello', '\\x01')"
    ))
    .execute(pool)
    .await
    .map_err(|err| format!("insert: {err}"))?;

    let peeked_rows: Vec<String> = sqlx::query_scalar(
        "select data from pg_logical_slot_peek_changes(
            $1, null, null,
            'include-pk', 'true',
            'include-transaction', 'false',
            'include-timestamp', 'true',
            'include-type-oids', 'true',
            'format-version', '2',
            'actions', 'insert',
            'add-tables', $2
         )",
    )
    .bind(slot)
    .bind(format!("public.{table}"))
    .fetch_all(pool)
    .await
    .map_err(|err| format!("peek wal: {err}"))?;
    let peeked = peeked_rows
        .into_iter()
        .find(|data| data.contains(table))
        .ok_or_else(|| "wal for this table".to_owned())?;

    let applied: Vec<Uuid> = sqlx::query_scalar(
        "select unnest(subscription_ids) from realtime.apply_rls($1::jsonb, 1048576)",
    )
    .bind(&peeked)
    .fetch_all(pool)
    .await
    .map_err(|err| format!("apply_rls: {err}"))?;
    if !applied.contains(&good) {
        return Err(format!(
            "apply_rls dropped the matching subscription: {applied:?}"
        ));
    }
    if applied.contains(&bad) {
        return Err(format!(
            "apply_rls kept the subscription whose filter did not evaluate: {applied:?}"
        ));
    }

    let listed: Vec<(Option<Vec<Uuid>>,)> =
        sqlx::query_as("select subscription_ids from realtime.list_changes($1, $2, 100, 1048576)")
            .bind("supabase_realtime")
            .bind(slot)
            .fetch_all(pool)
            .await
            .map_err(|err| format!("list_changes: {err}"))?;
    let listed_ids: Vec<Uuid> = listed
        .into_iter()
        .filter_map(|(ids,)| ids)
        .flatten()
        .collect();
    if !listed_ids.contains(&good) {
        return Err(format!(
            "list_changes dropped the matching subscription: {listed_ids:?}"
        ));
    }
    if listed_ids.contains(&bad) {
        return Err(format!(
            "list_changes kept the subscription whose filter did not evaluate: {listed_ids:?}"
        ));
    }
    Ok(())
}

/// Removes only the table, slot, and subscription ids this test created.
async fn cleanup(pool: &PgPool, table: &str, slot: &str, ids: &[Uuid]) {
    let Ok(table) = try_sql_ident(table) else {
        return;
    };
    let Ok(slot) = try_sql_ident(slot) else {
        return;
    };
    let _ = sqlx::query("delete from realtime.subscription where subscription_id = any($1)")
        .bind(ids)
        .execute(pool)
        .await;
    let _ = sqlx::raw_sql(&format!("drop table if exists public.{table}"))
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "select pg_drop_replication_slot(slot_name)
         from pg_replication_slots where slot_name = $1",
    )
    .bind(slot)
    .execute(pool)
    .await;
}

fn sql_ident(name: &str) -> &str {
    try_sql_ident(name).expect("sql ident")
}

fn try_sql_ident(name: &str) -> Result<&str, ()> {
    let ok = !name.is_empty()
        && name.len() <= 63
        && name.as_bytes()[0].is_ascii_lowercase()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if ok {
        Ok(name)
    } else {
        Err(())
    }
}
