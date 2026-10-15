# Storage: resumable uploads (TUS) (issue 63)

Status: draft. Blocked on a missing upstream (see [Blocker](#blocker)).
Unit ids (`coverage/units.json`), all `storage:route:` prefixed:

| Method | Unauthenticated scope (JWT) | Signed scope (`/sign`) |
|---|---|---|
| `POST` | `/storage/v1/upload/resumable`, `/storage/v1/upload/resumable/{*}` | `/storage/v1/upload/resumable/sign`, `/storage/v1/upload/resumable/sign/{*}` |
| `PUT` | `/storage/v1/upload/resumable/{*}` | `/storage/v1/upload/resumable/sign/{*}` |
| `PATCH` | `/storage/v1/upload/resumable/{*}` | `/storage/v1/upload/resumable/sign/{*}` |
| `HEAD` | `/storage/v1/upload/resumable/{*}` | `/storage/v1/upload/resumable/sign/{*}` |
| `DELETE` | `/storage/v1/upload/resumable/{*}` | `/storage/v1/upload/resumable/sign/{*}` |
| `OPTIONS` | `/storage/v1/upload/resumable`, `/storage/v1/upload/resumable/{*}` | `/storage/v1/upload/resumable/sign`, `/storage/v1/upload/resumable/sign/{*}` |

Level: 2

A client uploads a large object in chunks with the
[TUS 1.0.0](https://tus.io/protocols/resumable-upload) protocol: `POST`
creates an upload and returns its URL in `Location`, `PATCH` appends bytes
at `Upload-Offset`, `HEAD` reports the current offset, `DELETE` terminates
an unfinished upload, and `OPTIONS` advertises the server's capabilities.
When the last byte lands, storage-api writes the `storage.objects` row as
a normal upload would.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Routes mounted under `upload/resumable` | `vendor/storage/src/app.ts:93` |
| `tusPath` default `/upload/resumable` (`TUS_URL_PATH`), URL expiry 1 h, lock type `postgres` | `vendor/storage/src/config.ts:467`–`:480` |
| File backend store: `FileStore` at `<FILE_STORAGE_BACKEND_PATH>/<bucket>` | `vendor/storage/src/http/routes/tus/index.ts:63`–`:87` |
| `@tus/server` options: `disableTerminationForFinishedUploads`, Postgres locker, lifecycle hooks, `respectForwardedHeaders`, extra `allowedHeaders` | `vendor/storage/src/http/routes/tus/index.ts:104`–`:153` |
| `maxSize`: global `FILE_SIZE_LIMIT`, or the bucket's smaller `file_size_limit` once the upload id is known | `vendor/storage/src/http/routes/tus/index.ts:154`–`:178` |
| Four plugin scopes: JWT (`registerJwtAuth` + `db`), signed (`dbSuperUser`, prefix `/sign`), public `OPTIONS`, public signed `OPTIONS` | `vendor/storage/src/http/routes/tus/index.ts:203`–`:254` |
| Per-request context: `isUpsert` from `x-upsert: true`, `isSigned`, owner from the JWT `sub` | `vendor/storage/src/http/routes/tus/index.ts:256`–`:286` |
| `POST /`, `POST /*`, `PUT /*`, `PATCH /*`, `HEAD /*`, `DELETE /*` all call `tusServer.handle` | `vendor/storage/src/http/routes/tus/index.ts:304`–`:381` |
| `OPTIONS /`, `OPTIONS /*` call `tusServer.handle` without JWT auth | `vendor/storage/src/http/routes/tus/index.ts:400`–`:432` |
| JWT plugin: invalid or missing token is `AccessDenied` with the verifier's message | `vendor/storage/src/http/plugins/jwt.ts:36`–`:66` |
| Signed scope: `x-signature` header verified with the upload scope; missing is `InvalidSignature` `Missing x-signature header` | `vendor/storage/src/http/routes/tus/lifecycle.ts:63`–`:85` |
| `onIncomingRequest`: signed requests skip RLS; `OPTIONS` and `HEAD` skip it; `POST` parses `Upload-Metadata`; others load the stored upload; then `canUpload` | `vendor/storage/src/http/routes/tus/lifecycle.ts:90`–`:180` |
| `generateUrl`: `STORAGE_PUBLIC_URL` proto and host, `X-Forwarded-Prefix`, `/sign` suffix, id without tenant as base64url | `vendor/storage/src/http/routes/tus/lifecycle.ts:185`–`:230` |
| `getFileIdFromRequest`: last path segment, base64url-decoded, prefixed with the tenant id | `vendor/storage/src/http/routes/tus/lifecycle.ts:235`–`:246` |
| `namingFunction`: `<tenant>/<bucketName>/<objectName>/<uuid>`; no metadata is `MetadataRequired` | `vendor/storage/src/http/routes/tus/lifecycle.ts:254`–`:271` |
| `onCreate`: `cacheControl` digits become `max-age=N`, otherwise `no-cache`; bucket MIME allow-list; content encoding validation | `vendor/storage/src/http/routes/tus/lifecycle.ts:276`–`:312` |
| `onUploadFinish`: `completeUpload` (`uploadType: 'resumable'`) and `Tus-Complete: 1` | `vendor/storage/src/http/routes/tus/lifecycle.ts:317`–`:370` |
| `onResponseError`: renderable storage errors become a plain-text TUS error body with the storage status | `vendor/storage/src/http/routes/tus/lifecycle.ts:53`–`:61`, `:375`–`:387` |
| Upload id parsing and validation (`InvalidUploadId`, bucket name and key checks) | `vendor/storage/src/storage/protocols/tus/upload-id.ts:1`–`:105` |
| File store: metadata on create, `FILE_NOT_FOUND` on a missing file | `vendor/storage/src/storage/protocols/tus/file-store.ts` |
| Error codes `InvalidUploadId`, `MetadataRequired`, `TusError` | `vendor/storage/src/internal/errors/codes.ts:241`, `:288`, `:296` |
| Observed behavior (OPTIONS 204 with `Tus-Version: 1.0.0`, numeric `Tus-Max-Size`, `Tus-Extension` containing `creation`; create 201; delete 204; signed scope 400 `Missing x-signature header`; traversal key 400 `Invalid key`) | `vendor/storage/src/test/tus.test.ts:289`–`:298`, `:639`–`:730`, `:1117`–`:1150`, `:1201`–`:1235` |
| Gateway: Kong's `cors` plugin answers a preflight `OPTIONS` (with `Origin` and `Access-Control-Request-Method`) before storage-api | `vendor/supabase/docker/volumes/api/kong.yml:316`–`:341`, `specs/server/cors.md` |

Pins: storage `v1.80.2`, supabase `v1.26.08` (`vendor.toml`). The
self-hosted compose file sets `STORAGE_BACKEND=file`,
`FILE_SIZE_LIMIT=52428800`, `STORAGE_PUBLIC_URL`,
`REQUEST_ALLOW_X_FORWARDED_PATH=true` and `TENANT_ID`
(`vendor/supabase/docker/docker-compose.yml:378`–`:391`).

## Inputs

- JWT scope: `Authorization: Bearer <JWT>` verified with `JWT_SECRET`.
- Signed scope: `x-signature: <token>` from
  `POST /object/upload/sign/{bucket}/{*}`; no JWT.
- TUS headers: `Tus-Resumable`, `Upload-Length`, `Upload-Defer-Length`,
  `Upload-Offset`, `Upload-Metadata` (keys `bucketName`, `objectName`,
  `contentType`, `cacheControl`, `contentEncoding`, `metadata`),
  `Content-Type: application/offset+octet-stream`, `x-upsert`.
- Path: `{*}` is the base64url upload id without the tenant
  (`<bucket>/<objectName>/<version>`).

## Outputs

storage-api's own errors before `tusServer.handle` (JWT plugin) are the
JSON error bodies of `vendor/storage/src/http/error-handler.ts`. Every
response after that point, including errors raised in the lifecycle
hooks, is written by `@tus/server`: its status codes, its `Tus-*`,
`Upload-*`, `Location` and `Access-Control-*` headers, and its plain-text
error bodies.

## Blocker

The `@tus/server` 2.4.5, `@tus/file-store` 2.1.1 and `@tus/utils`
packages (`vendor/storage/package.json:87`–`:89`) are not in `vendor/`.
They decide the protocol responses for all 16 units: the `Tus-Resumable`
version check, the headers and status of each method, the `Upload-Offset`
conflict and missing-upload errors, and the exact `Tus-Extension`,
`Access-Control-Allow-Headers` and `Access-Control-Expose-Headers`
values. The storage-api tests above pin only part of that surface.
Porting the rest from memory would be guessing (GOAL.md §3 rule 5), so
every resumable route stays the structured 501 until the TUS server
source is available as a vendored, pinned upstream.

## Out of scope (501)

Everything in this file, until the blocker is lifted. S3 store and S3
locker (`STORAGE_BACKEND=s3`, `TUS_LOCK_TYPE=s3`) and multi-tenant
behavior stay out of scope after that.
