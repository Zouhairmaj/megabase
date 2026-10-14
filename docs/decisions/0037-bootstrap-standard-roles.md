# Megabase creates the standard API roles at startup

- Status: Accepted
- Date: 2026-10-14

Issue [#319](https://github.com/Zouhairmaj/megabase/issues/319). The
real-app test (nextjs-todo-list) could not run its migration on a stock
PostgreSQL: `anon`, `authenticated`, `service_role` and `authenticator`
did not exist. The Supabase Postgres image creates them at `initdb`, and
that image is the reference stack's database, not a vendored source.

**Decision.** Megabase bootstraps the roles (the issue's stated
preference) instead of documenting SQL for the operator. When
`DATABASE_URL` is set, `megabase_auth::install_roles` runs before
`install_schema`: it creates each role only when missing, grants the three
API roles to `authenticator` and to a non-superuser connecting role, and
grants `public` usage plus default privileges, all in one transaction.
Spec: [`specs/core/roles.md`](../../specs/core/roles.md).

**Source.** The image's init scripts are not in `vendor/`. The port uses
the two vendored files that repeat them: the Storage tenant migration
(`vendor/storage/migrations/tenant/0002-storage-schema.sql`, the
idempotent `CREATE ROLE` form) and the Supavisor dev setup
(`vendor/supavisor/dev/postgres/00-setup.sql`, `public` grants and
default privileges). Both upstreams are already in `NOTICE`. Attributes
the pinned image sets in later migrations and that no vendored file
contains (`INHERIT`, per-role `statement_timeout` and `lock_timeout`,
`session_preload_libraries = 'safeupdate'`) are not claimed; they apply at
login, not on `SET ROLE`, so served behavior is the same.

**Scope.** Existing roles are never altered and nothing is revoked, so a
cluster the official stack initialised is left as it is. `authenticator`
gets no password: Megabase does not connect as it. The connecting role
needs `CREATEROLE` (or superuser) only on a cluster that lacks the roles.

**Not chosen.** Documenting the SQL only: every new user would hit the
same startup failure. Putting the SQL in `megabase-core`: that crate has
no database client. A `// megabase:unit` marker: `coverage/units.json` has
no unit for an unvendored image, and a marker without a unit is a score
without a case.
