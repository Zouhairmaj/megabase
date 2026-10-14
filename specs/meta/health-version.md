# Meta: health and server version (issue 349)

Status: complete
Unit ids (`coverage/units.json`): `meta:route:GET /pg/health`,
`meta:route:GET /pg/config/version`
Level: 1

Studio polls Postgres Meta's health route and reads the server version to
decide which features to show. Both are behind the gateway's admin-only
`/pg/` route.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| `GET /health` returns `{ date: new Date() }` | `vendor/postgres-meta/src/server/app.ts:53` |
| `ConfigRoute` mounted at `/config` | `vendor/postgres-meta/src/server/routes/index.ts:65` |
| `GET /config/version`: first row of `VERSION_SQL`, 500 `{ error }` on failure | `vendor/postgres-meta/src/server/routes/config.ts:29`–`:45`, `vendor/postgres-meta/src/lib/PostgresMetaVersion.ts:11` |
| `VERSION_SQL` | `vendor/postgres-meta/src/lib/sql/version.sql.ts:1` |
| `int8` parsed as a JSON number when it is a safe integer | `vendor/postgres-meta/src/lib/db.ts:6` |
| `onRequest` hook: no connection string is a thrown error | `vendor/postgres-meta/src/server/routes/index.ts:26`–`:60` |
| Gateway `key-auth` and `acl` (`admin`) on `/pg/` | `vendor/supabase/docker/volumes/api/kong.yml:394`–`:410` |

Pins: postgres-meta `v0.100.0`, supabase `v1.26.08` (`vendor.toml`).

## Inputs

`GET /pg/health`, `GET /pg/config/version`. `apikey` must be the
service-role key (`specs/server/key-auth.md`).

## Outputs

- Health: 200 `{"date":"<UTC ISO-8601 with milliseconds>Z"}`.
- Version: 200 `{"version":<version()>,"version_number":<server_version_num>,"active_connections":<count(*) from pg_stat_activity>,"max_connections":<max_connections>}`.

Both use `Content-Type: application/json; charset=utf-8` (Fastify).

## Errors

| When | Status | Body |
|---|---|---|
| No `apikey` | 401 | Kong `No API key found in request` |
| `apikey` is not the service-role key | 403 | Kong `You cannot consume this service` |
| No database configured | 500 | `{"statusCode":500,"error":"Internal Server Error","message":"failed to get upstream connection details"}` |
| Query fails | 500 | `{"error":"<PostgreSQL message>"}` |

## Edge cases

`active_connections` counts every backend in the cluster at query time. The
reference stack and Megabase share one cluster in the judge, so the value is
expected to match but can differ by a connection that opens or closes
between the two requests.

## Out of scope

Every other `/pg` route stays 501, including `GET /pg/config` and `GET /pg/`.

## Judge cases

`judge/cases/meta.toml`: `meta.health`, `meta.config.version`.
