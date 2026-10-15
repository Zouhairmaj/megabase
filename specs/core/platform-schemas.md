# Core: platform schemas and the `supabase_realtime` publication (issue 383)

Status: complete
Unit ids (`coverage/units.json`): none. The Supabase Postgres image that
creates most of these objects is not vendored, so no unit is extracted for
them and no `// megabase:unit` marker is claimed.
Level: 1

A fresh Supabase project has per-database objects that dumps
(`supabase db dump`) and application migrations reference:

- schema `extensions`, holding `uuid-ossp`, `pgcrypto` and
  `pg_stat_statements`, on the default `search_path`;
- schema `supabase_functions` with `hooks`, `migrations` and the trigger
  function `http_request` used by database webhooks;
- schema `vault` (`vault.secrets`, `vault.decrypted_secrets`) from the
  `supabase_vault` extension;
- an empty publication `supabase_realtime`.

On a stock PostgreSQL, or a new database on the Supabase image, none of
them exist, so `alter publication supabase_realtime add table ...` or
`create extension ... with schema extensions` fails. Megabase creates them
at startup when `DATABASE_URL` is set, after the roles in
[`roles.md`](roles.md) and before the Auth objects in
[`auth/database.md`](../auth/database.md).

## Upstream

| Behavior | File:line (pin) |
|---|---|
| `supabase_functions` schema, usage and default privileges for `postgres, anon, authenticated, service_role`, `migrations` and `hooks` tables, indexes, `http_request()` body, `SECURITY DEFINER`, `search_path`, `REVOKE ... FROM PUBLIC`, `GRANT EXECUTE` | [`vendor/supabase/docker/volumes/db/webhooks.sql:5`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/volumes/db/webhooks.sql#L5) (lines 5–102, 203–207) |
| `pg_net` is installed into the pre-existing `extensions` schema | [`vendor/supabase/docker/volumes/db/webhooks.sql:3`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/volumes/db/webhooks.sql#L3) |
| An app recreates or extends the empty `supabase_realtime` publication | [`vendor/supabase/docker/dev/data.sql:29`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/dev/data.sql#L29) (lines 29–32) |
| Fixtures assume an `extensions` schema extensions are created in | [`vendor/postgrest/test/spec/fixtures/schema.sql:11`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/test/spec/fixtures/schema.sql#L11) |
| Realtime expects `public, extensions` on the path | [`vendor/realtime/dev/postgres/zb-supabase-schema.sql:12`](https://github.com/supabase/realtime/blob/d46540b3a4a79113f40301d5e5cb690f41150b9b/dev/postgres/zb-supabase-schema.sql#L12) |
| `vault.secrets` and `vault.decrypted_secrets` come from the extension | [`vendor/supabase/apps/docs/content/guides/database/vault.mdx:76`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/apps/docs/content/guides/database/vault.mdx#L76) |

The `extensions` usage grant, the extension list and the publication
follow the image's `00000000000000-initial-schema.sql` (supabase/postgres,
not vendored). The reference stack beats this table (GOAL.md §2).

Ported SQL is `crates/megabase-auth/sql/platform.sql` with the credit
header. Rust: `megabase_auth::install_platform`.

## Inputs

- `DATABASE_URL`. Unset skips the install (HTTP still serves).
- The connecting role must be a superuser or own the database (`CREATE` on
  the database for schemas and the publication, ownership for
  `ALTER DATABASE ... SET`).

## Outputs

One transaction under
`pg_advisory_xact_lock(hashtext('megabase.core.install_platform'))`:

1. `CREATE SCHEMA IF NOT EXISTS extensions`;
   `GRANT USAGE ON SCHEMA extensions` to `anon, authenticated,
   service_role`, plus `postgres` when that role exists.
2. For each of `uuid-ossp`, `pgcrypto`, `pg_stat_statements`, `pg_net`:
   when `pg_available_extensions` lists it and `pg_extension` does not,
   `CREATE EXTENSION <name> WITH SCHEMA extensions`. Then
   `CREATE SCHEMA IF NOT EXISTS vault` and
   `CREATE EXTENSION supabase_vault WITH SCHEMA vault` under the same
   condition (the schema is created first, so the install does not depend
   on the extension creating it; the extension creates the tables and
   view). A failure rolls the schema back with the extension. A failure of any of these is
   raised as a PostgreSQL `WARNING` (`megabase: extension <name> not
   created: <message>`) inside a subtransaction and the install continues.
3. When the database has no database-level `search_path`
   (`pg_db_role_setting` with `setrole = 0`):
   `ALTER DATABASE <current> SET search_path TO "$user", public, extensions`.
   New sessions resolve `uuid_generate_v4()` without a schema. REST sets its
   own `search_path` per request and is not affected.
4. When the `supabase_functions` schema is missing, the webhook setup from
   `webhooks.sql` lines 5–102 and 203–207 runs as written: schema, grants,
   default privileges (before the tables, so `hooks` and `migrations` get
   them), both tables and indexes, `migrations` rows `initial` and
   `20210809183423_update_grants`, and `http_request()` with
   `SECURITY DEFINER`, `search_path = supabase_functions`, execute revoked
   from `PUBLIC` and granted to the API roles (and `postgres`).
5. When `pg_publication` has no `supabase_realtime`:
   `CREATE PUBLICATION supabase_realtime` (no tables).

Nothing is revoked from an existing object. After the install the repro in
issue 383 succeeds:

```sql
create extension if not exists "uuid-ossp" with schema extensions;
alter publication supabase_realtime add table public.posts;
create trigger t after insert on public.posts for each row
  execute function supabase_functions.http_request('http://localhost/x','POST','{}','{}','1000');
```

## Errors

| When | Status | Message |
|---|---|---|
| `sslmode=require` in `DATABASE_URL` | startup aborts | same as `install_schema` |
| Connection or statement fails, for example `permission denied for database` | startup aborts | `installing platform schemas (extensions, supabase_functions, vault, publication supabase_realtime): <postgres error>; DATABASE_URL must connect as a superuser or as the owner of the database` |
| Connect plus SQL exceed 30 s | startup aborts | `installing platform schemas timed out after 30s` |
| An optional extension cannot be created | startup continues | PostgreSQL `WARNING` (step 2) |

## Edge cases

- **No `pg_net`** (stock PostgreSQL). `http_request()` and triggers that
  call it are created, because PL/pgSQL resolves `net.http_post` only when
  the trigger fires. Firing one fails with `schema "net" does not exist`;
  a webhook needs the Supabase image.
- **`pg_net` on the Supabase image in a non-`postgres` database.** The
  extension is created, but its background worker serves the database in
  `pg_net.database_name` (default `postgres`), so queued requests in
  another database are not sent.
- **No `supabase_vault`.** `vault` is not created (the schema is only created together with the extension); that is the "when
  available" case of the issue.
- **Extension already installed elsewhere** (for example `pgcrypto` in
  `public`): left there, not moved.
- **Database the official stack initialised.** Every object exists; no DDL
  runs except the grant in step 1.
- **Dumps that `drop publication if exists supabase_realtime` and recreate
  it** keep working.

## Out of scope

The `supabase_functions_admin` role and ownership transfers to it, the
`net` schema grants and `SECURITY DEFINER` changes on `net.http_*`, the
`extensions.grant_pg_net_access` event trigger, the removal of
`supabase_pg_net_admin` (`webhooks.sql` lines 103–202), `pgjwt` (not
shipped by current images), `graphql`, and the other extensions a
Supabase project lists but does not install by default.

## Judge cases

Not in this PR: cases live under `judge/`, a protected path, and land in a
separate `review/issue-383-cases` PR (AGENTS.md task workflow step 8).
Planned: compare `pg_namespace`, `pg_publication` and the schema ACLs of
`extensions` and `supabase_functions` between the `megabase` and
reference databases through Postgres Meta once it serves a query endpoint,
and restore a small dump that uses these objects.
