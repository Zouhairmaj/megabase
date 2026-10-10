# Judge databases are separate

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

Issue #113. Side-effect checks compare the official cluster's `postgres`
database with a dedicated `megabase` database on the same instance
(`megabase-judge prepare` on host port 54322, then `DATABASE_URL=…/megabase`).
Sharing one database would make catalog and row comparisons vacuous or
inverted. Level 1 snapshots `auth.users` and `public.todos` after mutating
HTTP cases (per-case row delta) and compares `auth` table/function catalogs
including `pg_get_constraintdef`. A required catalog object missing on both
databases fails; `auth.sso_sessions` is required absent (`absent = true`).
A missing fixture snapshot table on the reference still aborts.
`storage.objects` stays Level 2.
