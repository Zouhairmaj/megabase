# Storage: Iceberg REST catalog (issue 60)

Status: draft
Unit ids (`coverage/units.json`):
- `storage:route:DELETE /storage/v1/iceberg/bucket/{bucketName}`
- `storage:route:DELETE /storage/v1/iceberg/v1/{prefix}/namespaces/{namespace}`
- `storage:route:DELETE /storage/v1/iceberg/v1/{prefix}/namespaces/{namespace}/tables/{table}`
- `storage:route:GET /storage/v1/iceberg/bucket`
- `storage:route:GET /storage/v1/iceberg/v1/config`
- `storage:route:GET /storage/v1/iceberg/v1/{prefix}/namespaces`
- `storage:route:GET /storage/v1/iceberg/v1/{prefix}/namespaces/{namespace}`
- `storage:route:GET /storage/v1/iceberg/v1/{prefix}/namespaces/{namespace}/tables`
- `storage:route:GET /storage/v1/iceberg/v1/{prefix}/namespaces/{namespace}/tables/{table}`
- `storage:route:HEAD /storage/v1/iceberg/v1/{prefix}/namespaces/{namespace}`
- `storage:route:HEAD /storage/v1/iceberg/v1/{prefix}/namespaces/{namespace}/tables/{table}`
- `storage:route:POST /storage/v1/iceberg/bucket`
- `storage:route:POST /storage/v1/iceberg/v1/{prefix}/namespaces`
- `storage:route:POST /storage/v1/iceberg/v1/{prefix}/namespaces/{namespace}/tables`
- `storage:route:POST /storage/v1/iceberg/v1/{prefix}/namespaces/{namespace}/tables/{table}`
Level: 2

A service-role client manages analytics buckets and talks to an Apache
Iceberg REST catalog (S3 Tables or a token-authenticated catalog) through
Storage. On the self-hosted stack the feature is off.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Routes registered under the `iceberg` prefix | `vendor/storage/src/app.ts:100` |
| No route is registered unless `icebergEnabled` or `isMultitenant` | `vendor/storage/src/http/routes/iceberg/index.ts:17`–`:22` |
| `ICEBERG_ENABLED` must be the string `true`; `MULTI_TENANT` likewise | `vendor/storage/src/config.ts:788` |
| Enabled: JWT with `enforceJwtRoles: [dbServiceRole]`, `requireTenantFeature('icebergCatalog')` (multitenant only) | `vendor/storage/src/http/routes/iceberg/index.ts:24`–`:31` |
| Enabled: error body `{"error":{"message","type","code"}}` | `vendor/storage/src/http/routes/iceberg/index.ts:41`–`:51` |
| Analytics buckets: `POST /bucket`, `GET /bucket`, `DELETE /bucket/:bucketName` | `vendor/storage/src/http/routes/iceberg/bucket.ts:73`, `:100`, `:54` |
| `GET /v1/config?warehouse=` (`FeatureNotEnabled` without a catalog client) | `vendor/storage/src/http/routes/iceberg/catalog.ts:24`–`:45` |
| Namespaces: create, list, head, load, drop | `vendor/storage/src/http/routes/iceberg/namespace.ts:92`, `:115`, `:139`, `:161`, `:188` |
| Tables: create, list, load, head, drop, commit | `vendor/storage/src/http/routes/iceberg/table.ts:355`, `:384`, `:417`, `:450`, `:491`, `:540` |

Pins: storage `v1.80.2`, supabase `v1.26.08` (`vendor.toml`). The
self-hosted compose file (`vendor/supabase/docker/docker-compose.yml`) and
`judge/compose.override.yml` set neither `ICEBERG_ENABLED` nor
`MULTI_TENANT`, so the reference stack registers no `/iceberg` route.

## Inputs

Any method on `/storage/v1/iceberg`, `/storage/v1/iceberg/` or any path
below it, with any headers and query string. No JWT is checked: the auth
hooks live inside the unregistered plugin.

## Outputs

Fastify's default not-found handler (`basic404` in fastify
`lib/fourOhFour.js`, fastify `^5.11.2` per `vendor/storage/package.json`).
Status 404, `Content-Type: application/json; charset=utf-8`:

```json
{"message":"Route <METHOD>:<url> not found","error":"Not Found","statusCode":404}
```

`<url>` is `request.raw.url`: the path and query string as storage-api
receives them, after Kong (`strip_path: true` on `/storage/v1/`) removes
`/storage/v1`. For example `GET /storage/v1/iceberg/v1/config?warehouse=w`
gives `Route GET:/iceberg/v1/config?warehouse=w not found`. `statusCode`
is a number here, unlike storage-api's `errorSchema` bodies.

Megabase has no `ICEBERG_ENABLED` or `MULTI_TENANT` setting, so it always
answers as storage-api does with both unset. The database is not used.

## Edge cases

- `HEAD` gets the same status and headers; the body is dropped.
- Methods without an upstream route (`PUT`, `PATCH`, …) get the same 404.
- Fastify parses the body before the not-found handler for `POST`, `PUT`
  and `PATCH` when the request has a `Content-Type`, a `Transfer-Encoding`,
  or a non-zero `Content-Length`, and for `DELETE` and `OPTIONS` only when
  a `Content-Type` comes with a `Transfer-Encoding` or `Content-Length`
  (fastify `lib/handleRequest.js`). A parse failure (for example an empty
  `application/json` body, or an unknown media type) goes to the root
  error handler instead.

## Out of scope (501)

- Requests that Fastify body-parses (the path above).
- The enabled catalog: analytics bucket storage (`storage.buckets_analytics`),
  the S3 Tables / REST catalog client with SigV4 or token auth, the
  namespace and table limits, and the nested error format. It needs a
  remote catalog and an AWS SigV4 signer, which Megabase does not have.
