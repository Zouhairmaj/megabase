# REST: query-params (issue 41)

Status: draft
Unit ids (`coverage/units.json`): `rest:query-param:and`,
`rest:query-param:columns`, `rest:query-param:limit`,
`rest:query-param:offset`, `rest:query-param:on_conflict`,
`rest:query-param:or`, `rest:query-param:order`, `rest:query-param:select`
Level: 1

Reserved query parameters on `GET /rest/v1/{relation}`. Clients use them to
choose columns, order and page the rows, and combine filters with `and` and
`or`. `columns` and `on_conflict` are parsed on a read and do not change the
row shape.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Parse order: `order`, logic, `columns`, `select`, filters, `on_conflict` | [`QueryParams.hs:162`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L162) |
| `and` / `or` | [`QueryParams.hs:185`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L185) |
| `select` | [`QueryParams.hs:186`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L186) |
| `on_conflict` | [`QueryParams.hs:187`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L187) |
| `columns` | [`QueryParams.hs:188`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L188) |
| `order` | [`QueryParams.hs:189`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L189) |
| `limit` | [`QueryParams.hs:190`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L190) |
| `offset` | [`QueryParams.hs:192`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L192) |
| Select, order, logic, and column parsers | [`QueryParams.hs:260`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L260) |
| `order` term grammar | [`QueryParams.hs:826`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L826) |
| Logic tree | [`QueryParams.hs:883`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L883) |
| `PGRST108` message and hint | [`Error.hs:184`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Error.hs#L184) |
| Plan: orders, then ranges, then logic | [`Plan.hs:424`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Plan.hs#L424) |
| `LIMIT` / `OFFSET` | [`SqlFragment.hs:551`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Query/SqlFragment.hs#L551) |
| `ORDER BY` | [`SqlFragment.hs:574`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Query/SqlFragment.hs#L574) |
| Filter key `pTreePath`: embed path, column, JSON path | [`QueryParams.hs:329`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L329) |
| JSON path field: `to_jsonb` unless `json`/`jsonb` | [`Plan.hs:354`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Plan.hs#L354) |
| `pgFmtField`, `pgFmtJsonPath` | [`SqlFragment.hs:283`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Query/SqlFragment.hs#L283) |
| `Content-Range` without a total | [`RangeQuery.hs:113`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/RangeQuery.hs#L113) |

Pins: `postgrest v16.4` (`vendor.toml`). Ported code carries the credit header
required by AGENTS.md.

## Inputs

`GET /rest/v1/{relation}` with a query string. A key without `=` is ignored.
`+` in a query value is a space. The first `select`, `columns`, and
`on_conflict` win. The first root `order` wins. The last root `limit` and the
last root `offset` win. Every root `and` and `or` is kept and combined with
`AND` after the horizontal filters.

`select` lists field names, `*`, `alias:column`, a JSON path (`->` / `->>`
with a key or an integer index), and `::cast`. A missing `select` is `*`.
`order` lists `column[.asc|.desc][.nullsfirst|.nullslast]`, where the column
may carry a JSON path. A filter key is `[embed.]column` with an optional JSON
path (`payload->user->>name=eq.a`); so is a leaf inside `and` / `or`. `limit` and
`offset` are integers (`readInteger`: optional sign, digits, no trailing
junk). `and` and `or` take a parenthesized list of the same filter
expressions as [filtering](filtering.md), including nested calls.
`columns` and `on_conflict` are comma-separated field names.

## Outputs

`200` with a JSON array and `Content-Type: application/json; charset=utf-8`.
`Content-Range` is `{offset}-{offset+count-1}/*`. An empty page is `*/*`.
`select=*` and a missing select return every column. A list returns those
columns, in that order, with aliases and casts applied. A JSON path with no
alias is named after its last key. An index uses the previous key, or the
column name when the path has no key (`Plan.hs` `addAliases`). `columns` and
`on_conflict` do not project or constrain the read. A JSON-path filter
applies the path to the column when it is `json` or `jsonb`, and to
`to_jsonb(column)` otherwise (a missing column too, so PostgreSQL reports
`42703`). The value is compared as `text` after `->>`, and as the column's
`json` or `jsonb` after `->`. A JSON-path filter never matches a null embed
(`books=is.null`), and `PUT` does not count it as a key filter. `order` appends
`ASC`, `DESC`, `NULLS FIRST`, or `NULLS LAST` only when the term asked for
them. `limit=0` returns no rows. A nonzero `offset` without `limit` is
`LIMIT ALL OFFSET n`.

## Errors

| When | Status | Code / message (upstream) |
|---|---|---|
| `order`, logic, `columns`, `select`, a filter, or `on_conflict` does not parse | 400 | `PGRST100`. The first parser in the `parse` do-block wins. `order=id.ac` is column 4, unexpected `"c"`, expecting `"asc", "desc", "nullsfirst" or "nullslast"` |
| A filter, `order`, `limit`, `offset`, `and`, or `or` names a resource that is not embedded (`a.b=eq.1`, `items.order`, `order=clients(id)`) | 400 | `PGRST108` `'{resource}' is not an embedded resource in this request`, hint `Verify that '{resource}' is included in the 'select' query parameter.`, details null. The rightmost parameter in each stage wins; filters are checked before order, then limit/offset, then logic, then a related order term. Embedded parameters on an embed in `select` follow [`embedding.md`](embedding.md) |
| Spread, a quoted filter name, an empty key segment (`a..b=eq.1`), or text after a filter key's JSON path (`a->b)=eq.1`) | 501 | Route unit `{METHOD} {path}`, after a successful parse of the other parameters. `select=notes(body)&id=nope` is still `PGRST100` |
| Column is missing | 400 | PostgreSQL `42703` |
| `limit` or `offset` is negative, or does not fit `bigint` | 400 | PostgreSQL `2201W`, `2201X`, or `22003` |
| `DATABASE_URL` is unset | 503 | `PGRST000` |

## Edge cases

- Whitespace-only `select= ` is `PGRST100`. Empty `select=` is `*`.
- `columns=` and `on_conflict=` are `PGRST100`. `*` is not a field name there.
- An invalid `limit` or `offset` integer is ignored. A later invalid `limit`
  clears an earlier valid one. `offset=0` alone adds no `LIMIT`.
- `not.or=(...)` is `NOT ( ... )`. Inside a logic list, `in.(1,2)` and quoted
  commas stay one value. `or(id.in.1,2,id.eq.3)` is `PGRST100`.
- An operator on `json` after `->` (`json_col->a=eq.1`) is PostgreSQL's
  `42883`: `json` has no `=`.
- A raw `"`, `<` or `>` in the request-target (`curl '…?select=a->b'`) is
  percent-encoded by the gateway before hyper parses it, so it gets the same
  answer as the encoded form instead of an empty `400`
  (`crates/megabase-server/src/target.rs`).
- `id .desc` is `id` descending. `id.desc ` (trailing space) is `PGRST100`.
- Duplicate root orders keep the first. Duplicate root limits keep the last.
- Values, JSON keys, JSON indexes, and limit/offset integers are bound
  parameters. Direction, nulls placement, and cast targets are fixed tokens.
  A cast that contains `--` or `/*` is `PGRST100`.
- Bearer verification runs before the query string is parsed.

## Out of scope

Spreads (embeds are in [`embedding.md`](embedding.md)), `Prefer`, the `Range` header, object and CSV
media types, and writes. Those return HTTP 501 via
`megabase_core::MegabaseNotImplemented`. `columns` and `on_conflict` are
stored for a later write and are not applied to `POST` in this unit.
Horizontal operators stay in [filtering](filtering.md). `asc`, `desc`,
`nullsfirst`, `nullslast`, and the logic operators `and` / `or` / `not`
inside a tree are behavior of these parameters. Their own unit ids are not
marked here.

## Judge cases

`judge/cases/rest.toml`: `rest.select.all` (`select=*&order=id`),
`rest.select.columns` (`select=id,title&order=id.desc`),
`rest.limit-offset` (`order=id&limit=1&offset=1`),
`rest.filter.in` (`id=in.(1,3)&order=id`).
The review PR `review/issue-41-query-params-cases` adds `rest.query.and`,
`rest.query.or`, `rest.query.columns`, and `rest.query.on-conflict`.
