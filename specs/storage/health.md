# Storage: healthcheck (issue 59)

Status: draft
Unit id (`coverage/units.json`): `storage:route:GET /storage/v1/health`
Level: 2

Clients and monitors probe Storage through the gateway to check that the
service can reach its database.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Route registered under the `health` prefix | `vendor/storage/src/app.ts:99` |
| JWT auth, superuser connection and storage plugins | `vendor/storage/src/http/routes/health/index.ts:5`–`:9` |
| Handler: `{healthy: true}`, or `{healthy: false}` when the probe throws | `vendor/storage/src/http/routes/health/healthcheck.ts:6`–`:25` |
| JWT verification; failure is `AccessDenied` | `vendor/storage/src/http/plugins/jwt.ts:36`–`:68` |
| `Storage.healthcheck()` runs as the superuser | `vendor/storage/src/storage/storage.ts:367` |
| Probe SQL `SELECT id from storage.buckets limit 1` | `vendor/storage/src/storage/database/pg.ts:101`, `:2037` |
| No gateway `key-auth` on Storage | `vendor/supabase/docker/volumes/api/kong.yml:308` |

Pins: storage `v1.80.2`, supabase `v1.26.08` (`vendor.toml`).

## Inputs

`Authorization: Bearer <JWT>` verified with `JWT_SECRET`. Any role is
accepted: the route registers JWT auth without `enforceJwtRoles`. The
trailing-slash form `/storage/v1/health/` is the same route.

## Outputs

Status 200, `Content-Type: application/json; charset=utf-8`.

- Probe succeeds (with or without a row): `{"healthy":true}`.
- Probe fails: `{"healthy":false}`. Upstream records the error on the
  request; Megabase logs it.

Megabase runs the probe on the pool as the connecting (superuser) role,
without `SET ROLE`, as `asSuperUser()` does.

## Edge cases

- An empty `storage.buckets` is healthy.
- A missing `storage.buckets` table makes the probe fail: healthy `false`,
  still 200.

## Out of scope (501)

- A missing or invalid JWT (storage-api's `AccessDenied` error body is not
  ported).
- No database configured (`DATABASE_URL` unset).
- The unscoped-query statement timeout (`DATABASE_HEALTHCHECK_UNSCOPED`).

## Judge cases

To add on a `review/issue-59-cases` branch (`judge/` is protected):
`storage.health` — `GET /storage/v1/health` with the `anon` key.
