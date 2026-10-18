# Meta: policies (issue 84)

Status: complete
Unit ids (`coverage/units.json`): `meta:route:GET /pg/policies`,
`meta:route:GET /pg/policies/{id}`, `meta:route:POST /pg/policies`,
`meta:route:PATCH /pg/policies/{id}`, `meta:route:DELETE /pg/policies/{id}`
Level: 4

Studio lists, creates, edits and drops row-security policies through
Postgres Meta. The five routes sit behind the gateway's admin-only `/pg/`
route, the same key check as `specs/meta/health-version.md`.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Routes mounted at `/policies` | `vendor/postgres-meta/src/server/routes/index.ts:70` |
| `GET /` list, `GET /:id(\\d+)`, `POST /`, `PATCH /:id(\\d+)`, `DELETE /:id(\\d+)` | `vendor/postgres-meta/src/server/routes/policies.ts:7`–`:125` |
| `list`, `retrieve`, `create`, `update`, `remove` | `vendor/postgres-meta/src/lib/PostgresMetaPolicies.ts:14`–`:173` |
| `POLICIES_SQL` | `vendor/postgres-meta/src/lib/sql/policies.sql.ts:6`–`:55` |
| `filterByList` / `filterByValue` | `vendor/postgres-meta/src/lib/helpers.ts:16`–`:37` |
| System schemas excluded unless `include_system_schemas=true` | `vendor/postgres-meta/src/lib/constants.ts:1` |
| `ident` / `literal` (pg-format 1.0.4) | used by `PostgresMetaPolicies.ts` |
| `int8` parsed as a JSON number when it is a safe integer | `vendor/postgres-meta/src/lib/db.ts:6` |
| No connection string throws before the handler | `vendor/postgres-meta/src/server/routes/index.ts:46`–`:48` |
| Error handler sends `{ error: message }` at 500 | `vendor/postgres-meta/src/server/app.ts:21`–`:24` |
| Unknown path sends `{ error: "Not found" }` at 404 | `vendor/postgres-meta/src/server/app.ts:26`–`:29` |

Pin: postgres-meta `v0.100.0` (`vendor.toml`).

## Inputs

`apikey` must be the service-role key.

`GET /pg/policies` query string (Node `querystring`, last duplicate wins):

- `include_system_schemas` — only the exact string `true` includes system schemas.
- `included_schemas` — comma-separated list. When non-empty it wins over excludes.
- `excluded_schemas` — comma-separated list, appended to `information_schema`, `pg_catalog`, `pg_toast` unless system schemas are included.
- `limit`, `offset` — quoted with pg-format `literal` when the string is non-empty (`limit=0` is `limit '0'`).

`GET`, `PATCH` and `DELETE /pg/policies/:id` take a digit-only id. Postgres Meta
passes it through JavaScript `Number`, so `01` is `1`, `0` is falsy, and a
value past `Number.MAX_SAFE_INTEGER` is the same rounded decimal `String(Number(id))`
produces (`1e+21`, `Infinity`).

`POST` and `PATCH` bodies are JSON. `name` and `table` are required on create.
`schema` defaults to `public`, `action` to `PERMISSIVE`, `command` to `ALL`,
`roles` to `["public"]`. `definition` and `check` are optional SQL expressions
placed in `USING (...)` and `WITH CHECK (...)`. `action` and `command` are
interpolated as SQL keywords, the same way postgres-meta does. Identifiers go
through pg-format `ident`.

## Outputs

Success is HTTP 200, `Content-Type: application/json; charset=utf-8`.

A policy object, in column order:

`id`, `schema`, `table`, `table_id`, `name`, `action`, `roles`, `command`,
`definition`, `check`.

`id` and `table_id` are JSON numbers. `roles` is a JSON array (`public` when
`polroles` is `{0}`). `definition` and `check` are the text from
`pg_get_expr`, or null. List returns a JSON array of those objects. Create
and update return the row after the write. Delete returns the row as it was
before `DROP POLICY`.

## Errors

| When | Status | Body |
|---|---|---|
| No `apikey`, or not the service-role key | 401 / 403 | Kong, same as other `/pg/` routes |
| No database configured | 500 | `{"error":"failed to get upstream connection details"}` |
| Non-digit id, or a method the route does not register | 404 | `{"error":"Not found"}` |
| `GET` retrieve fails, including a missing id | 404 | `{"error":"<message>"}` |
| `POST` fails after the body is accepted | 400 | `{"error":"<PostgreSQL message>"}` |
| `PATCH` / `DELETE` and the message starts with `Cannot find` | 404 | `{"error":"Cannot find a policy with ID <id>"}` |
| `PATCH` / `DELETE` other query errors, including id `0` | 400 | `{"error":"<message>"}` |
| Empty `application/json` body | 500 | `{"error":"Body cannot be empty when content-type is set to 'application/json'"}` |
| JSON `null` body on create or update | 500 | `{"error":"Cannot destructure property 'name' of 'null' as it is null."}` |
| Missing `name` or `table` on create | 500 | `{"error":"SQL identifier cannot be null or undefined"}` |
| Unsupported content type | 500 | `{"error":"Unsupported Media Type: <header>"}` |
| Query fails before PostgreSQL returns an error | 500 / 400 / 404 | `{"error":"connection failed"}` on the route's status |

A missing database is the `onRequest` throw, formatted by `setErrorHandler`.
That body is `{"error":...}`. `GET /pg/config/version` still uses the envelope
documented in `specs/meta/health-version.md`.

## Edge cases

Each request opens one connection from the shared pool and closes it
afterwards, matching `pgMeta.end()`. `PATCH` applies `USING`, `WITH CHECK`,
`TO`, then `RENAME` on the original name, inside one transaction. Rename keeps
the policy oid, so the follow-up read uses the same id.

`included_schemas` is not trimmed. An empty `included_schemas=` filter is
`IN ('')`. Include wins over exclude.

`action`, `command`, `definition`, and `check` are interpolated into the
statement without parameter bindings. That is intentional: it is what
postgres-meta does, and the route accepts only the service-role key
(`docs/SECURE_DESIGN.md`).

Megabase uses the process `DATABASE_URL`. It does not read a per-request `pg`
header or `x-connection-encrypted`. Those headers are the same gap as
`GET /pg/health`.

## Out of scope

Every other `/pg` route stays 501. Judge cases for these five units belong in
a `review/` change; this spec does not add them.

## Tests

`crates/megabase-meta` checks the SQL text against pg-format, the id
`Number`/`String` rules, auth, 404s, body errors, and the status used when
the database refuses the connection. `MEGABASE_TEST_DATABASE_URL`, when set,
runs create, list, get, patch and delete against PostgreSQL.
