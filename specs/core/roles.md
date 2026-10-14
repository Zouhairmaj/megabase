# Core: standard API roles (issue 319)

Status: complete
Unit ids (`coverage/units.json`): none. `coverage/units.json` is extracted
from `vendor/`, and the Supabase Postgres image that ships these roles is
not vendored. No `// megabase:unit` marker is claimed.
Level: 1

The Supabase Postgres image creates four cluster roles at `initdb`:
`anon`, `authenticated`, `service_role` and `authenticator`. Application
migrations say `create policy ... to authenticated` and `grant ... to
anon`, PostgREST connects as `authenticator` and `SET`s the JWT `role` for
every request, and the image grants the three API roles usage on `public`
plus default privileges on new `public` objects. On a stock PostgreSQL none
of that exists, so Megabase creates it at startup when `DATABASE_URL` is
set, before the Auth SQL objects in [`auth/database.md`](../auth/database.md).

## Upstream

The image's own init scripts are not in `vendor/`. Two vendored files
repeat that setup and are the port. The reference stack beats both
(GOAL.md §2); differences known from the pinned image are listed under
Edge cases.

| Behavior | File:line (pin) |
|---|---|
| Create `anon`, `authenticated` (`NOLOGIN NOINHERIT`), `service_role` (`NOLOGIN NOINHERIT BYPASSRLS`) and `authenticator` (`LOGIN NOINHERIT`) only when missing | [`vendor/storage/migrations/tenant/0002-storage-schema.sql:21`](https://github.com/supabase/storage/blob/db42280506a07a165376219b2d6ac7bc92ccdfcc/migrations/tenant/0002-storage-schema.sql#L21) (lines 21–35) |
| `grant anon, authenticated, service_role to authenticator` | [`vendor/storage/migrations/tenant/0002-storage-schema.sql:38`](https://github.com/supabase/storage/blob/db42280506a07a165376219b2d6ac7bc92ccdfcc/migrations/tenant/0002-storage-schema.sql#L38) (lines 38–40) |
| `grant usage on schema public` and `alter default privileges in schema public grant all on tables / functions / sequences` to the three API roles | [`vendor/supavisor/dev/postgres/00-setup.sql:5`](https://github.com/supabase/supavisor/blob/5a51fbfadaa6d6286c1fee0a95d4c629e81a2750/dev/postgres/00-setup.sql#L5) (lines 5–9) |
| `postgres` is also a grantee of usage and default privileges on the official stack | [`vendor/storage/migrations/tenant/0002-storage-schema.sql:43`](https://github.com/supabase/storage/blob/db42280506a07a165376219b2d6ac7bc92ccdfcc/migrations/tenant/0002-storage-schema.sql#L43) (lines 43–47, schema `storage`); [`vendor/supabase/docker/volumes/db/webhooks.sql:6`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/volumes/db/webhooks.sql#L6) (lines 6–9, schema `supabase_functions`) |
| `authenticator` is the PostgREST connection role and `anon` its anonymous role | [`vendor/supabase/docker/docker-compose.yml:270`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/docker-compose.yml#L270) (`PGRST_DB_URI`), line 274 (`PGRST_DB_ANON_ROLE`) |
| `authenticator` is `LOGIN NOINHERIT` and becomes the request role | [`vendor/postgrest/docs/references/auth.rst:23`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/docs/references/auth.rst#L23) |
| The official stack sets the `authenticator` password from `POSTGRES_PASSWORD` | [`vendor/supabase/docker/volumes/db/roles.sql:4`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/volumes/db/roles.sql#L4) |
| What each role is for | [`vendor/supabase/apps/docs/content/guides/database/postgres/roles.mdx:89`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/apps/docs/content/guides/database/postgres/roles.mdx#L89) (lines 89–105) |

Pins: storage `v1.80.2`, supavisor `v2.9.13`, supabase `v1.26.08`,
postgrest `v16.4` (`vendor.toml`). Ported SQL is
`crates/megabase-auth/sql/roles.sql` with the credit header required by
AGENTS.md. Rust: `megabase_auth::install_roles`.

## Inputs

- `DATABASE_URL`. Unset skips the install (HTTP still serves).
- The connecting role. It must be a superuser, or have `CREATEROLE` and own
  the `public` schema of that database (PostgreSQL 15+ makes the database
  owner that owner).

## Outputs

One transaction under
`pg_advisory_xact_lock(hashtext('megabase.core.install_roles'))`, run
before `install_schema`:

1. Each of `anon`, `authenticated`, `service_role`, `authenticator` is
   created only when `pg_roles` has no row for it, with the attributes in
   the table above. `duplicate_object` on the `CREATE` is swallowed
   (another Megabase on a different database of the same cluster won the
   race). An existing role is never altered.
2. `anon`, `authenticated`, `service_role` are granted to `authenticator`,
   and, when the connecting role is not a superuser, to the connecting role
   as well (it plays `authenticator`: REST runs
   `set_config('role', …)` for every statement). Each grant is skipped
   when `pg_has_role(member, role, 'MEMBER')` already holds.
3. When the database has a `public` schema: `GRANT USAGE ON SCHEMA public`
   and `ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT ALL ON TABLES`,
   `FUNCTIONS`, `SEQUENCES` to `anon, authenticated, service_role`, plus
   `postgres` when that role exists. Default privileges apply to objects
   the connecting role creates later, as on the official stack where that
   role is `supabase_admin` or `postgres`.

Nothing is revoked. Side effects after the install on an empty database:
`\du` lists the four roles; `create policy p on t to authenticated` and
`grant select on t to anon` succeed; a REST request with an `anon` or
`authenticated` JWT runs as that role.

## Errors

| When | Status | Code / message (upstream) |
|---|---|---|
| `sslmode=require` in `DATABASE_URL` | startup aborts | `DATABASE_URL requests TLS (sslmode=require); Megabase does not use TLS for PostgreSQL yet` (same as `install_schema`) |
| Connection or statement fails, for example `permission denied to create role` | startup aborts | `installing standard roles (anon, authenticated, service_role, authenticator): <postgres error>; DATABASE_URL must connect as a superuser, or as a role with CREATEROLE that owns the public schema` |
| Connect plus SQL exceed 30 s | startup aborts | `installing standard roles timed out after 30s` |

The process does not serve with roles half-installed: the transaction
rolls back and startup fails loudly (GOAL.md §3 rule 5).

## Edge cases

- **Already-initialised cluster** (the judge's `megabase` database on the
  pinned image, or any Supabase-managed cluster): every role exists, every
  membership holds, so steps 1 and 2 run no DDL. Step 3 grants `public`
  usage and default privileges in that database, which the image does for
  `postgres` only; new objects created there by the connecting role then
  carry the same grants as on the `postgres` database.
- **Attributes not ported.** The pinned image later runs
  `ALTER ROLE anon/authenticated/service_role INHERIT`, sets
  `statement_timeout` (`anon` 3s, `authenticated` 8s, `authenticator` 8s),
  `lock_timeout` 8s on `authenticator`, and
  `session_preload_libraries = 'safeupdate'` on `authenticator`. None of
  those are in `vendor/`, so they are not claimed. Role-level settings
  apply at login, not on `SET ROLE`, so they do not change served REST
  behavior. `safeupdate` is a Supabase-built extension; preloading it on a
  stock cluster would stop `authenticator` from logging in.
- **No password.** `authenticator` is created `LOGIN` without a password,
  as the Storage migration does. The official stack sets it from
  `POSTGRES_PASSWORD` (`roles.sql`); Megabase does not use that role and
  does not read that variable.
- **`grant postgres to authenticator`** in the Storage migration is its
  local dev shortcut and is not ported: on the official stack
  `authenticator` is a member of the three API roles only.
- **Non-superuser connecting role that did not create the roles.** The
  membership grants in step 2 need `ADMIN OPTION`; without it startup fails
  with `permission denied`, naming the privilege.
- **Concurrency.** Two Megabase processes on the same database serialise
  on the advisory lock. Two on different databases of one cluster may both
  attempt a `CREATE ROLE`; the loser's `duplicate_object` is ignored.

## Out of scope

`supabase_admin`, `supabase_auth_admin`, `supabase_storage_admin`,
`supabase_functions_admin`, `dashboard_user`, `pgbouncer`, the
`extensions` schema, `pgsodium` and the role settings listed above. Judge
cases cannot observe this unit: the judge's `megabase` database is on the
pinned image, where the roles pre-exist. An empty-database run is verified
by hand (issue #319) until a judge step for a stock cluster exists.

## Judge cases

None. The harness compares the two databases on one pinned cluster, so a
missing-roles case has no reference side.
