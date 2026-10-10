# REST: resources (issue 43)

Status: complete
Unit ids (`coverage/units.json`): `rest:route:GET /rest/v1/`,
`rest:route:HEAD /rest/v1/`, `rest:route:OPTIONS /rest/v1/`,
`rest:route:GET /rest/v1/{relation}`, `rest:route:HEAD /rest/v1/{relation}`,
`rest:route:OPTIONS /rest/v1/{relation}`, `rest:route:POST /rest/v1/{relation}`,
`rest:route:PUT /rest/v1/{relation}`, `rest:route:PATCH /rest/v1/{relation}`,
`rest:route:DELETE /rest/v1/{relation}`
Level: 1

Table and view routes under `/rest/v1/`, plus the root OpenAPI document.
Clients read rows, insert, update, upsert, and delete, and they discover
the schema with `GET /rest/v1/` and `OPTIONS`.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Path: root, one relation, `rpc/{fn}`, else invalid | [`ApiRequest.hs:147`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest.hs#L147) |
| Method to action | [`ApiRequest.hs:158`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest.hs#L158) |
| `Accept-Profile` on reads, `Content-Profile` on writes | [`ApiRequest.hs:180`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest.hs#L180) |
| JSON body, empty body, mismatched keys | [`Payload.hs:40`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/Payload.hs#L40) |
| Insert, update, delete, empty-object row | [`QueryBuilder.hs:130`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Query/QueryBuilder.hs#L130), [`SqlFragment.hs:344`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Query/SqlFragment.hs#L344) |
| Status, `Content-Range`, empty minimal body | [`Response.hs:94`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Response.hs#L94) |
| `OPTIONS` `Allow` | [`Response.hs:219`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Response.hs#L219) |
| Insertable, updatable, deletable; skip partitions | [`SchemaCache.hs:744`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/SchemaCache.hs#L744) |
| OpenAPI document | [`OpenAPI.hs:487`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Response/OpenAPI.hs#L487) |
| Error codes and messages | [`Error.hs:148`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Error.hs#L148) |
| `Content-Range` text | [`RangeQuery.hs:113`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/RangeQuery.hs#L113) |

Pins: `postgrest v16.4` (`vendor.toml`). Ported code carries the credit header
required by AGENTS.md.

## Inputs

The gateway prefix is `/rest/v1`. Empty path segments are dropped, so
`/rest/v1` and `/rest/v1/` are the root. `/rest/v1` without a slash is the
Studio prefix and stays 501. One remaining segment is `{relation}`.
`rpc` plus one more segment stays the RPC unit (501). Any other shape is
`PGRST125`.

The exposed schemas are `public` and `graphql_public`. The default is
`public`. Reads and `OPTIONS` take `Accept-Profile`. Writes take
`Content-Profile`. A profile outside that list is `PGRST106`.

A blank query (`""`, `?`, `&&&`) is a read of every column with no filter.
A query that parses and is handled (filters, `select`, `order`, `limit`,
`offset`, `and`, `or`, `columns`, `on_conflict`) is executed. A non-blank
query that does not parse as a served parameter stays 501, including
`?select` with no `=`.

`Authorization` absent, or not a bearer token, is the `anon` role with
claims `{"role":"anon"}`. A bearer token is verified with HS256. The
statement runs after `SET LOCAL` role, `request.jwt.claims`,
`request.method`, `request.path` (`/{relation}`, not the gateway prefix),
and `search_path` to the negotiated schema. Reads use a read-only
transaction. Catalog lookups (existence, columns, privileges, primary key)
run before `SET ROLE`.

JSON is the default body media type when `Content-Type` is missing.
`application/json` and `application/*` are accepted. Other known media
types stay their own 501 unit. An unknown type is `PGRST102`.

## Outputs

| Request | Status | Headers that matter | Body |
|---|---|---|---|
| `GET /rest/v1/{relation}` | 200 | `Content-Type: application/json; charset=utf-8`, `Content-Range: 0-(n-1)/*` or `*/*` when empty, `Content-Profile` | JSON array of rows |
| `HEAD /rest/v1/{relation}` | 200 | same as GET | empty |
| `OPTIONS /rest/v1/{relation}` | 200 | `Allow`, `Access-Control-Allow-Origin: *` | empty |
| `OPTIONS /rest/v1/` | 200 | `Allow: OPTIONS,GET,HEAD`, `Access-Control-Allow-Origin: *` | empty |
| `GET /rest/v1/` | 200 | `Content-Type: application/openapi+json; charset=utf-8` unless `Accept` is `application/json`, then `application/json; charset=utf-8`. `Content-Profile` | Swagger 2.0 document |
| `HEAD /rest/v1/` | 200 | same as GET | empty |
| `POST` default | 201 | `Content-Range: */*`, no `Content-Type` | empty |
| `PATCH` default | 204 | `Content-Range: 0-(n-1)/*` or `*/*` when zero rows | empty |
| `DELETE` default | 204 | `Content-Range: */*` | empty |
| `PUT` default | 204 | no `Content-Range`, no `Content-Type` | empty |

`Allow` on a relation is `OPTIONS,GET,HEAD`, then `POST` when insertable,
`PUT` when insertable and updatable and the relation has a primary key,
`PATCH` when updatable, and `DELETE` when deletable. Tables and partitions
(`relkind` `r` and `p`) are insertable, updatable, and deletable. Views and
foreign tables use `pg_relation_is_updatable` bits 8, 4, and 16. Materialized
views are not. Partitions (`relispartition`) are absent from the cache.

The root document is Swagger 2.0: title `PostgREST API` unless the schema
comment's first line replaces it, description `This is a dynamic API
generated by PostgREST` unless the rest of that comment replaces it, version
`16.4`, host `!4:3000`, basePath `/`, schemes `http`. Paths are `/` and each
table the role can access. RPC paths are omitted.

Mutation JSON is one bound parameter. Column and relation names are
`quote_ident` after the catalog lookup. An empty object `{}` inserts one
default row. An empty array inserts nothing. `{}` on `PATCH` updates no
rows (`WHERE false`) and returns 204 with `Content-Range: */*`.

`PUT` is a single upsert: `ON CONFLICT` on the primary key. The filters
must be exactly those columns, each a non-negated `eq`, with no logic tree.

## Errors

| When | Status | Code / message (upstream) |
|---|---|---|
| No pool, or the pool cannot start a transaction | 503 | `PGRST000` Database connection error. |
| Bearer token and no JWT secret | 500 | `PGRST300` Server lacks JWT secret |
| Bad or expired JWT | 401 | `PGRST301` or `PGRST303`, plus `WWW-Authenticate` |
| Unknown relation | 404 | `PGRST205` Could not find the table '{schema}.{relation}' in the schema cache |
| More than one path segment, and not `rpc/{fn}` | 404 | `PGRST125` Invalid path specified in request URL |
| Method not in the table above | 405 | `PGRST117` Unsupported HTTP method: {METHOD} |
| Schema not in `public`, `graphql_public` | 406 | `PGRST106` Invalid schema: {name}. Hint lists the exposed schemas |
| Empty or invalid JSON on POST, PUT, PATCH | 400 | `PGRST102` Empty or invalid json |
| Array objects whose keys differ | 400 | `PGRST102` All object keys must match |
| Unknown `Content-Type` | 400 | `PGRST102` Content-Type not acceptable: {type} |
| Column not in the cache | 400 | `PGRST204` Could not find the '{column}' column of '{relation}' in the schema cache |
| `PUT` with `limit` or `offset` | 400 | `PGRST114` limit/offset querystring parameters are not allowed for PUT |
| `PUT` filters are not exactly the primary key `eq`s | 405 | `PGRST105` Filters must include all and only primary key columns with 'eq' operators |
| `PUT` body primary key does not match the URL, or the statement did not change one row | 400 | `PGRST115` Payload values do not match URL in primary key column(s). The transaction is rolled back |
| `42501` as `anon` | 401 | PostgreSQL message, code `42501` |
| `42501` as any other role | 403 | PostgreSQL message, code `42501` |
| `23505` or `23503` | 409 | PostgreSQL message |
| `Accept` on the root that names an unavailable type | 406 | `PGRST107` |

`details` and `hint` are JSON null unless the row above names a hint.
PostgREST's fuzzy table hint is not served; `hint` stays null on `PGRST205`.

`PUT` checks order is: query parse (`PGRST100`), then `Prefer` (501), then
`PGRST114`, then the body (`PGRST102`), then the database, then `PGRST105`
against the catalog primary key, then `PGRST115`. An empty `PUT` body is
`PGRST102` even when the filters are missing. Filters with no `eq` are
`PGRST105` before the pool is required, once the body is valid JSON.

## Edge cases

A missing `Content-Type` is JSON. A JSON value that is neither an object nor
an array is treated as an empty array. `columns=` on `POST` and `PATCH`
names the written columns and skips the key-uniformity check; an unknown
name is still `PGRST204`. `on_conflict` is ignored unless `Prefer`
resolution is set, and that preference stays 501.

`HEAD` keeps the success headers and drops the body. Error bodies stay JSON.
`Range` on `GET` stays 501. `HEAD` ignores `Range`. `max-rows` is not applied.

Existence uses `relkind` `r`, `p`, `v`, `m`, `f` and `NOT relispartition`.
The OpenAPI list uses the same kinds and `has_table_privilege` after
`SET ROLE` (`openapi-mode` follow-privileges). `OPTIONS` does not `SET ROLE`.

## Out of scope

RPC (`/rest/v1/rpc/{function}`), embeds, aggregates, `Prefer` (including
`return=representation`, `return=minimal`, and `count`), `Range` on `GET`,
and media types other than JSON and the root OpenAPI document. Those return
HTTP 501 via `megabase_core::MegabaseNotImplemented`. Vertical filters,
ordering, limits, and `and` / `or` on a read are the query-parameter and
filtering units, and a bare or filtered read uses them.

## Judge cases

`judge/cases/rest.toml` already lists `GET`, `POST`, `PATCH`, and `DELETE`
on `{relation}` (select, filters, missing table, RLS). This feature branch
does not edit `judge/` ([ADR 0003](../../docs/adr/0003-protected-paths.md)).
Cases that send `Prefer` or an embed stay 501 until those units are served.
`HEAD`, `OPTIONS`, `PUT`, and `GET /rest/v1/` have no case in that file yet;
they belong in a `review/issue-43-cases` pull request.
