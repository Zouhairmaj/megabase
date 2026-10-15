# Storage: database (issue 58)

Status: draft
Level: 2

Issue #58 ports the SQL objects storage-api's tenant migrations create in
the `storage` schema. RLS policies, clients and storage-api itself call
them inside PostgreSQL. This file covers the units of PR 1; later PRs add
their units here.

Pin: `vendor/storage` `v1.80.2` (`db42280506a07a165376219b2d6ac7bc92ccdfcc`),
Apache-2.0. storage-api runs every file in `migrations/tenant/` in order
(`src/internal/database/migrations/types.ts`), so the desired state is the
**final** definition after the last migration, not the first `CREATE`.

## Install

When `DATABASE_URL` is set, `megabase_storage::install_schema`
(`crates/megabase-storage/src/schema.rs`) runs one transaction under
`pg_advisory_xact_lock(hashtext('megabase.storage.install_schema'))`:
`sql/buckets.sql`, then the files below in install order. Every statement
is guarded or `CREATE OR REPLACE`, so a restart, or a database the
storage-api migrations already reached the pin on, keeps the same
definitions. Function owners and the `storage.install_roles` role setup in
`0002-storage-schema.sql` are not ported; functions are owned by the
`DATABASE_URL` role and keep PostgreSQL's default `EXECUTE` for `PUBLIC`.

The storage service role is read as upstream does:
`COALESCE(current_setting('storage.service_role', true), 'service_role')`.

---

## `storage:sql-function:storage.allow_only_operation()`

Upstream: `migrations/tenant/0058-operation-ergonomics.sql:4`.
File: `sql/allow_only_operation.sql`.

**Inputs.** `expected_operation text`. Reads `storage.operation()`, the
transaction-local GUC `storage.operation` that storage-api sets per
request (`src/internal/database/postgres/scope.ts:21`).

**Output.** `boolean`, `LANGUAGE sql`, `STABLE`. A leading `storage.` is
stripped from both the current and the expected operation (`substr(x, 9)`).
`false` when the expected operation is `NULL` or `''`; otherwise whether the
two are equal, with `NULL` (no current operation) coerced to `false`.

**Errors.** None.

**Dependency.** `LANGUAGE sql` bodies are checked at `CREATE`, so
`storage.operation()` (`0024-operation-function.sql:1`, `sql/operation.sql`)
is installed first. That unit carries no marker in this PR.

## `storage:sql-function:storage.allow_any_operation()`

Upstream: `migrations/tenant/0058-operation-ergonomics.sql:31`.
File: `sql/allow_any_operation.sql`.

**Inputs.** `expected_operations text[]`; reads `storage.operation()`.

**Output.** `boolean`, `LANGUAGE sql`, `STABLE`. `true` when any non-`NULL`,
non-empty element equals the current operation after both drop a leading
`storage.`. A `NULL` array, an empty array, or no current operation give
`false`.

**Errors.** None.

## `storage:sql-function:storage.enforce_bucket_name_length()`

Upstream: `migrations/tenant/0037-add-bucket-name-length-trigger.sql:1`.
File: `sql/enforce_bucket_name_length.sql`.

**Trigger.** `enforce_bucket_name_length_trigger` `BEFORE INSERT OR UPDATE
OF name ON storage.buckets FOR EACH ROW`. Upstream drops and creates it;
Megabase uses `CREATE OR REPLACE TRIGGER` (PostgreSQL 14+) to the same
result.

**Output.** Returns `NEW` unchanged when `length(NEW.name) <= 100`.

**Errors.** `length(NEW.name) > 100` raises `P0001`
`bucket name "<name>" is too long (<n> characters). Max is 100.`
`length` counts characters, not bytes.

## `storage:sql-function:storage.enforce_bucket_lifecycle_service_role()`

Upstream: `migrations/tenant/0068-bucket-lifecycle-configuration.sql:115`,
trigger at `:155`. File: `sql/enforce_bucket_lifecycle_service_role.sql`.

