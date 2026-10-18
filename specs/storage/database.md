# Storage: database (issue 58)

Status: draft
Level: 2

Issue #58 ports the SQL objects storage-api's tenant migrations create in
the `storage` schema. RLS policies, clients and storage-api itself call
them inside PostgreSQL. This file covers the units Megabase installs.
Later PRs add the units listed under "Still not served".

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
definitions. The `storage.install_roles` role setup in
`0002-storage-schema.sql` is not ported (issue #493 creates
`supabase_storage_admin`). `sql/function_owners.sql` runs
`ALTER ... OWNER TO` that role only when it already exists, and otherwise
leaves the `DATABASE_URL` role as owner. Functions keep PostgreSQL's
default `EXECUTE` for `PUBLIC`: the tenant migrations never `GRANT` or
`REVOKE` `EXECUTE`.

The storage service role is read as upstream does:
`COALESCE(current_setting('storage.service_role', true), 'service_role')`.
Multipart table grants do the same for `storage.anon_role` and
`storage.authenticated_role`.

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
`storage.operation()` is installed first.

## `storage:sql-function:storage.operation()`

Upstream: `migrations/tenant/0024-operation-function.sql:1`.
File: `sql/operation.sql`.

**Inputs.** None. Reads the transaction-local GUC `storage.operation`
(`current_setting(..., true)`), which storage-api sets per request
(`src/internal/database/postgres/scope.ts:21`).

**Output.** `text`, `LANGUAGE plpgsql`, `STABLE`. The GUC value, or `NULL`
when it is unset.

**Errors.** None. `missing_ok` keeps an unset GUC from raising `42704`.

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
from `storage.protect_bucket_control_columns()` run first, so an update or
insert they reject (`0A000`, `22023`, `42501`) never reaches this trigger.
The lifecycle `CHECK` constraints are still part of the unclaimed
`storage.buckets` unit, so a shape they would reject (`23514`) is stored
when the `BEFORE` trigger allows the row.

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
`sql/dropped_prefix_functions.sql` first drops the triggers that depend on
them (`objects_*_prefix` on `storage.objects`, `0050:803-806`;
`prefixes_*_hierarchy` on `storage.prefixes`, `0052:5-6`; each guarded on the
table existing), then runs the upstream `DROP FUNCTION IF
EXISTS` statements (the `0052` ones inside `EXCEPTION WHEN OTHERS THEN
NULL` blocks, as upstream) so a database an older storage-api migrated
matches the pin too. This follows the `auth.sso_sessions` precedent in
[`specs/auth/database.md`](../auth/database.md).

## `storage:sql-function:storage.can_insert_object()`

Upstream: `migrations/tenant/0015-add-can-insert-object-function.sql:1`.
File: `sql/can_insert_object.sql`. The signature is not replaced later.

**Inputs.** `bucketid text`, `name text`, `owner uuid`, `metadata jsonb`.

**Output.** `void`, `LANGUAGE plpgsql` (volatile). Inserts one row into
`storage.objects` (`bucket_id`, `name`, `owner`, `metadata`), then raises
SQLSTATE `PT200` with message `ROLLBACK` and detail
`rollback successful insert`. The raise aborts the statement, so the
inserted row is not committed. RLS policies call it to test whether an
upload would succeed.

**Errors.** `23503` when `bucketid` is not a `storage.buckets` id. `23505`
when a current object already uses that name (the unique indexes on
`storage.objects`). `PT200` after an insert that those constraints accept.

## `storage:sql-function:storage.get_size_by_bucket()`

Upstream: created `migrations/tenant/0005-add-size-functions.sql:2`,
replaced by `0006`, `0036` and `0060`, final body
`0070-list-objects-with-versions.sql:1633`. File: `sql/get_size_by_bucket.sql`.
The zero-argument function is dropped first.

**Inputs.** `noncurrent_versions text` default `'include'`,
`delete_markers text` default `'include'`. `NULL` or any value other than
`exclude`, `only` or `include` is treated as `include` (`COALESCE` runs
before the allow-list, because `NULL NOT IN (...)` is not true).

**Output.** `TABLE(size bigint, bucket_id text)`, `LANGUAGE plpgsql`,
`STABLE`. One row per bucket: `sum((metadata->>'size')::bigint)`.
`noncurrent_versions = 'exclude'` keeps `archived_at IS NULL`; `'only'`
keeps `archived_at IS NOT NULL`. `delete_markers` filters
`is_delete_marker` the same way.

**Errors.** `22P02` when a counted object's `metadata->>'size'` is not an
integer.

