# REST: filtering (issue 27)

Status: draft
Unit ids (`coverage/units.json`): `rest:filter-operator:eq`,
`rest:filter-operator:gt`, `rest:filter-operator:gte`,
`rest:filter-operator:ilike`, `rest:filter-operator:fts`,
`rest:filter-operator:cs`, `rest:filter-operator:cd`,
`rest:filter-operator:adj`, `rest:filter-operator:any`,
`rest:filter-operator:all`
Level: 1

Horizontal filters on `GET /rest/v1/{relation}`. A query runs only when every
filter value is one of the operators above and the request does not ask for
another unimplemented REST feature. Clients use these operators to restrict
rows (`done=eq.true`, `priority=gt.1`, `title=ilike.*spec*`).

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Operator names `eq`, `gt`, `gte`, `ilike`, `cs`, `cd`, `adj` | [`QueryParams.hs:234`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L234) |
| `any` / `all` quantifiers | [`QueryParams.hs:717`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L717) |
| `fts` and optional `(language)` | [`QueryParams.hs:730`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L730) |
| SQL for those operators | [`SqlFragment.hs:130`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Query/SqlFragment.hs#L130) |
| `ilike` `*` to `%`, `ANY` / `ALL` | [`SqlFragment.hs:424`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Query/SqlFragment.hs#L424) |
| `PGRST100` message | [`QueryParams.hs:929`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L929) |
| Read `Content-Range` | [`RangeQuery.hs:113`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/RangeQuery.hs#L113) |

Pins: `postgrest v16.4` (`vendor.toml`). Ported code carries the credit header
required by AGENTS.md.

## Inputs

`GET /rest/v1/{relation}` with a query string. `{relation}` is one path
segment in schema `public`. `+` in the query is a space. `%HH` is
percent-decoded.

Each `column=operator.value` pair is a filter. Served shapes:

| Value | SQL shape |
|---|---|
| `eq.value`, `gt.value`, `gte.value` | `"relation"."col" op ($n::text)::column_type` |
| `ilike.pattern` | `ilike`, every `*` in the pattern becomes `%` |
| `cs.value`, `cd.value`, `adj.value` | `@>`, `<@`, `-|-` |
| `fts.terms`, `fts(language).terms` | `@@ to_tsquery` with an optional regconfig |
| `op(any).value`, `op(all).value` | the same operator with `ANY` or `ALL` and `column_type[]` |

`op` in the last row is one of `eq`, `gt`, `gte`, `ilike`. Repeated filters
are `AND`. Values are bound parameters. The column is qualified with the
relation name. The column type is `format_type` from `pg_catalog` after a
bound lookup of the relation. A column that is not in the catalog is still
quoted and sent, so PostgreSQL reports `42703`.

A bearer token is verified with `JWT_SECRET` when `Authorization` is a
bearer token. No `Authorization` header uses role `anon` and claims
`{"role":"anon"}`. The transaction sets `role`, `request.jwt.claims`,
`request.method` (`GET`), `request.path` (`/{relation}`), and `search_path`
(`public`) with `set_config(..., true)` before the read, so row security
applies.

## Outputs

Status 200. `Content-Type` is `application/json; charset=utf-8`. The body is
`json_agg` of the matching rows, or `[]`. Column order is `attnum` order.
`Content-Range` is `0-{n-1}/*` when `n > 0`, and `*/*` when the array is
empty. There is no `ORDER BY` unless a later unit adds `order`.

## Errors

| When | Status | Code / message (upstream) |
|---|---|---|
| Value is not an operator expression (`id=0`, `id=nope.1`) | 400 | `PGRST100`. `id=nope.1` is line 1 column 1, details `unexpected "p" expecting "not" or operator (eq, gt, ...)`. A failed name is reported at the start of that name; a failed `.` is reported on that character (`notX` expects `delimiter (.)`) |
| Relation is missing | 404 | `PGRST205` `Could not find the table 'public.{name}' in the schema cache` |
| Column is missing | 400 | PostgreSQL `42703` `column {table}.{col} does not exist`, hint naming a real column |
| Operator is not defined for the column type | 404 | PostgreSQL `42883`. `function xmlagg(` is 406 |
| Bearer present and `JWT_SECRET` is unset | 500 | `PGRST300` `Server lacks JWT secret` |
| Bearer fails verification | 401 | `PGRST301` or `PGRST303` with the `JwtError` message |
| `DATABASE_URL` is unset | 503 | `PGRST000` `Database connection error.` |
| PostgreSQL rejects the statement | 400, or 401/403 for `42501` | SQLSTATE as `code`, PostgreSQL message |

## Edge cases

- `eq.` is an empty string value, not a parse error.
- `gte` is tried before `gt`, and `ilike` before a prefix of `like`.
- `not.eq.1` is `rest:filter-operator:not` (not this issue) and returns 501.
- `lt`, `lte`, `neq`, `like`, `in`, `is`, and the other full-text operators
  return 501 for their own unit. `priority=gt.1&priority=lt.3` is 501 for
  `lt` and does not run.
- `select`, `order`, `limit`, `offset`, `and`, `or`, `columns`, and
  `on_conflict` return 501 for that query-param unit.
- `Accept: application/vnd.pgrst.object+json`, a `Prefer` header, a `Range`
  header, and `Accept-Profile` other than `public` return 501.
- A path that is not a single relation, and every method other than `GET`,
  stays 501 with unit `{METHOD} {path}`.
- `ilike` replaces every `*`, including inside `ilike(any)`.
- A catalog type that is not a safe cast target is not spliced into SQL.
- The value is cast to the column type. An operator that does not exist for
  that type is PostgreSQL `42883` (404). PostgREST's unknown literal can
  report `42725` instead when several candidates match (`text <@ unknown`).

## Out of scope

Operators other than the ten ids above, embeds, vertical filtering,
`order` / `limit` / `offset`, preferences, object and CSV media types, and
writes. Those return HTTP 501 via `megabase_core::MegabaseNotImplemented`.

## Judge cases

`judge/cases/rest.toml`: `rest.filter.eq` (`GET /rest/v1/todos?done=eq.true`).
`rest.filter.gt-lt` also lists `rest:filter-operator:lt`, which this issue
does not serve, so that case stays a 501 until filtering (2/3).
`rest.filter.unknown-operator` is the `PGRST100` shape for `id=nope.1`.
