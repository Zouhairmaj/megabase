# Storage: CDN cache purge (issue 57)

Status: draft
Unit ids (`coverage/units.json`):
- `storage:route:DELETE /storage/v1/cdn`
- `storage:route:DELETE /storage/v1/cdn/{bucketName}`
- `storage:route:DELETE /storage/v1/cdn/{bucketName}/{*}`
Level: 2

A service-role client asks Storage to purge the CDN cache for the whole
tenant, one bucket or one object, optionally only the image
transformations.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Routes registered under the `cdn` prefix | `vendor/storage/src/app.ts:98` |
| JWT auth with `enforceJwtRoles: [dbServiceRole]` | `vendor/storage/src/http/routes/cdn/index.ts:13`–`:15` |
| `requireTenantFeature('purgeCache')` is a no-op in single tenant | `vendor/storage/src/http/routes/cdn/index.ts:16`, `vendor/storage/src/http/plugins/tenant-feature.ts:24` |
| `DELETE /` (tenant), `/:bucketName` (bucket), `/:bucketName/*` (object) | `vendor/storage/src/http/routes/cdn/purgeCache.ts:57`, `:82`, `:110` |
| Params, query and header schemas | `vendor/storage/src/http/routes/cdn/purgeCache.ts:8`–`:31`, `vendor/storage/src/http/schemas/auth.ts:1` |
| Success body `{"message":"success"}` | `vendor/storage/src/http/routes/cdn/purgeCache.ts:77`, `:105`, `:135` |
| No `CDN_PURGE_ENDPOINT_URL`: `MissingParameter` | `vendor/storage/src/storage/cdn/cdn-cache-manager.ts:88`–`:91` |
| Role check: `AccessDenied` `Access denied: Invalid role`, status 403 | `vendor/storage/src/http/plugins/jwt.ts:85`–`:103` |
| Error rendering (`userStatusCode`, `errorSchema`) | `vendor/storage/src/http/error-handler.ts:69`–`:85`, `vendor/storage/src/internal/errors/codes.ts:169`, `:223` |

Pins: storage `v1.80.2`, supabase `v1.26.08` (`vendor.toml`). The
self-hosted compose file sets no `CDN_PURGE_ENDPOINT_URL` and no
`MULTI_TENANT`.

## Inputs

- `Authorization: Bearer <JWT>` verified with `JWT_SECRET`.
- Path: none, `{bucketName}`, or `{bucketName}/{*}` with a non-empty
  object name. `/storage/v1/cdn/` is the tenant route.
- Query: `transformations=true|false` (optional; other keys are ignored).
  It selects the `*-transforms` purge type, which does not change the
  response below.

Fastify validation (query, params, headers) runs before the JWT hooks.

## Outputs

`Content-Type: application/json; charset=utf-8`.

- Service role: the purge throws before any request because the endpoint
  is unset. Status 400:
  `{"statusCode":"400","error":"MissingParameter","message":"Missing Required Parameter CDN_PURGE_ENDPOINT_URL is not set","code":"MissingParameter"}`
- Any other role (or no `role` claim) in a valid JWT: status 403:
  `{"statusCode":"403","error":"Unauthorized","message":"Access denied: Invalid role","code":"AccessDenied"}`

Megabase has no `CDN_PURGE_ENDPOINT_URL` setting, so it always answers as
storage-api does with the variable unset. The database is not used.

## Out of scope (501)

- A configured purge endpoint (the `POST <url>/purge` call and the
  `{"message":"success"}` 200).
- Validation failures: a missing `Authorization` header, a
  `transformations` value other than `true`/`false` or repeated, and an
  empty object name (`/storage/v1/cdn/{bucketName}/`).
- A missing or invalid JWT (`AccessDenied` carries the verifier's message).
- Multi-tenant `FeatureNotEnabled`.

## Judge cases

To add on a `review/issue-57-cases` branch (`judge/` is protected):
`storage.cdn.purge_tenant`, `storage.cdn.purge_bucket` and
`storage.cdn.purge_object` with the `service_role` key, and
`storage.cdn.purge_anon` with the `anon` key.
