# Meta: tables (issue 90)

Status: complete
Unit ids (`coverage/units.json`): `meta:route:GET /pg/tables`,
`meta:route:GET /pg/tables/{id}`, `meta:route:POST /pg/tables`,
`meta:route:PATCH /pg/tables/{id}`, `meta:route:DELETE /pg/tables/{id}`
Level: 4

Studio lists, creates, alters and drops tables through Postgres Meta. The
pinned gateway exposes those routes under `/pg/` and allows only the admin
group.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| `GET /tables` list | `vendor/postgres-meta/src/server/routes/tables.ts:16`–`:67` |
| `GET /tables/:id` retrieve | `vendor/postgres-meta/src/server/routes/tables.ts:70`–`:103` |
| `POST /tables` create | `vendor/postgres-meta/src/server/routes/tables.ts:106`–`:135` |
| `PATCH /tables/:id` update | `vendor/postgres-meta/src/server/routes/tables.ts:138`–`:176` |
| `DELETE /tables/:id` remove | `vendor/postgres-meta/src/server/routes/tables.ts:179`–`:220` |
| List, retrieve, create, update, remove | `vendor/postgres-meta/src/lib/PostgresMetaTables.ts:20`–`:246` |
| Table catalog SQL | `vendor/postgres-meta/src/lib/sql/table.sql.ts:4`–`:111` |
| Column catalog SQL (included by default) | `vendor/postgres-meta/src/lib/sql/columns.sql.ts:11`–`:130` |
| `filterByList`, `filterByValue`, `coalesceRowsToArray` | `vendor/postgres-meta/src/lib/helpers.ts:3`–`:37` |
| System schemas excluded unless asked | `vendor/postgres-meta/src/lib/constants.ts:1` |
| `ident` / `literal` | pg-format 1.0.4, required by postgres-meta `package.json` |
| Missing connection string thrown in `onRequest` | `vendor/postgres-meta/src/server/routes/index.ts:26`–`:60` |
| Query failure body `{ error: error.message }` | `vendor/postgres-meta/src/lib/db.ts:207`–`:214`, route handlers above |
| List status from `translateErrorToResponseCode` | `vendor/postgres-meta/src/server/utils.ts:48`–`:59` |
| Schemas | `vendor/postgres-meta/src/lib/types.ts:363`–`:409` |
| Gateway `key-auth` and `acl` (`admin`) on `/pg/` | `vendor/supabase/docker/volumes/api/kong.yml:394`–`:410` |

Pins: postgres-meta `v0.100.0`, supabase `v1.26.08` (`vendor.toml`).

## Inputs

`apikey` must be the service-role key (`specs/server/key-auth.md`).

`GET /pg/tables` query string (all optional):

| Parameter | Meaning |
|---|---|
| `include_system_schemas` | Boolean. Default false. False adds `information_schema`, `pg_catalog` and `pg_toast` to the excluded schemas. |
| `included_schemas` | Comma-separated schema names. When non-empty, this `IN` list replaces excludes. |
| `excluded_schemas` | Comma-separated schema names, used only when `included_schemas` is empty. |
| `limit`, `offset` | Integers. `0` is omitted, matching the JavaScript truthiness check. Any other integer is passed through `literal`. |
| `include_columns` | Boolean. Default true. |

`GET /pg/tables/:id`, `PATCH` and `DELETE` take a decimal id (`:id(\\d+)`).
`DELETE` also takes optional `cascade` (boolean, default false).

`POST /pg/tables` JSON: required `name` (string), optional `schema` (string,
default `public`), optional `comment` (string).

`PATCH /pg/tables/:id` JSON, every field optional: `name`, `schema`,
`rls_enabled`, `rls_forced`, `replica_identity` (`DEFAULT`, `INDEX`, `FULL`,
`NOTHING`), `replica_identity_index`, `primary_keys` (`[{ "name": string }]`),
`comment`.

Megabase runs the statements on a new connection taken from the process pool,
the same way `GET /pg/config/version` does. It does not open the `pg` or
`x-connection-encrypted` header as a connection string.

