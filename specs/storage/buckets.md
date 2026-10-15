# Storage: list and get buckets (issue 349)

Status: draft
Unit ids (`coverage/units.json`): `storage:route:GET /storage/v1/bucket`,
`storage:route:GET /storage/v1/bucket/{bucketId}`
Level: 1

Dashboards and `supabase-js` (`storage.listBuckets()`, `getBucket()`) read
bucket metadata from `storage.buckets`.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| `GET /bucket`: selected columns, optional `limit`/`offset`/`sortColumn`/`sortOrder`/`search` | `vendor/storage/src/http/routes/bucket/getAllBuckets.ts:47`–`:80` |
| `listBuckets` SQL | `vendor/storage/src/storage/database/pg.ts:917` |
| `GET /bucket/:bucketId`: selected columns | `vendor/storage/src/http/routes/bucket/getBucket.ts:28`–`:46` |
| `findBucketById` SQL; no row is `NoSuchBucket` | `vendor/storage/src/storage/database/pg.ts:501`–`:538` |
| `NoSuchBucket`: `error` and `message` `Bucket not found`, `httpStatusCode` 404 | `vendor/storage/src/internal/errors/codes.ts:134` |
| Renderable errors are sent with status 400 unless the status code is 500 | `vendor/storage/src/http/error-handler.ts:67`–`:84` |
| Response body schema (fast-json-stringify) | `vendor/storage/src/storage/schemas/bucket.ts:3`, `vendor/storage/src/http/schemas/error.ts:1` |
| `storage.buckets` final shape, RLS and grants | `vendor/storage/migrations/tenant/0002`, `0007`, `0008`, `0012`–`0014`, `0018`, `0038`, `0044`, `0046`, `0062`, `0068`, `0073` |
| No gateway `key-auth` on Storage | `vendor/supabase/docker/volumes/api/kong.yml:308` |

Pins: storage `v1.80.2`, supabase `v1.26.08` (`vendor.toml`).

## Inputs

`Authorization: Bearer <JWT>` verified with `JWT_SECRET`. Only
`role: service_role` is served. `bucketId` is a path segment.

## Outputs

`Content-Type: application/json; charset=utf-8`.

- List: 200, array of `{id, name, owner, public, type, file_size_limit, allowed_mime_types, created_at, updated_at}`.
- Get: 200, the same object without `type`.

fast-json-stringify writes a `null` string as `""` and a `null` boolean as
`false`; dates are `Date.prototype.toISOString()`.

Megabase installs `storage` and `storage.buckets` (`crates/megabase-storage/sql/buckets.sql`)
at startup when `DATABASE_URL` is set, and runs each query in a
transaction under `SET LOCAL ROLE service_role`, as storage-api does.

## Errors

| When | Status | Body |
|---|---|---|
| Unknown `bucketId` | 400 | `{"statusCode":"404","code":"NoSuchBucket","error":"Bucket not found","message":"Bucket not found"}` |

## Edge cases

The list has no `ORDER BY` without `sortColumn`, as upstream.

## Out of scope (501)

- Any role other than `service_role`, and a missing or invalid JWT (RLS
  queries and storage-api's JWT error bodies are not ported).
- List query options (`limit`, `offset`, `sortColumn`, `sortOrder`, `search`).
- Database error mapping.
- The rest of the `storage` schema: triggers, the versioning and lifecycle
  constraints, `storage.objects` and the other tables. The
  `storage:sql-table:storage.buckets` unit carries no marker.

## Judge cases

`judge/cases/storage.toml`: `storage.bucket.list`, `storage.bucket.missing`.
