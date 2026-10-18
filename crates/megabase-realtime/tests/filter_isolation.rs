//! `list_changes` and `apply_rls` with two subscriptions on one table.
//!
//! One filter matches the inserted row. The other does not evaluate.
//! Both functions still return the matching subscription.
//!
//! Needs a superuser URL and `wal2json` (`wal_level=logical`):
//!
//! ```text
//! MEGABASE_REALTIME_PG_URL=postgres://postgres@127.0.0.1:54329/postgres \
//!   cargo test -p megabase-realtime --test filter_isolation -- --ignored --nocapture
//! ```

use megabase_realtime::install_schema;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

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

    let slot = "mb_filter_iso";
    sqlx::raw_sql(
        "drop table if exists public.iso_notes;
         create table public.iso_notes (
            id int primary key,
            body text not null,
            blob bytea
         );
         grant select on public.iso_notes to anon, authenticated, service_role;
         do $$ begin
           if not exists (
             select 1 from pg_publication_tables
             where pubname = 'supabase_realtime'
               and schemaname = 'public'
               and tablename = 'iso_notes'
           ) then
             alter publication supabase_realtime add table public.iso_notes;
           end if;
         end $$;
         delete from realtime.subscription;",
    )
    .execute(&pool)
    .await
    .expect("table");
    // A logical slot cannot be created in a transaction that already wrote.
    sqlx::query(
        "select pg_drop_replication_slot(slot_name)
         from pg_replication_slots where slot_name = $1",
    )
    .bind(slot)
    .execute(&pool)
    .await
    .expect("drop slot");
    sqlx::query("select pg_create_logical_replication_slot($1, 'wal2json')")
        .bind(slot)
        .execute(&pool)
        .await
        .expect("create slot");

    let good = Uuid::new_v4();
    let bad = Uuid::new_v4();
    let claims = serde_json::json!({"role": "anon"});
    sqlx::query(
        "insert into realtime.subscription (subscription_id, entity, filters, claims, action_filter)
         values
           ($1, 'public.iso_notes'::regclass,
            array[row('body', 'eq', 'hello', false)::realtime.user_defined_filter],
            $3, '*'),
           ($2, 'public.iso_notes'::regclass,
            array[row('blob', 'eq', 'zz', false)::realtime.user_defined_filter],
            $3, '*')",
    )
    .bind(good)
    .bind(bad)
    .bind(claims)
    .execute(&pool)
    .await
    .expect("subscriptions");

    sqlx::query("insert into public.iso_notes (id, body, blob) values (1, 'hello', '\\x01')")
        .execute(&pool)
        .await
        .expect("insert");

    let peeked: String = sqlx::query_scalar(
        "select data from pg_logical_slot_peek_changes(
            $1, null, null,
            'include-pk', 'true',
            'include-transaction', 'false',
            'include-timestamp', 'true',
            'include-type-oids', 'true',
            'format-version', '2',
            'actions', 'insert',
            'add-tables', 'public.iso_notes'
         )",
    )
    .bind(slot)
    .fetch_one(&pool)
    .await
    .expect("peek wal");

    let applied: Vec<Uuid> = sqlx::query_scalar(
        "select unnest(subscription_ids) from realtime.apply_rls($1::jsonb, 1048576)",
    )
    .bind(&peeked)
    .fetch_all(&pool)
    .await
    .expect("apply_rls");
    assert!(
        applied.contains(&good),
        "apply_rls dropped the matching subscription: {applied:?}"
    );
    assert!(
        !applied.contains(&bad),
        "apply_rls kept the subscription whose filter did not evaluate: {applied:?}"
    );

    let listed: Vec<(Option<Vec<Uuid>>,)> =
        sqlx::query_as("select subscription_ids from realtime.list_changes($1, $2, 100, 1048576)")
            .bind("supabase_realtime")
            .bind(slot)
            .fetch_all(&pool)
            .await
            .expect("list_changes");
    let listed_ids: Vec<Uuid> = listed
        .into_iter()
        .filter_map(|(ids,)| ids)
        .flatten()
        .collect();
    assert!(
        listed_ids.contains(&good),
        "list_changes dropped the matching subscription: {listed_ids:?}"
    );
    assert!(
        !listed_ids.contains(&bad),
        "list_changes kept the subscription whose filter did not evaluate: {listed_ids:?}"
    );

    let _ = sqlx::query("select pg_drop_replication_slot($1)")
        .bind(slot)
        .execute(&pool)
        .await;
}