## `storage:sql-function:storage.protect_bucket_control_columns()`

Upstream: `migrations/tenant/0068-bucket-lifecycle-configuration.sql:61`,
triggers at `:144`. File: `sql/protect_bucket_control_columns.sql`.
`LANGUAGE plpgsql`, `SET search_path = pg_catalog`.

**Triggers.** `protect_bucket_control_insert` `BEFORE INSERT` on
`storage.buckets`, argument `TG_ARGV[0]` = the storage service role at
install time. `protect_bucket_control_update` `BEFORE UPDATE OF
lifecycle_configuration, lifecycle_configuration_generation`, with no
argument. Megabase uses `CREATE OR REPLACE TRIGGER`.

**Output.** `NEW`.

**Errors.**

| When | SQLSTATE | Message |
|---|---|---|
| `INSERT` sets either lifecycle column and `current_user` is not a member of `TG_ARGV[0]` | `42501` | `only members of the configured storage service role may insert lifecycle policy state` |
| `UPDATE` changes a lifecycle column on a bucket whose `type` is not `STANDARD` | `0A000` | `bucket versioning and lifecycle controls require a Standard bucket` |
| `UPDATE` changes one lifecycle column without a new non-null generation for both | `22023` | `a changed lifecycle policy requires a new non-null generation` |

Membership uses `pg_has_role(..., 'MEMBER')`, unlike
`storage.enforce_bucket_lifecycle_service_role()`, which compares
`current_user` to the role name. Clearing both lifecycle columns on a
`STANDARD` bucket passes. An `UPDATE` that changes neither column passes.
The lifecycle `CHECK` constraints from the same migration are not
installed (they belong to `storage.buckets`).

## `storage:sql-table:storage.s3_multipart_uploads` and `storage.s3_multipart_uploads_parts`

Upstream: `0021-s3-multipart-uploads.sql:1` and `:12`, column types from
`0022-s3-multipart-uploads-big-ints.sql`, `user_metadata` from
`0025-custom-metadata.sql:2`, `metadata` from
`0057-s3-multipart-uploads-metadata.sql:1`. File: `sql/s3_multipart.sql`.

**`storage.s3_multipart_uploads`.** `id text` primary key,
`in_progress_size bigint NOT NULL DEFAULT 0`, `upload_signature text NOT
NULL`, `bucket_id text NOT NULL` referencing `storage.buckets(id)`,
`key text COLLATE "C" NOT NULL`, `version text NOT NULL`, `owner_id text`,
`created_at timestamptz NOT NULL DEFAULT now()`, `user_metadata jsonb`,
`metadata jsonb`. Index `idx_multipart_uploads_list` on
`(bucket_id, key COLLATE "C", created_at)`.

**`storage.s3_multipart_uploads_parts`.** `id uuid` primary key default
`gen_random_uuid()`, `upload_id text NOT NULL` referencing
`storage.s3_multipart_uploads(id) ON DELETE CASCADE`, `size bigint NOT NULL
DEFAULT 0`, `part_number int NOT NULL`, `bucket_id text NOT NULL`
referencing `storage.buckets(id)`, `key text COLLATE "C" NOT NULL`,
`etag text NOT NULL`, `owner_id text`, `version text NOT NULL`,
`created_at timestamptz NOT NULL DEFAULT now()`.

RLS is enabled with no policies. Grants follow 0021: `REVOKE ALL` from the
anon and authenticated roles, `GRANT ALL` to the service role, `GRANT
SELECT` to anon and authenticated. An existing table gains the later
columns and the `bigint` sizes through `ALTER`.

## `storage:sql-function:storage.list_multipart_uploads_with_delimiter()`

Upstream: created `0021-s3-multipart-uploads.sql:28`, final body
`0070-list-objects-with-versions.sql:20`. File:
`sql/list_multipart_uploads_with_delimiter.sql`. The six-argument function
is dropped first. `LANGUAGE sql`, `STABLE`, so the uploads table must
exist at `CREATE`.

**Inputs.** `bucket_id text`, `prefix_param text`, `delimiter_param text`,
`max_keys integer` default `100`, `next_key_token text` default `''`,
`next_upload_token text` default `''`, `raw_prefix_param text` default
`NULL`. The prefix used to split common prefixes is
`coalesce(raw_prefix_param, prefix_param)`.

**Output.** `TABLE(key text, id text, created_at timestamptz)`. Rows whose
key has `delimiter_param` after the prefix collapse to one common-prefix
row (the earliest `created_at`, `id`). An empty `next_key_token` returns
every match. Otherwise the result starts after that key, and when
`next_upload_token` is set, after that upload id within the same key.

