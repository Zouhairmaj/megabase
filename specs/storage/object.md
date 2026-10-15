# Storage: objects (issue 380)

Status: draft
Level: 2
Unit ids (`coverage/units.json`):
- `storage:route:POST /storage/v1/object/{bucketName}/{*}`
- `storage:route:PUT /storage/v1/object/{bucketName}/{*}`
- `storage:route:GET /storage/v1/object/authenticated/{bucketName}/{*}`
- `storage:route:GET /storage/v1/object/{bucketName}/{*}`
- `storage:route:GET /storage/v1/object/public/{bucketName}/{*}`
- `storage:route:POST /storage/v1/object/sign/{bucketName}/{*}`
- `storage:route:GET /storage/v1/object/sign/{bucketName}/{*}`
- `storage:route:DELETE /storage/v1/object/{bucketName}`
- `storage:route:DELETE /storage/v1/object/{bucketName}/{*}` (issue 61)
- `storage:route:HEAD /storage/v1/object/authenticated/{bucketName}/{*}` (issue 61)
- `storage:route:GET /storage/v1/object/info/authenticated/{bucketName}/{*}` (issue 61)
- `storage:route:GET /storage/v1/object/info/{bucketName}/{*}` (issue 61)
- `storage:route:GET /storage/v1/object/info/public/{bucketName}/{*}` (issue 61)

supabase-js `storage.from(b).upload/update/download/remove/createSignedUrl/getPublicUrl/info/exists`.
The table these routes write is in [`database.md`](database.md).

Pins: storage `v1.80.2`, supabase `v1.26.08` (`vendor.toml`).

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Route registration: JWT + request `db` for upload, download, sign, delete; `dbSuperUser` for public and signed downloads | `vendor/storage/src/http/routes/object/index.ts:24` |
| JWT plugin; `allowInvalidJwt` falls back to `{role: anon}` | `vendor/storage/src/http/plugins/jwt.ts:36` |
| Request scope: `role`, `request.jwt.claim.role`, `request.jwt`, `request.jwt.claim.sub`, `request.jwt.claims`, `request.headers`, `request.method`, `request.path`, `storage.operation`, `storage.allow_delete_query` | `vendor/storage/src/internal/database/postgres/scope.ts:14` |
| Operation names | `vendor/storage/src/http/routes/operations.ts:17` |
| Upload: `x-upsert`, `PUT` always upserts, response `{Id, Key}` | `vendor/storage/src/http/routes/object/createObject.ts:65`, `updateObject.ts:65` |
| Bucket lookup as superuser, then file checks | `vendor/storage/src/storage/object.ts:92` |
| Binary body: `content-type` default `application/octet-stream`, `cache-control` default `no-cache`, MIME check, `Content-Length` over the limit | `vendor/storage/src/storage/uploader.ts:380`, `:470` |
| Multipart body: first file part, fields before it, `contentType` field over the part type, `cacheControl` field as `max-age=`, MIME check, truncation is `EntityTooLarge` after `canUpload` | `vendor/storage/src/storage/uploader.ts:161`, `:432` |
| `mustBeValidKey`, then `canUpload` (insert or upsert as the request role, rolled back), then `completeUpload` as superuser | `vendor/storage/src/storage/object.ts:125`, `vendor/storage/src/storage/uploader.ts:62`, `:184` |
| `createObject` / `upsertObject` SQL and `ON CONFLICT (bucket_id, name COLLATE "C") WHERE archived_at IS NULL` | `vendor/storage/src/storage/database/pg.ts:1093`, `:1209` |
| PG error mapping (`42501` AccessDenied, `23505` duplicate) | `vendor/storage/src/storage/database/errors.ts:16` |
| File backend: bytes at `<bucket>/<key>/<version>`, `md5` ETag in quotes | `vendor/storage/src/storage/backend/file.ts:194`, `:780` |
| Download handlers | `vendor/storage/src/http/routes/object/getObject.ts:36`, `getPublicObject.ts:52`, `getSignedObject.ts:58` |
| Response headers | `vendor/storage/src/storage/renderer/renderer.ts:123` |
| Sign: body, `signObjectUrl`, token payload `{url, scope: "download", iat, exp}` | `vendor/storage/src/http/routes/object/getSignedURL.ts:19`, `vendor/storage/src/storage/object.ts:825`, `vendor/storage/src/internal/auth/jwt.ts:509` |
| `verifyObjectSignature` | `vendor/storage/src/storage/object.ts:979` |
| Delete: body, `deleteObjects` SQL, `objectSchema` response | `vendor/storage/src/http/routes/object/deleteObjects.ts:16`, `vendor/storage/src/storage/object.ts:193`, `vendor/storage/src/storage/database/pg.ts:1276`, `vendor/storage/src/storage/schemas/object.ts:4` |
| Single delete: superuser `findObject ... FOR UPDATE`, `deleteObject` as the request role, `AccessDenied` when nothing was deleted, `{"message":"Successfully deleted"}` | `vendor/storage/src/http/routes/object/deleteObject.ts:42`, `vendor/storage/src/storage/object.ts:144`, `vendor/storage/src/storage/database/pg.ts:1247` |
| Object info: bucket, auth and lookup order; operations per route | `vendor/storage/src/http/routes/object/getObjectInfo.ts:37`, `:96`, `:140` |
| `head` and `info` renderers: metadata parsing, headers, JSON body | `vendor/storage/src/storage/renderer/head.ts:11`, `:60`, `vendor/storage/src/storage/renderer/info.ts:11`, `vendor/storage/src/storage/renderer/renderer.ts:123` |

