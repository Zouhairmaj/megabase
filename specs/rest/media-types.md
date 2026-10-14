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

## Text media types on a relation (`GET`/`HEAD /rest/v1/{relation}`)

Upstream: `MediaType.hs:81-83` (`toMime`), `Plan/Negotiate.hs`
(`negotiateContent`), `SchemaCache.hs` (`initialMediaHandlers`),
`Query/SqlFragment.hs` (`asCsvF`), `Error.hs` (`MediaTypeError`).

- **Negotiation.** Builtin handlers for a table or view exist for `*/*`,
  `application/json`, `application/geo+json` and `text/csv`. Entries without
  a handler are skipped before ranking: `text/plain`, `text/xml` and types
  PostgREST does not know. The remaining entries rank as in the table above.
  `application/geo+json` is recognized but returns 501. Writes and RPC run
  the same negotiation (`Plan.hs:229,253`), so `text/plain` on a `POST` is
  the same 406; only `text/csv` is 501 there.
- **`text/csv`.** 200, `Content-Type: text/csv; charset=utf-8`, the usual
  `Content-Range`. Body: the column names of the first result row joined by
  `,`, `\n`, then each row as record text (`row::text` without its outer
  parentheses) joined by `\n`. An empty result is `\n`. Filters, order and
  paging apply as for JSON. `HEAD` has the same headers and no body.
- **`text/plain`, `text/xml`.** No handler exists for a table or view
  (a handler needs a custom aggregate or a domain over the media type). If
  no accepted type has a handler, or every type has `q=0`: 406, `PGRST107`,
  message `None of these media types are available: <mime>, ...` listing the
  skipped types without parameters.
- Mixed lists (`text/plain, text/csv`, `text/xml, application/json`) pick the
  first handled type in rank order.
- The 406 is decided before the relation lookup, so a missing relation with
  `Accept: text/plain` is 406, not `PGRST205`.
- **Out of scope:** `text/csv` on writes and RPC (501), custom
  aggregate / domain handlers, `text/csv` as a request `Content-Type`, a
  specific `q=0` entry overriding a matching wildcard.

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

`application/geo+json`, `application/octet-stream` (needs a single unnamed
`bytea` RPC parameter and a custom handler), `application/vnd.pgrst.plan+{format}`,
and `text/csv` on writes and RPC.

## Judge cases

Cases go in separate `review/issue-33-cases` and `review/issue-34-cases` PRs.
The singular shape on writes and RPC is covered by `rest.media.object.*` in
`judge/cases/rest.toml` (one-row insert and delete, update matching zero and
two rows, `return=minimal` with two rows, RPC scalar).