**Errors.** None of its own. A missing uploads table fails at `CREATE`.

## `storage:sql-function:storage.list_objects_with_delimiter()`

Upstream: created `0020-list-objects-with-delimiter.sql:3`, replaced by
`0050-search-v2-optimised.sql:90`, final body
`0070-list-objects-with-versions.sql:114`. File:
`sql/list_objects_with_delimiter.sql`. The six-argument and seven-argument
overloads are dropped first. `LANGUAGE plpgsql`, `STABLE`.

**Inputs.** `_bucket_id text`, `prefix_param text`, `delimiter_param text`,
`max_keys integer` default `100`, `start_after text` default `''`,
`next_token text` default `''`, `sort_order text` default `'asc'`,
`noncurrent_versions text` default `'exclude'`, `delete_markers text`
default `'exclude'`, `next_token_archived_at timestamptz` default `NULL`,
`next_token_version text` default `''`. A non-empty `next_token` wins over
`start_after`. `sort_order` other than `asc` (case-insensitive) sorts
descending.

**Output.** `TABLE(name text, id uuid, metadata jsonb, updated_at
timestamptz, created_at timestamptz, last_accessed_at timestamptz, version
text, archived_at timestamptz, is_delete_marker boolean, is_versioned
boolean)`. Common prefixes come from `storage.get_common_prefix`. At most
`max_keys` rows. `noncurrent_versions` and `delete_markers` accept
`exclude`, `include` and `only` and otherwise act as `exclude`.

**Errors.** When a seek position does not advance, raises
`storage.list_objects_with_delimiter made no progress at seek (<name>, <archived_at>, <version>)`.

## `storage:sql-function:storage.search_by_timestamp()`

Upstream: created `0050-search-v2-optimised.sql:676`, validation added by
`0064-fix-search-by-timestamp-sqli.sql:5`, final body
`0070-list-objects-with-versions.sql:1347`. File:
`sql/search_by_timestamp.sql`. The eight-argument function is dropped
first. `LANGUAGE plpgsql`, `STABLE`.

**Inputs.** `p_prefix text`, `p_bucket_id text`, `p_limit integer`,
`p_level integer`, `p_start_after text`, `p_sort_order text`,
`p_sort_column text`, `p_sort_column_after text`, `noncurrent_versions
text` default `'exclude'`, `delete_markers text` default `'exclude'`,
`p_start_after_version text` default `''`. `p_sort_order` other than
`asc` or `desc` becomes `asc`. `p_sort_column` other than `updated_at` or
`created_at` becomes `updated_at`. Version filters follow
`get_size_by_bucket`, except the default here is `exclude`.

**Output.** `TABLE(key text, name text, id uuid, updated_at timestamptz,
created_at timestamptz, last_accessed_at timestamptz, metadata jsonb,
version text, archived_at timestamptz, is_delete_marker boolean,
is_versioned boolean)`. `key` is the common prefix from
`storage.get_common_prefix(name, prefix, '/')` when there is one, else the
object name. `LIKE` metacharacters in the prefix are escaped before they
are interpolated.

**Errors.** None of its own. The sort column and order are allow-listed
before they are interpolated into dynamic SQL (`0064`).

## `storage:sql-function:storage.search_v2()`

Upstream: created `0027-search-v2.sql:2`, sort arguments added by
`0039-add-search-v2-sort-support.sql:2`, replaced by
`0050-search-v2-optimised.sql:598`, final body
`0070-list-objects-with-versions.sql:1540`. File: `sql/search_v2.sql`.
The five-argument and eight-argument overloads are dropped first.
`LANGUAGE plpgsql`, `STABLE`.

**Inputs.** `prefix text`, `bucket_name text`, `limits integer` default
`100`, `levels integer` default `1`, `start_after text` default `''`,
`sort_order text` default `'asc'`, `sort_column text` default `'name'`,
`sort_column_after text` default `''`, `noncurrent_versions text` default
`'exclude'`, `delete_markers text` default `'exclude'`,
`start_after_archived_at timestamptz` default `NULL`,
`start_after_version text` default `''`, `start_after_is_continuation
boolean` default `false`. `limits` is capped at `1500` (`NULL` becomes
`100`). `sort_order` other than `asc` or `desc` becomes `asc`.
`sort_column` other than `name`, `updated_at` or `created_at` becomes
`name`.