## Configuration

Object bytes live under `FILE_STORAGE_BACKEND_PATH` (the self-hosted
compose sets `/var/lib/storage`). Without it, or without `DATABASE_URL`,
every object route is a 501. Without `JWT_SECRET`, the routes that need a
verified JWT (upload, authenticated download and info, sign, signed
download, delete) are a 501; public downloads and info, and
`GET /object/:bucket/*` and `GET /object/info/:bucket/*` on a public bucket
(anonymous fallback) still work.

`list` is served by [`object-list.md`](object-list.md). The unported
prefixes `list-v2`, `sign/:bucket` (sign many) and `upload/sign` are
explicit 501s, so they never reach the generic `:bucket/*` handlers.

## Request scope

Every query runs in a transaction that first runs `scope.ts`'s
`set_config` statement. Request routes use the JWT `role` (default
`anon`), the raw token, `sub`, the claims JSON, the request headers as a
JSON object, the method, the path after `/storage/v1` with its query, and
the route's operation. Superuser queries use `service_role` and the claims
`{"role":"service_role"}` (storage-api uses its service key's payload).

## Upload — `POST` / `PUT /object/:bucketName/*`

Order: JWT, bucket lookup (`NoSuchBucket`), MIME check
(`InvalidMimeType`, skipped for an empty bucket list and for names ending
`.emptyFolderPlaceholder`), `Content-Length` above
`min(bucket.file_size_limit, 52428800)` (0 for an empty-folder name) is
`EntityTooLarge`, `InvalidKey`, `canUpload` as the request role, write the
file, then as superuser lock the current row (`FOR UPDATE`); without upsert
an existing row is `KeyAlreadyExists`. The row stores `owner` (the `sub`
when it is a UUID), `owner_id` (`sub`), `version` (a new UUID) and
`metadata` `{eTag, size, mimetype, cacheControl, lastModified,
contentLength, httpStatusCode: 200}`. The replaced version's file is
removed. Response `200 {"Id": <row id>, "Key": "<bucket>/<name>"}`.

A `multipart/form-data` body (supabase-js sends one for a `Blob`, `File`
or `FormData`) uploads its first file part (a part whose
`Content-Disposition` has a `filename`; supabase-js names it `""`). Only
fields before that part count, and the first of each name wins. The
mimetype is the `contentType` field when non-empty, else the part's
lowercased `type/subtype`; `cacheControl` is `max-age=<cacheControl
field>`, else `no-cache` (the `Cache-Control` header is ignored). The MIME
check runs as for a binary body, but the size limit applies to the file
bytes and only after `canUpload` (busboy truncates the stream; the request
`Content-Length` counts the form overhead). `x-upsert` and `PUT` behave as
for a binary body.

| Case | `statusCode` | `code` | `error` | `message` |
|---|---|---|---|---|
| RLS rejects the insert | `403` | `AccessDenied` | `Unauthorized` | `new row violates row-level security policy` |
| Name exists, no upsert (`canUpload` or `completeUpload`) | `409` | `KeyAlreadyExists` | `Duplicate` | `The resource already exists` |

## Download

- `GET /object/authenticated/:bucket/*`: valid JWT required.
- `GET /object/:bucket/*`: a missing or invalid JWT is `anon` and only a
  public bucket is readable; otherwise as `authenticated`.
- Both: bucket missing (or private when unauthenticated) is `NoSuchBucket`;
  then `InvalidKey`; a public bucket reads the row as superuser, a private
  one under RLS; no row is `NoSuchKey`.
- `GET /object/public/:bucket/*`: `InvalidKey` first, then bucket (public
  only) and row as superuser. Row found but bucket missing or private is
  `NoSuchBucket`; bucket public and no row is `NoSuchKey`; both missing is a
  501 (storage-api races the two lookups).
- `GET /object/sign/:bucket/*?token=`: jose verification (`InvalidJWT`
  `"exp" claim timestamp check failed` or `signature verification failed`),
  scope (`InvalidSignature` `Token is not scoped for download`), `url` equal
  to `<bucket>/<*>` (`InvalidSignature` `Invalid signature`), `InvalidKey`
  on the token object, then the row as superuser.

200 headers: `Accept-Ranges: bytes`, `Content-Type` (the stored mimetype;
`text/html` becomes `text/plain`), `ETag`, `X-Robots-Tag: none`,
`Last-Modified` (file mtime, `toUTCString`), `Content-Length`, and
`Cache-Control` (the stored value). Signed downloads send `Expires` (the
token `exp`) and `Cache-Control: no-transform` only when the stored value
has that directive. `download=` adds `Content-Disposition: attachment;`;
`download=<name>` adds `attachment; filename=<name>; filename*=UTF-8''<name>`.
A row whose file is missing answers status 400, `Cache-Control: no-store`,
`{"statusCode":"404","code":"NoSuchKey","error":"Not found","message":"The resource was not found"}`.

## Sign — `POST /object/sign/:bucket/*`

Body `{"expiresIn": n}`. Then JWT, `InvalidKey`, the row under RLS
(`NoSuchKey`). The token is HS256 with `JWT_SECRET`, payload
`{url: "<bucket>/<name>", scope: "download", iat, exp: iat + n}`. Response
`200 {"signedURL": "/object/sign/<bucket>/<name>?token=<jwt>"}`.

## Remove — `DELETE /object/:bucket`

Body `{"prefixes": ["<name>", ...]}` (1 to 500 names). Deletes the current
rows the request role may delete (`DELETE ... RETURNING *` under RLS), then
their files. Response `200` with the deleted rows in `objectSchema` order:
`name, bucket_id, owner, owner_id, version` (a `null` is `""`), `id`,
`updated_at, created_at, last_accessed_at` (ISO with milliseconds or
`null`), `metadata, user_metadata`, `archived_at`, `is_delete_marker`,
`is_versioned`. Names that match nothing are left out; an empty result is
`[]`.

## Remove one — `DELETE /object/:bucket/*`

JWT required. In one transaction: the current row as superuser
`FOR UPDATE` (none is `NoSuchKey`), then `DELETE` of the current row as the
request role; when RLS leaves nothing to delete the answer is
`AccessDenied` with message `Access denied` (status 400, `statusCode`
`403`) and the transaction rolls back. After the commit the version's file
is removed. Response `200 {"message":"Successfully deleted"}`. Unlike the
other object routes there is no `InvalidKey` check.

## Object info — `HEAD /object/authenticated/:bucket/*`, `GET /object/info/...`

| Route | JWT | Operation | Renderer |
|---|---|---|---|
| `HEAD /object/authenticated/:bucket/*` | required | `object.head_authenticated_info` | `head` |
| `GET /object/info/authenticated/:bucket/*` | required | `object.get_authenticated_info` | `info` |
| `GET /object/info/:bucket/*` | `allowInvalidJwt` (`anon`) | `object.get_authenticated_info` | `info` |
| `GET /object/info/public/:bucket/*` | none (superuser) | `storage.object.info_public` | `info` |

Order: bucket as superuser; missing, or private without a JWT (always on
the public route), is `NoSuchBucket`. Then `InvalidKey`; the row is read as
superuser for a public bucket or the public route, under RLS otherwise; no
row is `NoSuchKey`. Neither renderer reads the file.

Headers come from the row's `metadata` (`createAssetMetadata`: numbers are
safe integers or digit strings, `lastModified` goes through `new Date`):
`ETag`, `Last-Modified` (`toUTCString`), `Cache-Control` (the stored value
when non-empty). `head` adds `Accept-Ranges: bytes`, `Content-Type` (the
stored mimetype, `text/html` becomes `text/plain`), `X-Robots-Tag: none`,
`Content-Encoding` when stored, and `Content-Length` (the stored
`contentLength`), with no body. `info` answers
`Content-Type: application/json; charset=utf-8` and
`{id, name, version, bucket_id, size, content_type, cache_control,
content_encoding, etag, metadata (user_metadata), last_modified
(updated_at), created_at, archived_at, is_delete_marker, is_versioned}`;
missing metadata values are `null`, timestamps are ISO with milliseconds.

