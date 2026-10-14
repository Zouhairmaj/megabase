# REST: media-types

Status: partial
Unit ids: `rest:media-type:` + `application/json`, `*/*`, `text/csv`,
`text/plain`, `text/xml`, `application/octet-stream`, `application/geo+json`,
`application/openapi+json`, `application/x-www-form-urlencoded`,
`application/vnd.pgrst.object+json`, `application/vnd.pgrst.object+json;nulls=stripped`,
`application/vnd.pgrst.array+json;nulls=stripped`, `application/vnd.pgrst.plan+{format}`
Level: 1

Content negotiation via `Accept` and `Content-Type`. Template:
[`specs/_template.md`](../_template.md).

## Upstream

- `vendor/postgrest/src/library/PostgREST/MediaType.hs` (`toMime` lines 77-95,
  `toContentType` line 68)
- `Plan.hs:219,229,253`: reads, writes and RPC share `negotiateContent`
  (`Plan/Negotiate.hs:56-85`).
- `Query/SqlFragment.hs:226-242` (`asJsonF`, `asJsonSingleF`),
  `MainTx.hs:163-190,239-243` (`failMutation`, `failNotSingular`),
  `Response.hs` (`contentTypeHeaders`).
- Pin: `postgrest v16.4` (`vendor.toml`).

## Behavior

All comma-separated `Accept` entries are considered. Entries with `q=0` are
dropped; the entry with the highest `q` wins. Ties go to a concrete type over a
wildcard, then to `nulls=stripped`, then to header order. On `GET`/`HEAD` of a
relation, entries with no handler are skipped first (see below).

| Accept | Served | Response `Content-Type` |
|---|---|---|
| `*/*`, `application/json`, `application/*`, none | GET/HEAD/writes/RPC | `application/json; charset=utf-8` |
| `application/vnd.pgrst.array+json` | GET/HEAD on a relation | `application/json; charset=utf-8` (plain alias) |
| `application/openapi+json` | root document | `application/openapi+json; charset=utf-8` |
| `application/vnd.pgrst.object+json` | GET/HEAD/writes/RPC | `application/vnd.pgrst.object+json; charset=utf-8` |
| `application/vnd.pgrst.object+json;nulls=stripped` | GET/HEAD/writes/RPC | `application/vnd.pgrst.object+json;nulls=stripped; charset=utf-8` |
| `application/vnd.pgrst.array+json;nulls=stripped` | GET/HEAD/writes/RPC | `application/vnd.pgrst.array+json;nulls=stripped; charset=utf-8` |
| `text/csv` | GET/HEAD on a relation | `text/csv; charset=utf-8` |
| `application/geo+json` | GET/HEAD on a relation | `application/geo+json; charset=utf-8` |
| `application/octet-stream` | request `Content-Type` on RPC only | (406 as an `Accept`, see below) |

Singular object: the result must be exactly one row. The body is that row as
an object, not an array. Any other row count returns 406 `PGRST116`, message
`Cannot coerce the result to a single JSON object`, details
`The result contains N rows`.

On `POST`, `PATCH` and `DELETE` the result is the affected rows, whatever
`Prefer: return` says: `return=minimal` with two affected rows is still 406.
The check runs after the statement and the transaction is rolled back, so
the rows stay unchanged (`MainTx.hs` `failNotSingular`, `SQL.condemn`).
`PUT` is exempt: its only count check is `PGRST115`. With
`return=representation` the body is `json_agg(rows)->0` and the response
`Content-Type` is the negotiated type; without a body no `Content-Type` is
sent. Status, `Content-Range` and `Location` are those of the plain write
(`specs/rest/prefer.md`). Check and RLS errors keep their codes.

On RPC the result is the page count: a scalar or a single composite is one
row, a set must return exactly one row, and a `void` function stays 204. A
failed check rolls a volatile call back. For a set of scalars the singular
body is the call row (`{"pgrst_scalar": …}`), as `asJsonSingleF` only reads
`pgrst_scalar` for a single scalar.

`nulls=stripped` removes object members whose value is JSON `null`
(`json_strip_nulls`, at every depth), keeping key order. `Content-Range` is
unchanged. Whitespace around `=` is ignored.

## Text and binary media types on a relation (`GET`/`HEAD /rest/v1/{relation}`)

Upstream: `MediaType.hs:80-88` (`toMime`), `Plan/Negotiate.hs`
(`negotiateContent`), `SchemaCache.hs:1082` (`initialMediaHandlers`),
`Query/SqlFragment.hs:224,249` (`asCsvF`, `asGeoJsonF`), `Error.hs`
(`MediaTypeError`, `mapSQLtoHTTP`).

- **Negotiation.** Builtin handlers for a table or view exist for `*/*`,
  `application/json`, `application/geo+json` and `text/csv`. Entries without
  a handler are skipped before ranking: `text/plain`, `text/xml`,
  `application/octet-stream` and types PostgREST does not know. The
  remaining entries rank as in the table above. Writes and RPC run the same
  negotiation (`Plan.hs:229,253`), so `text/plain` on a `POST` is the same
  406; `text/csv` and `application/geo+json` are 501 there.
- **`text/csv`.** 200, `Content-Type: text/csv; charset=utf-8`, the usual
  `Content-Range`. Body: the column names of the first result row joined by
  `,`, `\n`, then each row as record text (`row::text` without its outer
  parentheses) joined by `\n`. An empty result is `\n`. Filters, order and
  paging apply as for JSON. `HEAD` has the same headers and no body.
