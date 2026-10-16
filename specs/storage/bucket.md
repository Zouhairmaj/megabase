# Storage: create, update and lifecycle bucket routes (issue 56)

Status: draft
Unit ids (`coverage/units.json`): `storage:route:POST /storage/v1/bucket`,
`storage:route:PUT /storage/v1/bucket/{bucketId}`,
`storage:route:GET /storage/v1/bucket/{bucketId}/lifecycle`,
`storage:route:PUT /storage/v1/bucket/{bucketId}/lifecycle`,
`storage:route:DELETE /storage/v1/bucket/{bucketId}/lifecycle`
Level: 2

The list and get routes of the same epic are in
[`buckets.md`](buckets.md). Shared inputs (JWT, service role, response
content type, `SET LOCAL ROLE service_role` transactions) are described
there and not repeated.

`supabase-js` `storage.createBucket()` and `updateBucket()` call these routes.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| `POST /bucket` body schema and handler; `id` defaults to `name`, empty `allowed_mime_types` entries are dropped | `vendor/storage/src/http/routes/bucket/createBucket.ts:8`–`:84` |
| `PUT /bucket/:bucketId` body schema (`anyOf` one of `public`, `file_size_limit`, `allowed_mime_types`) and handler | `vendor/storage/src/http/routes/bucket/updateBucket.ts:8`–`:78` |
| `file_size_limit`: non-negative integer, `null`, or `^[0-9]+(?:\.[0-9]+)?(?:[gG][bB]\|[mM][bB]\|[kK][bB]\|[bB])$` | `vendor/storage/src/http/schemas/file-size-limit.ts:1` |
| `Storage.createBucket`: whitespace, name and reserved-suffix checks, `ANALYTICS` branch, size limit, MIME normalization | `vendor/storage/src/storage/storage.ts:119`–`:172` |
| `Storage.updateBucket`: name check, size limit, MIME normalization, CDN purge | `vendor/storage/src/storage/storage.ts:215`–`:255` |
| `parseMaxSizeLimit` against `FILE_SIZE_LIMIT` | `vendor/storage/src/storage/storage.ts:371` |
| `isValidBucketName`, reserved suffix, `parseFileSizeToBytes`, `UUID_PATTERN` | `vendor/storage/src/storage/limits.ts:89`–`:175` |
| `normalizeAllowedMimeTypes` | `vendor/storage/src/storage/validators/mime-type.ts:1`–`:46` |
| `createBucket` insert (`owner` only when `sub` is a UUID, `owner_id` = `sub`, `type` `STANDARD`), duplicate is `BucketAlreadyExists` | `vendor/storage/src/storage/database/pg.ts:451`–`:498` |
| `updateBucket` update of the given columns; no row is `NoSuchBucket` | `vendor/storage/src/storage/database/pg.ts:1051`–`:1091` |
| `23505` is `ResourceAlreadyExists` | `vendor/storage/src/storage/database/errors.ts:25` |
| Lifecycle routes: body validation, then `assertLifecycleApiEnabled` / `assertLifecycleWriteReady`; the delete accepts an empty JSON body | `vendor/storage/src/http/routes/bucket/lifecycle.ts:57`–`:131`, `vendor/storage/src/storage/lifecycle/guards.ts:1`, `vendor/storage/src/http/plugins/empty-json-body.ts:1` |
| `bucketLifecycleConfigurationSchema` | `vendor/storage/src/storage/schemas/lifecycle.ts:1`–`:47` |
| `STORAGE_LIFECYCLE_ENABLED` (off unless `true`), `FILE_SIZE_LIMIT`, `ICEBERG_BUCKET_DETECTION_SUFFIX` default `--table-s3` | `vendor/storage/src/config.ts:450`, `:496`, `:795` |
| Error bodies | `vendor/storage/src/internal/errors/codes.ts` (`FeatureNotEnabled` `:105`, `NoSuchBucket` `:134`, `InvalidMimeType` `:310`, `EntityTooLarge` `:342`, `InvalidBucketName` `:375`, `BucketAlreadyExists` `:420`) |
| Status 400 unless the error status is 500; `error` falls back to `code` | `vendor/storage/src/http/error-handler.ts:67`–`:84` |
| Self-hosted configuration: `FILE_SIZE_LIMIT: 52428800`, no `STORAGE_LIFECYCLE_ENABLED` | `vendor/supabase/docker/docker-compose.yml:364`–`:398` |

