# Storage: move and copy objects (issue 455)

Status: draft
Level: 2
Unit ids (`coverage/units.json`):
- `storage:route:POST /storage/v1/object/move`
- `storage:route:POST /storage/v1/object/copy`

supabase-js `storage.from(b).move(from, to, { destinationBucket })` and
`copy(from, to, { destinationBucket })`. Request scope, file layout
(`<root>/<bucket>/<name>/<version>`), MD5 ETags and the database error
mapping are described in [`object.md`](object.md) and not repeated.

Pins: storage `v1.80.2`, supabase `v1.26.08` (`vendor.toml`).

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Move body schema, `destinationBucket \|\| bucketId`, response schema `{message}` (fast-json-stringify drops `Id` and `Key`) | `vendor/storage/src/http/routes/object/moveObject.ts:7`–`:80` |
| Copy body schema, `x-metadata`, `x-upsert`, `copyMetadata` default `true`, response `{Id, Key, ...objectSchema}` | `vendor/storage/src/http/routes/object/copyObject.ts:9`–`:94` |
| `ObjectStorage.copyObject`: `mustBeValidKey(destinationKey)`, source read under RLS, metadata merge, `canUpload`, backend copy, superuser upsert, `ObjectAdminDelete` of a replaced version | `vendor/storage/src/storage/object.ts:311`–`:470` |
| `ObjectStorage.moveObject`: `mustBeValidKey`, `testPermission` (find + rename under RLS, rolled back), superuser source read, same-key shortcut, backend copy, superuser rename, `ObjectAdminDelete` of the old version | `vendor/storage/src/storage/object.ts:478`–`:630` |
| `canUpload`: `createObject` (or `upsertObject` with `x-upsert`) under RLS, rolled back; a duplicate is `KeyAlreadyExists` | `vendor/storage/src/storage/uploader.ts:76`–`:102`, `vendor/storage/src/storage/database/pg.ts:1209`–`:1245` |
| `updateObject` (`NoSuchKey` when nothing matches), `upsertObject`, `findObject`; `undefined` columns are skipped | `vendor/storage/src/storage/database/pg.ts:1155`, `:1093`, `:1362`, `:2361`–`:2425` |
| File backend `copyObject` and `headObject` (`cacheControl` default `no-cache`, `mimetype` default `application/octet-stream`) | `vendor/storage/src/storage/backend/file.ts:265`–`:323`, `:385`–`:405`, `:673`–`:681` |
| `objectSchema` | `vendor/storage/src/storage/schemas/object.ts:4` |
| Operations `storage.object.move`, `storage.object.copy` | `vendor/storage/src/http/routes/operations.ts:17`, `:33` |

## Inputs

- `Content-Type: application/json` body, Ajv with `coerceTypes: false`:
  `bucketId`, `sourceKey`, `destinationKey` (required strings),
  `destinationBucket` (string; empty means `bucketId`). Copy also takes
  `metadata` (object with optional string `cacheControl`,
  `contentEncoding`, `mimetype`) and `copyMetadata` (boolean).
- Any verified JWT; queries run in the request scope, so RLS on
  `storage.objects` applies to the checks described below.
- Copy: `x-upsert: true` lets the destination be replaced.

## Outputs

- Move: 200 `{"message":"Successfully moved"}`. The row keeps its `id`
  and gets the destination `bucket_id` and `name`, a new `version`,
  `owner` (when `sub` is a UUID) and `owner_id` (when there is a `sub`),
  the source `user_metadata`, and `headObject` metadata of the copied
  file (`httpStatusCode`, `size`, `cacheControl`, `contentEncoding` when
  the source had one, `mimetype`, `eTag`, `lastModified`,
  `contentLength`). The old file is removed. Moving an object onto its own
  key changes nothing and answers the same message.
- Copy: 200 `{"Id": <row id>, "Key": "<destinationBucket>/<destinationKey>",`
  then the destination row through `objectSchema` (`name`, `bucket_id`,
  `owner`, `owner_id`, `version`, `id`, timestamps, `metadata`,
  `user_metadata`, `archived_at`, `is_delete_marker`, `is_versioned`)`}`.
  The metadata is the source's, with `cacheControl` and `mimetype`
  replaced from `metadata` when `copyMetadata` is `false`, plus the new
  file's `lastModified` and `eTag`. `user_metadata` is copied when
  `copyMetadata` is `true`. A replaced destination's old file is removed.

## Errors

All are sent with HTTP 400.

| When | Body |
|---|---|
| `destinationKey` empty or outside `[A-Za-z0-9_/!.*'() &$=@;:+,?-]` | `{"statusCode":"400","code":"InvalidKey","error":"InvalidKey","message":"Invalid key: <key>"}` |
| Source missing or hidden by RLS (select, or for move the update `USING`) | `{"statusCode":"404","code":"NoSuchKey","error":"not_found","message":"Object not found"}` |
| Move onto an existing key | `{"statusCode":"409","code":"ResourceAlreadyExists","error":"Duplicate","message":"The resource already exists"}` |
| Copy onto an existing key without `x-upsert` | `{"statusCode":"409","code":"KeyAlreadyExists","error":"Duplicate","message":"The resource already exists"}` |
| RLS `WITH CHECK` refuses the renamed row or the destination insert | `{"statusCode":"403","code":"AccessDenied","error":"Unauthorized","message":"new row violates row-level security policy"}` |

## Edge cases

- Megabase keeps no extended attributes on files; the cache control,
  encoding and MIME type the file backend reads from them come from the
  source row, where Megabase records them on upload.
- Upstream takes an advisory lock (`waitObjectLock`) before the superuser
  write; Megabase relies on the `FOR UPDATE` row lock alone.
- Hardening over upstream, which renames as the superuser: after the row lock
  Megabase checks the row still has the version it copied (else `NoSuchKey`),
  then applies the request scope in the same transaction and runs the rename
  under RLS, so a change after the permission probe cannot widen what the
  caller may rename.
- The replaced and old files are removed after the commit; a removal
  failure is logged, as upstream's queued `ObjectAdminDelete`.

## Out of scope (501)

- Missing or invalid JWTs, no database or storage root (as `object.md`).
- Bodies Fastify rejects (another content type, malformed JSON, schema
  failures, coercible values).
- `sourceVersionId` (object versions), `x-metadata` (`parseUserMetadata`)
  and `metadata.contentEncoding` with `copyMetadata: false`
  (`validateContentEncoding`).
- A missing destination bucket (`23503`, `RelatedResourceNotFound`) and
  other database errors; a source row without a version or with a key
  Megabase could not store; a failed file copy.
- The S3 backend: Megabase stores objects on the file backend only.

## Judge cases

Not yet written; they go in a separate `review/issue-455-cases` PR:
move then download the new key and 404 the old one, move across buckets
with `destinationBucket`, copy with and without `x-upsert`, copy onto an
existing key (`KeyAlreadyExists`), and a move of a missing object
(`NoSuchKey`).