**Trigger.** `protect_bucket_control_update_role` `AFTER UPDATE OF
lifecycle_configuration, lifecycle_configuration_generation ON
storage.buckets FOR EACH ROW`, argument `TG_ARGV[0]` = the storage service
role at install time. Function has `SET search_path = pg_catalog`.

**Output.** `NULL` (an `AFTER` row trigger's result is ignored).

**Errors.** When `current_user` is not `TG_ARGV[0]` and either lifecycle
column changed (`IS DISTINCT FROM`), raises SQLSTATE `PST01`
`bucket control columns may only be changed by the configured storage service role`
with `SCHEMA` `storage`, `TABLE` `buckets`, `CONSTRAINT`
`protect_bucket_control_update_role`. Membership is not enough: the check
compares the role name.

**Edge cases.** `UPDATE ... SET lifecycle_configuration = lifecycle_configuration`
fires the trigger but changes nothing, so it passes. The `BEFORE` triggers
from the same migration (`storage.protect_bucket_control_columns()`, PR 3)
and the lifecycle `CHECK` constraints are not installed yet, so an update
they would reject first (`0A000`, `22023`, `23514`) reaches this trigger
instead.

## `storage:sql-function:storage.extension()`

Upstream: created `migrations/tenant/0002-storage-schema.sql:109`, replaced
by `0036-optimise-existing-functions.sql:37` and `:72`, final body
`0060-optimize-existing-functions-again.sql:4`. File: `sql/extension.sql`.

**Inputs.** `name text`.

**Output.** `text`, `LANGUAGE plpgsql`, `IMMUTABLE`. Takes the last `/`
segment, then the text after its last `.`.

| `name` | Result |
|---|---|
| `folder/avatar.png` | `png` |
| `a.tar.gz` | `gz` |
| `folder/README` | `README` (no dot: the whole segment) |
| `folder/` | `''` |
| `''` | `NULL` (`string_to_array('', '/')` is `{}`) |
| `NULL` | `NULL` |

**Errors.** None.

## Units dropped before the pin

| Unit | Created | Dropped |
|---|---|---|
| `storage:sql-function:storage.add_prefixes()` | `0026-objects-prefixes.sql:59` | `0052-drop-not-used-indexes-and-functions.sql:96` |
| `storage:sql-function:storage.delete_prefix()` | `0026-objects-prefixes.sql:78` | `0052-drop-not-used-indexes-and-functions.sql:89` |
| `storage:sql-function:storage.delete_prefix_hierarchy_trigger()` | `0026-objects-prefixes.sql:133` | `0052-drop-not-used-indexes-and-functions.sql:54` |
| `storage:sql-function:storage.delete_leaf_prefixes()` | `0040-fix-prefix-race-conditions-optimized.sql:33` | `0059-drop-unused-functions.sql:4` |

Final shape: none of these functions exist, so calling one fails with
`42883` (`function ... does not exist`) on the reference stack.
`sql/dropped_prefix_functions.sql` runs the upstream `DROP FUNCTION IF
EXISTS` statements (the `0052` ones inside `EXCEPTION WHEN OTHERS THEN
NULL` blocks, as upstream) so a database an older storage-api migrated
matches the pin too. This follows the `auth.sso_sessions` precedent in
[`specs/auth/database.md`](../auth/database.md).

## Not served yet

`storage:sql-function:storage.can_insert_object()`
(`0015-add-can-insert-object-function.sql:1`) inserts into
`storage.objects` and then raises `PT200` `ROLLBACK` to undo the insert.
`storage.objects` is not installed (PR 5), so the function would fail with
`42P01` where upstream succeeds; it is not installed and carries no marker.

## Judge cases

Not written yet. They go in a separate `review/issue-58-cases` PR and
should call each function through SQL on the Megabase and reference
databases (for example `storage.extension('a/b.png')`,
`storage.allow_only_operation('object.upload')` with `storage.operation`
set, a 101-character bucket name, a non-service-role lifecycle update, and
`to_regprocedure` for the dropped functions).