Pins: storage `v1.80.2`, supabase `v1.26.08` (`vendor.toml`).

## Inputs

As `buckets.md`, plus a JSON body (`Content-Type: application/json`) on
`POST /bucket`, `PUT /bucket/:bucketId` and `PUT /bucket/:bucketId/lifecycle`.

- Create: `name` (required string), `id`, `public`, `type`
  (`STANDARD` or `ANALYTICS`), `file_size_limit`, `allowed_mime_types`.
  Other keys are ignored. `lifecycle_configuration` is rejected by the schema.
- Update: at least one of `public`, `file_size_limit`, `allowed_mime_types`.
- Lifecycle put: `{"rules": [...]}`, 1 to 1000 rules, each with `status`
  (`Enabled`/`Disabled`), `filter` (object) and
  `noncurrentVersionExpiration.noncurrentDays` (integer).

## Outputs

- Create: 200 `{"name": <name>}`. The row has `id` = `id` or `name` when
  `id` is empty, `public` defaulting to `false`, `type` `STANDARD`,
  `owner_id` = JWT `sub` and `owner` = `sub` when it is a UUID.
  `file_size_limit` is stored in bytes (`"1.5MB"` → `1500000`).
  `allowed_mime_types` is stored lowercased as `type/subtype`, parameters
  removed, duplicates dropped; `null` stays `null`.
- Update: 200 `{"message":"Successfully updated"}`. Only the given columns
  change; `null` clears `file_size_limit` / `allowed_mime_types`.
- Lifecycle get, put and delete: the lifecycle API is disabled in the
  pinned self-hosted configuration, so all three answer the
  `FeatureNotEnabled` error. The delete takes no body, an empty
  `application/json` body, or any JSON value.

## Errors

All are sent with HTTP 400 and `Content-Type: application/json; charset=utf-8`.

| When | Body |
|---|---|
| Create name empty, over 100 characters, outside `[A-Za-z0-9_!.*'() &$=@;:+,?-]`, with surrounding whitespace, or ending in `--table-s3`; update path id invalid | `{"statusCode":"400","code":"InvalidBucketName","error":"Invalid Input","message":"Bucket name invalid"}` |
| `file_size_limit` over 52428800 bytes | `{"statusCode":"413","code":"EntityTooLarge","error":"Payload too large","message":"The object exceeded the maximum allowed size"}` |
| An `allowed_mime_types` entry that is not a media type (wildcard type, wildcard subtype other than `*`, bad parameter, over 1000 characters) | `{"statusCode":"415","code":"InvalidMimeType","error":"invalid_mime_type","message":"mime type <entry> is not supported"}` |
| Create with an existing `id` or `name` | `{"statusCode":"409","code":"BucketAlreadyExists","error":"Duplicate","message":"The resource already exists"}` |
| Update of an unknown bucket | `NoSuchBucket` (see `buckets.md`) |
| Lifecycle get and delete; lifecycle put with a valid body | `{"statusCode":"409","code":"FeatureNotEnabled","error":"FeatureNotEnabled","message":"The feature object lifecycles is not enabled for this resource"}` |

Checks run in upstream order: body schema, name, `ANALYTICS`, size limit,
MIME types, database.

## Edge cases

- The create `id` is not validated, as upstream.
- `Math.round` semantics for fractional sizes (`"0.5B"` → 1).
- `allowed_mime_types: []` and `[""]` store an empty array.
- The lifecycle get and delete do not look up the bucket: an unknown
  bucket gets the same `FeatureNotEnabled` error.

## Out of scope (501)

- Everything `buckets.md` lists (roles, JWT errors, database error mapping).
- Bodies Fastify rejects (wrong content type, malformed JSON, schema
  failures) and values Ajv would coerce (`"true"`, `"100"`, a scalar MIME
  type). Fastify's `FST_ERR_VALIDATION` envelope is not ported.
- `type: "ANALYTICS"` (Iceberg catalog buckets, `createIcebergBucket`).
- The enabled lifecycle API (`STORAGE_LIFECYCLE_ENABLED=true`):
  normalization, storage of `lifecycle_configuration`, and
  `NoSuchLifecycleConfiguration`.
- The CDN cache purge when a bucket turns private (no CDN in self-hosting).

## Judge cases

Not yet written; they go in a separate `review/issue-56-cases` PR.
