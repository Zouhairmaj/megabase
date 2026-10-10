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
| `application/vnd.pgrst.object+json` | GET/HEAD on a relation | `application/vnd.pgrst.object+json; charset=utf-8` |
| `application/vnd.pgrst.object+json;nulls=stripped` | GET/HEAD on a relation | `application/vnd.pgrst.object+json;nulls=stripped; charset=utf-8` |
| `application/vnd.pgrst.array+json;nulls=stripped` | GET/HEAD on a relation | `application/vnd.pgrst.array+json;nulls=stripped; charset=utf-8` |
| `text/csv` | GET/HEAD on a relation | `text/csv; charset=utf-8` |

Singular object: the result must be exactly one row. The body is that row as
an object, not an array. Any other row count returns 406 `PGRST116`, message
`Cannot coerce the result to a single JSON object`, details
`The result contains N rows`.

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
  `application/geo+json` is recognized but returns 501.
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
- **Out of scope:** `Accept` on writes and RPC (still 501), custom
  aggregate / domain handlers, `text/csv` as a request `Content-Type`, a
  specific `q=0` entry overriding a matching wildcard.

## Not yet served (501 with the unit id)

`application/geo+json`, `application/octet-stream`,
`application/x-www-form-urlencoded` bodies, `application/vnd.pgrst.plan+{format}`,
and the singular/stripped/CSV shapes on writes and RPC.

## Judge cases

Cases go in separate `review/issue-33-cases` and `review/issue-34-cases` PRs.
