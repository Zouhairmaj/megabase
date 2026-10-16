# Storage: S3 Vectors routes (issue 65)

Status: draft
Unit ids (`coverage/units.json`), PR 1 of the epic:
- `storage:route:POST /storage/v1/vector/CreateIndex`
- `storage:route:POST /storage/v1/vector/CreateVectorBucket`
- `storage:route:POST /storage/v1/vector/DeleteIndex`
- `storage:route:POST /storage/v1/vector/DeleteVectorBucket`
- `storage:route:POST /storage/v1/vector/DeleteVectors`
- `storage:route:POST /storage/v1/vector/GetIndex`
- `storage:route:POST /storage/v1/vector/GetVectorBucket`
- `storage:route:POST /storage/v1/vector/GetVectors`
- `storage:route:POST /storage/v1/vector/ListIndexes`
- `storage:route:POST /storage/v1/vector/ListVectorBuckets`

PR 2 covers `ListVectors`, `PutVectors` and `QueryVectors`; the same handler
answers them, so it only adds their markers.
Level: 2

A service-role client manages vector buckets and indexes and puts, queries
and deletes vectors through Storage. On the self-hosted stack the feature is
off.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Routes registered under the `vector` prefix | `vendor/storage/src/app.ts:101` |
| Not-found handler installed, then no route unless `vectorEnabled` or `isMultitenant` | `vendor/storage/src/http/routes/vector/index.ts:31`–`:35` |
| `VECTOR_ENABLED` must be the string `true` | `vendor/storage/src/config.ts:809` |
| Not-found handler body | `vendor/storage/src/http/not-found-handler.ts:6`–`:19` |
| `errorSchema` (all four fields are strings) | `vendor/storage/src/http/schemas/error.ts:1`–`:11` |
| Enabled: SigV4 (`s3vectors`) or JWT, `enforceJwtRole` service role | `vendor/storage/src/http/routes/vector/index.ts:42`–`:54` |
| Handlers | `create-index.ts:58`, `create-bucket.ts:28`, `delete-index.ts:36`, `delete-bucket.ts:28`, `delete-vectors.ts:35`, `get-index.ts:36`, `get-bucket.ts:28`, `get-vectors.ts:38`, `list-indexes.ts:31`, `list-buckets.ts:29` under `vendor/storage/src/http/routes/vector/` |

Pins: storage `v1.80.2`, supabase `v1.26.08` (`vendor.toml`).
`vendor/supabase/docker/docker-compose.yml` and `judge/compose.override.yml`
set neither `VECTOR_ENABLED` nor `MULTI_TENANT`, so the reference stack
registers no `/vector` route.

## Inputs

Any method on `/storage/v1/vector`, `/storage/v1/vector/` or any path below
it, with any headers and query string. No JWT or signature is checked: the
auth hooks live inside the unregistered plugin.

## Outputs

Status 404, `Content-Type: application/json; charset=utf-8`, serialized with
`errorSchema`:

```json
{"statusCode":"404","error":"Not Found","message":"Route <METHOD>:<url> not found","code":"InvalidRequest"}
```

`<url>` is `request.raw.url` after Kong strips `/storage/v1`, query string
included. Unlike `/iceberg` (Fastify's default 404), `statusCode` is a
string and `code` is present.

Megabase has no `VECTOR_ENABLED` or `MULTI_TENANT` setting, so it always
answers as storage-api does with both unset. The database is not used.

## Edge cases

- `HEAD` gets the same status and headers; the body is dropped.
- Methods without an upstream route (`GET`, `PUT`, …) get the same 404.
- Fastify parses the body before the not-found handler under the same
  framing rules as `/iceberg` (see `specs/storage/iceberg.md`). The root
  catch-all parser (`app.ts:18`) accepts every media type but
  `application/json`, so those get the 404. A JSON body that is non-empty,
  well-formed and under 1 MiB also gets the 404.

## Out of scope (501)

A JSON body that is empty, malformed, over the limit, or has a `__proto__`
or `constructor` key: the root error handler answers it, and its body is not
ported. The enabled behavior of every route (`VECTOR_ENABLED=true`) is not
ported.