- **`application/geo+json`.** The body is built in SQL by `asGeoJsonF`:
  `json_build_object('type', 'FeatureCollection', 'features',
  coalesce(json_agg(ST_AsGeoJSON(_postgrest_t)::json), '[]'))`, where
  `_postgrest_t` is the anonymous record of each result row. `ST_AsGeoJSON`
  comes from PostGIS, so the response depends on the database:
  - PostGIS installed and the row has a geometry column: 200,
    `Content-Type: application/geo+json; charset=utf-8`, the usual
    `Content-Range`, and one `Feature` per row (`geometry` from the
    geometry column, the other columns under `properties`). No rows is
    `{"type":"FeatureCollection","features":[]}`.
  - PostGIS installed, no geometry column: the PostGIS error (`22023`
    `geometry column is missing`), 400.
  - PostGIS absent (the judge fixtures): Postgres `42883` `function
    st_asgeojson(record) does not exist` with its hint, which
    `mapSQLtoHTTP` turns into 404. Megabase runs the same SQL on the same
    cluster, so the error body matches. Function lookup follows the request
    `search_path`.
  - `HEAD` runs no aggregate upstream (`NoAgg`), so it is 200 with the
    geo+json `Content-Type` and `Content-Range` and no body even without
    PostGIS.
- **`text/plain`, `text/xml`, `application/octet-stream`.** No handler
  exists for a table or view (a handler needs a custom aggregate or a
  domain over the media type). If no accepted type has a handler, or every
  type has `q=0`: 406, `PGRST107`, message
  `None of these media types are available: <mime>, ...` listing the
  skipped types without parameters.
- Mixed lists (`text/plain, text/csv`, `text/xml, application/json`,
  `application/octet-stream, application/json`) pick the first handled type
  in rank order.
- The 406 is decided before the relation lookup, so a missing relation with
  `Accept: text/plain` is 406, not `PGRST205`.
- **Out of scope:** `text/csv` and `application/geo+json` on writes and RPC
  (501, including `Prefer: return=representation`), custom
  aggregate / domain handlers, `text/csv` as a request `Content-Type`, a
  specific `q=0` entry overriding a matching wildcard.

## Raw bodies on RPC (`application/octet-stream`)

Upstream: `ApiRequest/Payload.hs:75` (`RawPay`), `Plan.hs:307-320`
(`hasSingleUnnamedParam`, `matchesParams`), `Query/SqlFragment.hs:161`
(`singleParameter`), `Error.hs:246-288` (`NoRpc`).

`POST /rest/v1/rpc/{function}` with `Content-Type: application/octet-stream`
does not parse the body and carries no argument keys. The function is
resolved as usual, with two differences: a function without parameters does
not match, and the fallback is a function with exactly one unnamed `bytea`
parameter (not `json`/`jsonb`). The body is bound as a `bytea` parameter,
never cast from text, so every byte reaches the function. The response is
negotiated from `Accept` as for any RPC (JSON by default). When nothing
matches: 404 `PGRST202`, message `Could not find the function
<schema>.<name> in the schema cache` (no parameter list), details
`Searched for the function <schema>.<name> with a single unnamed bytea
parameter, but no matches were found in the schema cache.`, hint null.
`GET`/`HEAD` ignore the request `Content-Type`. `text/plain` and `text/xml`
bodies (one unnamed `text` / `xml` parameter upstream) are still 501.

## Form bodies (`application/x-www-form-urlencoded`)

Upstream: `ApiRequest/Payload.hs:68-72,93` (`parseSimpleQuery`), `Plan.hs:245`.

As a request `Content-Type` on `POST`/`PATCH`/`PUT /rest/v1/{relation}` and
`POST /rest/v1/rpc/{function}`, the body is split on `&` into `key=value`
pairs. `+` is a space and `%XX` is decoded. Empty pairs are dropped, a key
with no `=` has the value `""`, and the last duplicate key wins. The result
is one JSON object of text values for relation writes, then handled like a
JSON object body (Postgres casts the text to the column type). On RPC the
pairs are named arguments, as in a GET query string: repeated keys reach a
variadic parameter, and the unnamed `json`/`jsonb` parameter fallback applies
only to JSON bodies. An empty body is `{}` on writes. `Accept: application/x-www-form-urlencoded` has no handler and is not
served. RPC still returns 501 when the query string is non-empty.

## Not yet served (501 with the unit id)

`text/csv` and `application/geo+json` on writes and RPC,
`application/octet-stream` as an RPC `Accept` (Megabase answers 501; upstream
answers 406 when no custom handler exists, and that 406 is served only on
relations, see above), and `text/plain` / `text/xml` request bodies on RPC.

## Query plans (`application/vnd.pgrst.plan+{format}`)

Upstream: `MediaType.hs:91-96` (`toMime`), `Plan/Negotiate.hs:72-78`.

`+json`, `+text` and the bare `application/vnd.pgrst.plan` (text) are served
only when `db-plan-enabled` is on. It is off by default and in the Judge
config, and Megabase has no switch for it, so the entry has no handler: it is
skipped like `text/plain`, and if nothing else is acceptable the answer is 406
`PGRST107`. The message names the type as upstream `toMime` prints it:
`application/vnd.pgrst.plan+<json|text>; for="<mime>"` (`for` defaults to
`application/json`) plus `; options=` with the recognised flags of
`analyze|verbose|settings|buffers|wal` in that order. Returning the EXPLAIN
output when enabled is not implemented.

## Judge cases

`judge/cases/rest.toml`: `rest.media-type.*` and `rest.media.*`. The
`geojson.get` and `geojson.filtered` cases assert whatever the reference
cluster returns for `ST_AsGeoJSON` on `todos`; `geojson.head` is
PostGIS-independent. `octet-stream.rpc.bytea` calls the
`public.byte_count(bytea)` fixture. The singular shape on writes and RPC is
covered by `rest.media.object.*` (one-row insert and delete, update matching
zero and two rows, `return=minimal` with two rows, RPC scalar).
