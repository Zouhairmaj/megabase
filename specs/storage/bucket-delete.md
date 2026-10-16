# Storage: delete and empty bucket routes (issue 455)

Status: draft
Unit ids (`coverage/units.json`):
`storage:route:DELETE /storage/v1/bucket/{bucketId}`,
`storage:route:POST /storage/v1/bucket/{bucketId}/empty`
Level: 2

`supabase-js` `storage.deleteBucket(id)` and `storage.emptyBucket(id)` call
these routes. Shared inputs (JWT, service role, response content type) are
described in [`buckets.md`](buckets.md) and not repeated. A direct
`DELETE FROM storage.objects` or `storage.buckets` fails on the
`storage.protect_delete()` trigger unless `storage.allow_delete_query` is
`true`, which storage-api sets in every request scope. Megabase sets it in
both transactions below.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| `DELETE /bucket/:bucketId` handler, `{"message":"Successfully deleted"}` | `vendor/storage/src/http/routes/bucket/deleteBucket.ts:20`–`:48` |
| `POST /bucket/:bucketId/empty` handler and message | `vendor/storage/src/http/routes/bucket/emptyBucket.ts:28`–`:57` |
| `emptyBucket` is registered in the `serviceRoleOnly` group (`enforceJwtRoles`) | `vendor/storage/src/http/routes/bucket/index.ts:31`–`:39` |
| Both bodies may be empty under `application/json` | `vendor/storage/src/http/plugins/empty-json-body.ts:1` |
| `Storage.deleteBucket`: lock the bucket as the superuser (`NoSuchBucket`), count one object (`BucketNotEmpty`), delete as the request role (no row is `NoSuchBucket`), purge the CDN cache | `vendor/storage/src/storage/storage.ts:261`–`:285` |
| `Storage.emptyBucket`: `findBucket` (`NoSuchBucket`), at most `emptyBucketMax` objects (`UnableToEmptyBucket`), stop when no object predates the request, send `ObjectAdminDeleteAllBefore` | `vendor/storage/src/storage/storage.ts:340`–`:365` |
| `findBucketById`, `countObjectsInBucket`, `listObjects` (`created_at < before`, `ORDER BY name COLLATE "C"`), `deleteBucket`, `deleteObjects` by `id` with `archived_at IS NULL` | `vendor/storage/src/storage/database/pg.ts:501`, `:596`, `:635`, `:899`, `:1276` |
| `ObjectAdminDeleteAllBefore.handle`: batches of `MAX_OBJECTS_PER_DELETE_BATCH` (500), rows and backend files deleted in one transaction, the next batch sent again | `vendor/storage/src/storage/events/objects/object-admin-delete-all-before.ts:35`–`:125` |
| Without `PG_QUEUE_ENABLE`, `send` runs the handler inline | `vendor/storage/src/internal/queue/event.ts:248`–`:275`, `vendor/storage/src/config.ts:694` |
| `STORAGE_EMPTY_BUCKET_MAX` default 200000 | `vendor/storage/src/config.ts:495` |
| `BucketNotEmpty` (`ResourceNotEmpty`), `UnableToEmptyBucket` (`InvalidRequest`), `NoSuchBucket` | `vendor/storage/src/internal/errors/codes.ts:75`, `:127`, `:134` |
| Status 400 unless the error status is 500; `error` falls back to `code` | `vendor/storage/src/http/error-handler.ts:67`–`:84` |
| Self-hosted configuration: no `PG_QUEUE_ENABLE`, no `STORAGE_EMPTY_BUCKET_MAX`, file backend | `vendor/supabase/docker/docker-compose.yml:364`–`:398` |

Pins: storage `v1.80.2`, supabase `v1.26.08` (`vendor.toml`).

## Inputs

- Path `bucketId`.
- Body: none (no `Content-Type`), or `Content-Type: application/json` with
  an empty body or any JSON value. `supabase-js` sends `{}`.
- Service-role JWT. Empty also needs `FILE_STORAGE_BACKEND_PATH`.

## Outputs

- Delete: 200 `{"message":"Successfully deleted"}`; the row is gone.
- Empty: every object row created before the request and its file
  `<root>/<bucket>/<name>/<version>` are deleted, 500 rows per
  transaction, before the response. Then 200
  `{"message":"Empty bucket has been queued. Completion may take up to an hour."}`.
  The message is the pinned source's; on the self-hosted stack the work is
  done when it is sent. An already empty bucket gets the same answer.

## Errors

All are sent with HTTP 400 and `Content-Type: application/json; charset=utf-8`.

| When | Body |
|---|---|
| Unknown bucket (both routes) | `{"statusCode":"404","code":"NoSuchBucket","error":"Bucket not found","message":"Bucket not found"}` |
| Delete of a bucket with any object row (archived rows included) | `{"statusCode":"409","code":"ResourceNotEmpty","error":"ResourceNotEmpty","message":"The bucket you tried to delete is not empty"}` |
| Empty of a bucket with more than 200000 object rows | `{"statusCode":"409","code":"InvalidRequest","error":"InvalidRequest","message":"Unable to empty the bucket because it contains too many objects"}` |

## Edge cases

- Objects created after the request started are kept by empty.
- A file removal failure rolls back that batch's rows (as upstream, where
  the backend delete runs inside the transaction); Megabase then answers
  501 rather than port the error mapping.
- Megabase never writes archived rows. The empty loop lists only current
  rows so it stays finite; upstream lists archived rows too but never
  deletes them.
- The CDN purge after delete is a no-op: there is no CDN in self-hosting.

## Out of scope (501)

- Roles other than the service role, missing or invalid JWTs (as
  `buckets.md`). Upstream lets other roles delete under RLS on
  `storage.buckets`; that path is not ported.
- Bodies Fastify rejects: another content type, a body without a content
  type, malformed JSON.
- Database errors other than the ones above, such as a bucket still
  referenced by a pending multipart upload (`ResourceReferenced`).
- Analytics (Iceberg) buckets.

## Judge cases

Not yet written; they go in a separate `review/issue-455-cases` PR.
