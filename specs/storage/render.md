# Storage: image render (issue 62)

Status: draft
Unit ids (`coverage/units.json`):
- `storage:route:GET /storage/v1/render/image/authenticated/{bucketName}/{*}`
- `storage:route:GET /storage/v1/render/image/public/{bucketName}/{*}`
- `storage:route:GET /storage/v1/render/image/sign/{bucketName}/{*}`
Level: 2

A client asks Storage for an object image with transformations (size,
resize mode, format, quality, gravity). storage-api looks the object up,
then streams it through imgproxy.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Routes registered under the `render/image` prefix | `vendor/storage/src/app.ts:96` |
| Routes exist only when `ENABLE_IMAGE_TRANSFORMATION` is true (the self-hosted compose sets it, with imgproxy) | `vendor/storage/src/http/routes/render/index.ts:15`, `vendor/storage/src/config.ts:738` |
| `authenticated`: JWT auth and the request-role `db`; `sign`/`public`: `dbSuperUser`. `requireTenantFeature` is a no-op in single tenant; the rate limiter is off by default | `vendor/storage/src/http/routes/render/index.ts:19`–`:43`, `vendor/storage/src/http/plugins/tenant-feature.ts:24`, `vendor/storage/src/config.ts:770` |
| Query schema: transformation options plus `download`, `versionId` | `vendor/storage/src/http/routes/render/renderAuthenticatedImage.ts:21`, `vendor/storage/src/http/schemas/transformations.ts:1` |
| `authenticated` handler: `findObject` then render | `vendor/storage/src/http/routes/render/renderAuthenticatedImage.ts:37`–`:84` |
| `public` handler: `findBucket(isPublic)` and `findObject` in `Promise.all`, then render | `vendor/storage/src/http/routes/render/renderPublicImage.ts:37`–`:87` |
| `sign` handler: `token` required, `verifyObjectSignature` with the download scope, `findObject` on the token `url` | `vendor/storage/src/http/routes/render/renderSignedImage.ts:22`–`:105` |
| `verifyObjectSignature`: `InvalidJWT`, scope, `url`, `exp` checks | `vendor/storage/src/storage/object.ts:979`–`:1013` |
| `isDownloadScopedToken` | `vendor/storage/src/internal/auth/jwt.ts:95` |
| `verifyJWT` (jose `jwtVerify`, no clock tolerance) | `vendor/storage/src/internal/auth/jwt.ts:487` |
| `findObject`: `mustBeValidKey`, then `NoSuchKey` | `vendor/storage/src/storage/object.ts:278`–`:287`, `vendor/storage/src/storage/database/pg.ts:1362`–`:1403` |
| `findBucketById` with `isPublic`: `NoSuchBucket` | `vendor/storage/src/storage/database/pg.ts:501`–`:542` |
| `VALID_OBJECT_KEY` | `vendor/storage/src/storage/limits.ts:88`–`:99` |
| Error rendering (`userStatusCode`, `error` defaults to the code) | `vendor/storage/src/http/error-handler.ts:69`–`:85`, `vendor/storage/src/internal/errors/storage-error.ts:23`, `vendor/storage/src/internal/errors/codes.ts:134`, `:159`, `:256`, `:401` |

Pins: storage `v1.80.2`, supabase `v1.26.08` (`vendor.toml`).

## Inputs

- Path: `{bucketName}` and a non-empty object name `{*}` (may contain `/`),
  both percent-decoded.
- `authenticated`/`public` query: `height`, `width` (integer ≥ 0),
  `resize` (`cover|contain|fill`), `format` (`origin|avif|webp`),
  `quality` (integer 20–100), `gravity` (`no so ea we noea nowe soea sowe
  ce sm fp`), `x_offset`, `y_offset` (finite numbers; with `gravity=fp`
  both are required and in 0–1), `download`, `versionId`. Other keys are
  ignored. A repeated key fails validation.
- `sign` query: `token` (required), `download`.
- `authenticated`: `Authorization: Bearer <JWT>` verified with `JWT_SECRET`.

Fastify validation runs before the JWT hook and the handler.

## Outputs

Errors are `Content-Type: application/json; charset=utf-8`, status 400,
body `{"statusCode","code","error","message"}`:

| Case | `statusCode` | `code` | `error` | `message` |
|---|---|---|---|---|
| Object key fails `VALID_OBJECT_KEY` | `400` | `InvalidKey` | `InvalidKey` | `Invalid key: <key>` |
| No current object after the bucket is public, or on `authenticated` / `sign` (including no row for `versionId`) | `404` | `NoSuchKey` | `not_found` | `Object not found` |
| `public`: bucket missing or not public | `404` | `NoSuchBucket` | `Bucket not found` | `Bucket not found` |
| `sign`: token not download-scoped | `400` | `InvalidSignature` | `InvalidSignature` | `Token is not scoped for download` |
| `sign`: token `url` ≠ `<bucketName>/<*>` | `400` | `InvalidSignature` | `InvalidSignature` | `Invalid signature` |

Order: `public` checks the key, then the bucket, then the object row.
Upstream starts `findBucket(id, 'id,public', { isPublic: true })` and
`findObject` together with `Promise.all`
(`vendor/storage/src/http/routes/render/renderPublicImage.ts:57`). The
`isPublic` filter throws `NoSuchBucket` when the bucket is missing or not
public. `findObject` throws `NoSuchKey` when the row is missing. When the
row exists, only the bucket lookup rejects, so the response is
`NoSuchBucket`. When both reject, storage-api reports whichever query
settles first. Megabase reads the bucket first and answers `NoSuchBucket`
for a missing or non-public bucket, which is one of those outcomes, and
does not answer 501. It reads the object row only when the bucket is
public. `sign` checks scope, then `url`, then the key of the object named
by `url`. `authenticated` checks the key after JWT auth.

`findObject` selects from `storage.objects` with `name COLLATE "C" = $1
AND bucket_id = $2` plus `version = $3` or, after migration
`object-versioning-core`, `archived_at IS NULL`.

## Out of scope (501)

- Rendering a found object (no object backend or imgproxy in Megabase).
- Query validation failures (Ajv messages), and coercions Megabase does
  not reproduce (exponents, empty values).
- `authenticated` with a missing or invalid JWT (`AccessDenied` with the
  verifier's message) or a role other than `service_role` (RLS request
  scope is not ported).
- `sign` tokens jose rejects (`InvalidJWT` with jose's message): bad
  signature, `exp <= now`, any `nbf`, a non-numeric `iat`. A `versionId`
  claim that is not a string.
- Any database error, including a database without `storage.objects`
  (Megabase installs only `storage.buckets`).
- Multi-tenant `FeatureNotEnabled` and rate limiting (`SlowDown`).

## Judge cases

To add on a `review/issue-62-cases` branch (`judge/` is protected):
`storage.render.sign_wrong_url` (token signed for another object),
`storage.render.public_invalid_key` (object name with `%23`), and
`storage.render.authenticated_invalid_key` with the `service_role` key.