**Output.** The same columns as `storage.search_by_timestamp()`. A `name`
sort calls `storage.list_objects_with_delimiter` and sets `key` to
`split_part(name, '/', levels)`. Any other sort calls
`storage.search_by_timestamp`. `start_after_is_continuation` passes
`start_after` as `next_token` instead of `start_after`.

**Errors.** Those of the function it calls.

## Still not served

These units are absent from the install and carry no marker. Several were
dropped before the pin (`0052`, `0059`); a later PR serves them by
dropping them, the way `sql/dropped_prefix_functions.sql` does for
`storage.add_prefixes()`. `storage.prefixes` is dropped by `0052:21`.

- `storage:sql-function:storage.get_level()`
- `storage:sql-function:storage.get_prefix()`
- `storage:sql-function:storage.get_prefixes()`
- `storage:sql-function:storage.lock_top_prefixes()`
- `storage:sql-function:storage.objects_delete_cleanup()`
- `storage:sql-function:storage.objects_insert_prefix_trigger()`
- `storage:sql-function:storage.objects_update_cleanup()`
- `storage:sql-function:storage.objects_update_level_trigger()`
- `storage:sql-function:storage.objects_update_prefix_trigger()`
- `storage:sql-function:storage.prefixes_delete_cleanup()`
- `storage:sql-function:storage.prefixes_insert_trigger()`
- `storage:sql-function:storage.search_legacy_v1()`
- `storage:sql-function:storage.search_v1_optimised()`
- `storage:sql-table:storage.buckets`
- `storage:sql-table:storage.buckets_analytics`
- `storage:sql-table:storage.buckets_vectors`
- `storage:sql-table:storage.iceberg_namespaces`
- `storage:sql-table:storage.iceberg_tables`
- `storage:sql-table:storage.migrations`
- `storage:sql-table:storage.prefixes`
- `storage:sql-table:storage.vector_indexes`

## `storage:sql-table:storage.objects`

File: `sql/objects.sql`. Final shape after the tenant migrations (see the
file header for the list): columns `id uuid` (`gen_random_uuid()`, primary
key), `bucket_id text` (FK `objects_bucketId_fkey` to `storage.buckets`),
`name`, `owner uuid`, `created_at`, `updated_at`, `last_accessed_at`
(`now()`), `metadata jsonb`, `path_tokens text[]` (generated from `name`),
`version`, `owner_id`, `user_metadata jsonb`, `archived_at`,
`is_delete_marker`, `is_versioned` (`false`). `level` is dropped by 0052,
`bucketid_objname` by 0072. Indexes `name_prefix_search`,
`idx_objects_bucket_id_name_lower`, `idx_objects_current_version`,
`idx_objects_null_version`, `idx_objects_delete_markers` and
`objects_bucket_id_name_version_key` (built without `CONCURRENTLY`, since the
install is one transaction). RLS is enabled with no policies; grants as
0046 and 0073. Trigger `update_objects_updated_at` runs
`storage.update_updated_at_column()`. An existing older table gains the
later columns through `ADD COLUMN IF NOT EXISTS`.

## `storage:sql-function:storage.protect_delete()`

Upstream: `0055-prevent-direct-deletes.sql:2`. Statement-level `BEFORE
DELETE` triggers `protect_buckets_delete` and `protect_objects_delete`
raise `42501` `Direct deletion from storage tables is not allowed. Use the
Storage API instead.` unless `storage.allow_delete_query` is `true` (the
request scope sets it).

## `storage:sql-function:storage.foldername()` and `storage.filename()`

Upstream: `0060-optimize-existing-functions-again.sql:22` and
`0061-mark-filename-immutable.sql:7`. File: `sql/path_functions.sql`.
`IMMUTABLE` plpgsql: `foldername('u1/a/b.png')` is `{u1,a}`,
`filename('u1/a/b.png')` is `b.png`.

## `storage:sql-function:storage.search()` and `storage.get_common_prefix()`

Upstream: `0070-list-objects-with-versions.sql:763` and `:89`. File:
`sql/search.sql`, copied from the final definitions; the eight-argument
`storage.search` of 0050–0063 is dropped first. Behavior and the route that
calls it are in [`object-list.md`](object-list.md).

## Judge cases

Not written yet. They go in a separate `review/issue-58-cases` PR and
should call each function through SQL on the Megabase and reference
databases (for example `storage.extension('a/b.png')`,
`storage.allow_only_operation('object.upload')` with `storage.operation`
set, a 101-character bucket name, a non-service-role lifecycle update,
`storage.can_insert_object` raising `PT200`, `storage.get_size_by_bucket()`,
`storage.search_v2` for a name sort and a timestamp sort, and
`to_regprocedure` for the dropped functions).