## Out of scope (501)

- A missing or invalid JWT where a JWT is required (`AccessDenied` with
  the verifier's message).
- `Content-Encoding`, `x-robots-tag`, `x-metadata` on a raw upload (a
  form ignores it), an upload without `Content-Length` or with an empty
  file.
- Multipart forms that are malformed or have no file part
  (`NoContentProvided`), `contentEncoding`, `metadata` or `userMetadata`
  fields, a repeated `contentType` field, a file part without its own
  `Content-Type`.
- A `Content-Type` that starts with `multipart/form-data` in other letter
  case. `uploader.ts:433` tests a case-sensitive `startsWith`, so it would
  take the binary path; what Fastify's multipart parser does with that body
  first is not verified.
- `versionId`, `Range` and conditional request headers, `download` names
  other than `[A-Za-z0-9._-]*`, objects whose metadata has `xRobotsTag` or
  `contentEncoding`.
- Sign bodies other than `{"expiresIn": 1..315360000}` (`transform`,
  `versionId`, Fastify validation messages); paths with percent escapes.
- Delete entries with `versionId`, more than 500 names, validation messages.
- Other jose failures on signed URLs; database errors other than `42501`
  and `23505`; object keys with `.`/`..`/empty segments.
- Single delete with `versionId`.
- Object info with any query string (`versionId`, transformation options
  and `X-Transformations`), with `If-None-Match` (revalidation
  directives), or for a row whose metadata has `contentRange`, an
  `httpStatusCode` other than 200, no `eTag`, or a `lastModified` that is
  not RFC 3339 or epoch milliseconds; `HEAD` also needs a stored
  `mimetype` and `contentLength`. Fastify's automatic `HEAD` on the
  `/object/info` routes.
- `HEAD /object/public/:bucket/*` and `HEAD /object/:bucket/*` (the same
  handler; issue 61 PR 2), `/object/list-v2`, copy, move,
  `POST /object/sign/:bucket` (many) and signed uploads.

## Judge cases

To add on a `review/issue-380-cases` branch (`judge/` is protected), each
creating its own public or private bucket with `{{run}}`: upload then
upsert (`x-upsert`), duplicate upload (`KeyAlreadyExists`), authenticated
and public download, sign then fetch the signed URL, a signed URL for a
different object (`Invalid signature`), remove, and an upload with the
`anon` key to a bucket without policies (RLS `AccessDenied`). Multipart
(issue 398, on `review/issue-398-cases`): a small PNG as a supabase-js
`Blob` form, a form with `cacheControl`, and a form upsert
(`x-upsert: true`) over an existing object, each downloaded to compare the
bytes, `Content-Type` and `Cache-Control`. Object info and single delete
(issue 61, on `review/issue-61-cases`): `info()` and `exists()` on a
private and a public bucket, info for a missing object (`NoSuchKey`) and
bucket (`NoSuchBucket`), anonymous info on a private bucket, single
`DELETE` then a download (`NoSuchKey`), and a single `DELETE` with the
`anon` key under RLS (`AccessDenied`). Timestamps,
`Id`, `ETag`/`Last-Modified` and the token need the harness normalizer.
