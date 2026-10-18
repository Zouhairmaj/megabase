# Bootstrap Supabase admin roles on a fresh cluster

- Status: Accepted
- Date: 2026-10-18

Issue [#493](https://github.com/Zouhairmaj/megabase/issues/493). On a stock
PostgreSQL the realtime schema install fails with
`role "supabase_admin" does not exist`, and a public-schema dump fails on
`GRANT ... TO "supabase_auth_admin"` and
`CREATE POLICY ... TO "supabase_auth_admin"`. Decision
[0037](0037-bootstrap-standard-roles.md) created only the four API roles
and left the admin roles out of scope.

**Decision.** `megabase_auth::install_roles` also creates
`supabase_admin`, `supabase_auth_admin`, `supabase_storage_admin` and
`supabase_realtime_admin` when `pg_roles` has no row for them. They are
`NOLOGIN` and have no password. `supabase_admin` is superuser-like
(`SUPERUSER CREATEDB CREATEROLE INHERIT REPLICATION BYPASSRLS`). The auth
and storage admin roles are `CREATEROLE NOINHERIT NOREPLICATION`.
`supabase_realtime_admin` is `NOINHERIT NOREPLICATION`, matching the pinned
Realtime migration. A concurrent `CREATE` that loses with
`duplicate_object` or `unique_violation` is ignored. An existing role is
not altered.

**Degrade.** When the full `CREATE` fails because the connecting role
cannot grant an attribute (`insufficient_privilege`), the role is created
with its `NOLOGIN` base and each attribute is applied separately. A denied
attribute is a PostgreSQL `WARNING` that startup logs. It does not fail
startup. The next start sees the role and does not retry the attribute.

**NOLOGIN.** The pinned image `supabase/postgres:17.6.1.136`
(`vendor/supabase/docker/docker-compose.yml`, supabase pin `v1.26.08`)
marks `supabase_admin`, `supabase_auth_admin` and `supabase_storage_admin`
`LOGIN`, because services connect as them and `roles.sql` sets passwords
from `POSTGRES_PASSWORD`. Megabase does not connect as those roles and
does not read that variable. `GRANT`, `CREATE POLICY` and
`AUTHORIZATION` need the role to exist, not a login. Creating a login
role with no password would add a principal this process does not use.

**Not chosen.** Altering roles that already exist, so a cluster the image
initialised would lose `LOGIN` or gain attributes the operator removed.
Setting passwords. Judge cases on the pinned cluster: both databases
already have the roles, so a case cannot observe a missing role. The live
check is `MEGABASE_ROLES_TEST_URL` in
`crates/megabase-auth/tests/admin_roles.rs`. Spec:
[`specs/core/roles.md`](../../specs/core/roles.md).
