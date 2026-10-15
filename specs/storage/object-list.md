# Storage: list objects (issue 399)

Status: draft
Level: 2
Unit ids (`coverage/units.json`):
- `storage:route:POST /storage/v1/object/list/{bucketName}`
- `storage:sql-function:storage.search()`
- `storage:sql-function:storage.get_common_prefix()`

supabase-js `storage.from(b).list(path, { limit, offset, sortBy, search })`.
The request scope, JWT handling and `storage.objects` table are in
[`object.md`](object.md) and [`database.md`](database.md).

Pin: storage `v1.80.2` (`vendor.toml`).

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Route, body schema, operation `storage.object.list` | `vendor/storage/src/http/routes/object/listObjects.ts:10`, `:58` |
| `storage.from(bucket)`: `mustBeValidBucketName` | `vendor/storage/src/storage/storage.ts:45`, `vendor/storage/src/storage/limits.ts:130` |
| A non-empty prefix without a trailing `/` gains one | `vendor/storage/src/storage/object.ts:643` |
| `searchObjects`: arguments of `storage.search`, `escapeLike` for non-`name` sorts | `vendor/storage/src/storage/database/pg.ts:1630`, `:63` |
| `storage.search` and `storage.get_common_prefix` (final shape) | `vendor/storage/migrations/tenant/0070-list-objects-with-versions.sql:763`, `:89` |
| Response schema `objectListEntrySchema` | `vendor/storage/src/storage/schemas/object.ts:48` |

## Input

`POST /storage/v1/object/list/:bucketName` with a verified bearer JWT and a
JSON body:

| Key | Type | Default |
|---|---|---|
| `prefix` (required) | string | — |
| `limit` | integer ≥ 1 | 100 (the function caps it at 1500) |
| `offset` | integer ≥ 0 | 0 |
| `sortBy` | `{column: name\|updated_at\|created_at\|last_accessed_at, order?: asc\|desc}` | `name`, `asc` |
| `search` | string | `""` |
| `noncurrentVersions`, `deleteMarkers` | `exclude\|include\|only` | `exclude` |
| `exactMatch` | `false` | — |

Other keys are ignored (the schema has no `additionalProperties: false`).

## Behavior

1. An invalid bucket name (`isValidBucketName`) is `InvalidBucketName`
   (400, `statusCode` `"400"`).
2. `prefix` gets a trailing `/` unless it is empty or already has one.
3. For a sort column other than `name`, `prefix` and `search` are
   `escapeLike`d (`\`, `%`, `_` get a backslash).
4. `storage.search(prefix, bucket, limit, levels, offset, search, column,
   order, noncurrent_versions, delete_markers)` runs in the request scope,
   so RLS on `storage.objects` decides which rows the caller sees. `levels`
   is `(prefix + search).split('/').length`; the function ignores it.
5. The bucket is never looked up: an unknown bucket lists `[]` with 200.

## Output

200, `application/json; charset=utf-8`, an array in `storage.search` order.
Each entry has the keys of `objectListEntrySchema` that the query returns,
in schema order: `name`, `version`, `id`, `updated_at`, `created_at`,
`last_accessed_at`, `metadata`, `archived_at`, `is_delete_marker`,
`is_versioned`. `name` is relative to the prefix. A folder (a common prefix
up to the next `/`) has every other key `null`. Timestamps are
`toISOString()` (millisecond precision, `Z`).

## Out of scope (501)

- A missing or invalid JWT (`AccessDenied`), no database configured.
- Fastify validation messages and its type coercion: a body that is not a
  JSON object, a missing or non-string `prefix`, `limit`/`offset` that are
  not integers in range (or do not fit PostgreSQL `integer`), `null` values,
  an unknown `sortBy.column`/`order` or version filter.
- `exactMatch: true` (`SearchObjectsExactMatch`).
- Database errors other than `42501`.

## Judge cases

To add on a `review/issue-399-cases` branch (`judge/` is protected), each
in its own bucket named with `{{run}}`: list the root (folders and files),
a prefix with a nested folder, `limit`/`offset`, `sortBy` name `desc`, a
`search` filter, a private bucket listed with the `anon` key and no
policies (`[]` under RLS), and an unknown bucket (`[]`). Timestamps, `id`
and `metadata.eTag`/`lastModified` need the harness normalizer.
