# REST: media-types

Status: partial (text/csv, text/plain, text/xml on relation reads)
Unit ids: `rest:media-type:` + `application/json`, `*/*`, `text/csv`,
`text/plain`, `text/xml`, `application/octet-stream`, `application/geo+json`,
`application/openapi+json`, `application/x-www-form-urlencoded`,
`application/vnd.pgrst.object+json`, `application/vnd.pgrst.object+json;nulls=stripped`,
`application/vnd.pgrst.array+json;nulls=stripped`, `application/vnd.pgrst.plan+{format}`
Level: 1

Content negotiation via `Accept` and `Content-Type`. Template:
[`specs/_template.md`](../_template.md).

## Upstream

- `vendor/postgrest/src/library/PostgREST/MediaType.hs`
- Pin: `postgrest v16.4` (`vendor.toml`).

## Text media types on a relation (`GET`/`HEAD /rest/v1/{relation}`)

Upstream: `MediaType.hs:81-83` (`toMime`), `Plan/Negotiate.hs`
(`negotiateContent`), `SchemaCache.hs` (`initialMediaHandlers`),
`Query/SqlFragment.hs` (`asCsvF`), `Error.hs` (`MediaTypeError`).

- **Negotiation.** `Accept` is split on `,`, `q=0` entries are dropped and
  the rest are ordered by `q` (stable), as `parseHttpAccept` does; the first
  type with a handler wins. Builtin handlers for a table or view exist for
  `*/*`, `application/json`, `application/geo+json` and `text/csv`. Only
  `*/*`, `application/json` and `text/csv` are served here;
  `application/geo+json` is recognized but returns 501. A type without a
  handler is skipped.
- **`text/csv`.** 200, `Content-Type: text/csv; charset=utf-8`, the usual
  `Content-Range`. Body: the column names of the first result row joined by
  `,`, `\n`, then each row as record text (`row::text` without its outer
  parentheses) joined by `\n`. An empty result is `\n`. Filters, order and
  paging apply as for JSON. `HEAD` has the same headers and no body.
- **`text/plain`, `text/xml`.** No handler exists for a table or view
  (a handler needs a custom aggregate or a domain over the media type). If
  no accepted type has a handler: 406, `PGRST107`, message
  `None of these media types are available: <mime>, ...` listing every
  accepted type without parameters.
- Mixed lists (`text/plain, text/csv`, `text/xml, application/json`) pick the
  first handled type in `q` order.
- **Out of scope:** `Accept` on writes and RPC (still 501), custom
  aggregate / domain handlers, `text/csv` as a request `Content-Type`.
- Judge cases: none yet; they go in a separate `review/issue-34-cases` PR.