## Outputs

`Content-Type: application/json; charset=utf-8`.

List: 200 JSON array of table rows. Retrieve, create, update and delete: 200
one table row (delete returns the row from before `DROP`).

A row has `id`, `schema`, `name`, `rls_enabled`, `rls_forced`,
`replica_identity`, `bytes`, `size`, `live_rows_estimate`,
`dead_rows_estimate`, `comment`, `primary_keys`, `relationships`, and, when
columns are included, `columns`. Integers are JSON numbers. The SQL is the
pinned `TABLES_SQL` / `COLUMNS_SQL`, wrapped in `json_agg(to_jsonb(...))` so
PostgreSQL encodes the row.

## Errors

| When | Status | Body |
|---|---|---|
| No `apikey` | 401 | Kong `No API key found in request` |
| `apikey` is not the service-role key | 403 | Kong `You cannot consume this service` |
| No database configured | 500 | Same body as `GET /pg/config/version`: `statusCode`, `error`, `message` `failed to get upstream connection details` |
| List query fails | 500, or 504 / 503 / 408 for the three messages below | `{"error":"<PostgreSQL message>"}` |
| Retrieve fails, including an unknown id | 404 | `{"error":"<message>"}` |
| Create fails | 400 | `{"error":"<message>"}` |
| Update or delete fails | 400, or 404 when the message starts with `Cannot find` | `{"error":"<message>"}` |
| Body or query fails the route schema | 500 | `{"error":"<Fastify schema message>"}` |

`translateErrorToResponseCode` maps `Connection terminated due to connection timeout` to 504, `sorry, too many clients already` to 503, and `Query read timeout` to 408. Those mappings apply to list only. Create, update and delete keep 400 (or 404 for `Cannot find`) and put the same message in `error`. Connect waits 15 seconds. A query waits 55 seconds.

An id of `0` (including `00`) never reaches a query: JavaScript treats it as
missing, so retrieve returns `Invalid parameters on table retrieve`. Get
answers 404. Patch and delete answer 400.

Schema failures use Fastify's default Ajv text (`body must have required property 'name'`, `body/name must be string`, `body/replica_identity must match a schema in anyOf`, `querystring/limit must be integer`, `params/id must be integer`, and the same shape for the other fields). The pinned `setErrorHandler` sends that text as 500 `{"error":"..."}`.

A body that is not a JSON object uses `body must be object`. A body that is
not JSON at all uses serde_json's parse error in that same 500 body. V8's
`SyntaxError` text differs; the status and the `error` key match.

## Edge cases

`included_schemas` wins over `excluded_schemas` and over the system-schema
exclude. Names are split on `,` and not trimmed. Schema filters and comments
go through `literal` (quotes and backslashes doubled; a backslash uses the
`E'...'` form). Identifiers go through `ident`: a lowercase unreserved name
stays bare; anything else, including reserved words, is double-quoted with
quotes doubled.

Upstream interpolates `replica_identity_index` raw. Megabase passes it through
`ident`. A plain unreserved name produces the same SQL. A mixed-case or
reserved name is quoted, so it cannot close the statement.

`limit` and `offset` are rendered with `literal`, so the SQL is `limit '10'`
the way pg-format quotes numbers.

Create and update run their statements in one transaction, in the pinned
order: row security, replica identity, primary keys, comment, `SET SCHEMA`,
rename. Rename uses the new schema when both change. Comment, row security
and primary keys still name the table as it was. An empty primary-key list
drops an existing primary key and adds none. `cascade` false emits
`RESTRICT`; true emits `CASCADE`.

A path whose id is not all digits is not this route. It stays 501.

## Out of scope

Column, foreign-table, privilege and other `/pg` routes stay 501. Judge cases
for these five units belong in `judge/cases/meta.toml` on a `review/` branch.

## Judge cases

Not in this tree. `judge/` is protected, so the cases are a follow-up on
`review/issue-90-cases`.
