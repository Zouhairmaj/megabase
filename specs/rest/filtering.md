# REST: filtering (issues 27, 28, and 29)

Status: draft
Unit ids (`coverage/units.json`): `rest:filter-operator:eq`,
`rest:filter-operator:neq`, `rest:filter-operator:gt`,
`rest:filter-operator:gte`, `rest:filter-operator:lt`,
`rest:filter-operator:lte`, `rest:filter-operator:like`,
`rest:filter-operator:ilike`, `rest:filter-operator:match`,
`rest:filter-operator:imatch`, `rest:filter-operator:fts`,
`rest:filter-operator:plfts`, `rest:filter-operator:phfts`,
`rest:filter-operator:wfts`, `rest:filter-operator:cs`,
`rest:filter-operator:cd`, `rest:filter-operator:ov`,
`rest:filter-operator:sl`, `rest:filter-operator:sr`,
`rest:filter-operator:nxr`, `rest:filter-operator:nxl`,
`rest:filter-operator:adj`, `rest:filter-operator:any`,
`rest:filter-operator:all`, `rest:filter-operator:in`,
`rest:filter-operator:is`, `rest:filter-operator:isdistinct`,
`rest:filter-operator:not`
Level: 1

Horizontal filters on `GET /rest/v1/{relation}`. A query runs only when every
filter value is one of the operators above and the request does not ask for
another unimplemented REST feature. Clients use these operators to restrict
rows (`done=eq.true`, `priority=lt.3`, `id=in.(1,3)`, `title=like.*spec*`).

## Upstream

| Behavior | File:line (pin) |
|---|---|
| `neq`, `cs`, `cd`, `ov`, `sl`, `sr`, `nxr`, `nxl`, `adj`, then `eq`, `gte`, `gt`, `lte`, `lt`, `like`, `ilike`, `match`, `imatch` | [`QueryParams.hs:234`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L234) |
| `not.` prefix | [`QueryParams.hs:700`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L700) |
| `in.(...)` list | [`QueryParams.hs:706`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L706) |
| `is` tri-state keywords | [`QueryParams.hs:707`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L707) |
| `isdistinct` | [`QueryParams.hs:709`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L709) |
| `any` / `all` quantifiers | [`QueryParams.hs:717`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L717) |
| `fts`, `plfts`, `phfts`, `wfts` and optional `(language)` | [`QueryParams.hs:728`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L728) |
| SQL for those operators, `IS`, and `= ANY` | [`SqlFragment.hs:130`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Query/SqlFragment.hs#L130) |
| Range operator text (`&&`, `<<`, `>>`, `&<`, `&>`) | [`SqlFragment.hs:135`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Query/SqlFragment.hs#L135) |
| Full-text function names | [`SqlFragment.hs:154`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Query/SqlFragment.hs#L154) |
| `like` / `ilike` `*` to `%`, empty `in` | [`SqlFragment.hs:424`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Query/SqlFragment.hs#L424) |
| `PGRST100` message | [`QueryParams.hs:929`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L929) |
| Read `Content-Range` | [`RangeQuery.hs:113`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/RangeQuery.hs#L113) |

Pins: `postgrest v16.4` (`vendor.toml`). Ported code carries the credit header
required by AGENTS.md.

## Inputs

`GET /rest/v1/{relation}` with a query string. `{relation}` is one path
segment in schema `public`. `+` in the query is a space. `+` in the path
stays `+`. `%HH` is percent-decoded only when both characters are hex
digits. `%+1` is not an escape: the query keeps a literal `%` and then
treats `+` as a space.

Each `column=operator.value` pair is a filter. Served shapes:

| Value | SQL shape |
|---|---|
| `eq.value`, `neq.value`, `gt.value`, `gte.value`, `lt.value`, `lte.value` | `"relation"."col" op ($n::text)::column_type` |
| `like.pattern`, `ilike.pattern` | `like` / `ilike`, every `*` in the pattern becomes `%` |
| `match.pattern`, `imatch.pattern` | `~` / `~*`. `*` stays `*` |
| `cs.value`, `cd.value`, `ov.value`, `sl.value`, `sr.value`, `nxr.value`, `nxl.value`, `adj.value` | `@>`, `<@`, `&&`, `<<`, `>>`, `&<`, `&>`, `-\|-` |
| `fts.terms`, `plfts.terms`, `phfts.terms`, `wfts.terms`, and the same with `(language)` | `@@ to_tsquery`, `@@ plainto_tsquery`, `@@ phraseto_tsquery`, `@@ websearch_to_tsquery`. A language is a bound `regconfig`. The terms are bound `text`. The column type is not cast |
| `op(any).value`, `op(all).value` | the same operator with `ANY` or `ALL` and `column_type[]` |
| `in.(a,b)`, `in.("a,b",c)` | `= ANY` of one bound array literal. A quoted element keeps commas. A backslash escapes the next character inside quotes |
| `in.()` and `in.(   )` | `= ANY('{}')` with no parameter. Space and tab inside the parentheses count as empty |
| `is.null`, `is.not_null`, `is.true`, `is.false`, `is.unknown` | `IS NULL`, `IS NOT NULL`, `IS TRUE`, `IS FALSE`, `IS UNKNOWN`. Matching is case-insensitive and ignores text after the keyword. No parameter |
| `isdistinct.value` | `IS DISTINCT FROM` a bound value |
| `not.` plus a served operator | `NOT` before the same predicate |

`op` in the quantifier row is one of `eq`, `gt`, `gte`, `lt`, `lte`, `like`,
`ilike`, `match`, `imatch`. `neq` does not take `(any)` or `(all)`. Repeated
filters are `AND`. Values are bound parameters. The column is qualified with
the relation name. The column type is `format_type(atttypid, NULL)` from
`pg_catalog` after a bound lookup of the relation, so a `varchar(n)` or
`numeric(p,s)` cast does not apply the typmod. A column that is not in the
catalog is still quoted and sent, so PostgreSQL reports `42703`.

A bearer token is verified with `JWT_SECRET` when `Authorization` is a
bearer token. No `Authorization` header uses role `anon` and claims
`{"role":"anon"}`. The transaction is `READ ONLY`. It sets `role`, `request.jwt.claims`,
`request.method` (`GET`), `request.path` (`/{relation}`), and `search_path`
(`public`) with `set_config(..., true)` before the read, so row security
applies.

## Outputs

Status 200. `Content-Type` is `application/json; charset=utf-8`. The body is
`json_agg` of the matching rows, or `[]`. Column order is `attnum` order.
`Content-Range` is `0-{n-1}/*` when `n > 0`, and `*/*` when the array is
empty. The count is `pg_catalog.count` of the aggregated rows, not a second
parse of the JSON body. There is no `ORDER BY` unless a later unit adds `order`.

## Errors

| When | Status | Code / message (upstream) |
|---|---|---|
| Value is not an operator expression (`id=0`, `id=nope.1`) | 400 | `PGRST100`. `id=nope.1` is line 1 column 1, details `unexpected "p" expecting "not" or operator (eq, gt, ...)`. A failed name is reported at the start of that name; a failed `.` is reported on that character (`notX` expects `delimiter (.)`) |
| `is.` is not a tri-state keyword (`is.foo`) | 400 | `PGRST100` at the furthest keyword mismatch, expecting `isVal: (null, not_null, true, false, unknown)`. `<?>` does not replace that label, because the `is.` prefix already consumed input |
| `in.` is not a parenthesized list (`in.foo`) | 400 | `PGRST100` expecting `"("` or `")"` at that character |
| Relation is missing | 404 | `PGRST205` `Could not find the table 'public.{name}' in the schema cache` |
| Column is missing | 400 | PostgreSQL `42703` `column {table}.{col} does not exist`, hint naming a real column |
| Operator is not defined for the column type | 404 | PostgreSQL `42883`. `function xmlagg(` is 406 |
| Bearer present and `JWT_SECRET` is unset | 500 | `PGRST300` `Server lacks JWT secret` |
| Bearer fails verification | 401 | `PGRST301` or `PGRST303` with the `JwtError` message |
| `DATABASE_URL` is unset | 503 | `PGRST000` `Database connection error.` |
| PostgreSQL rejects the statement | 400, or 401/403 for `42501` | SQLSTATE as `code`, PostgreSQL message |

## Edge cases

- `eq.` is an empty string value, not a parse error.
- `gte` is tried before `gt`, `lte` before `lt`, and `ilike` before `like`.
- `not.eq.1` negates `eq`. `not.ov.[1,4)` negates overlap the same way.
  Every horizontal operator in this spec is served, including under `not.`.
- `in.("")` is the empty set, the same as `in.()`. `in.( ,3)` keeps the
  empty element and is not the empty set.
- `is.nullity` matches `null` and ignores the leftover. `is.not_null` does
  not match `null`.
- `select`, `order`, `limit`, `offset`, `and`, `or`, `columns`, and
  `on_conflict` return 501 for that query-param unit.
  `id=in.(1,3)&order=id` is 501 for `rest:query-param:order`.
- `Accept: application/vnd.pgrst.object+json`, a `Prefer` header, a `Range`
  header, and `Accept-Profile` other than `public` return 501.
- A path that is not a single relation, and every method other than `GET`,
  stays 501 with unit `{METHOD} {path}`.
- `like` and `ilike` replace every `*`, including inside `(any)` / `(all)`.
  `ov`, `sl`, `sr`, `nxr`, `nxl`, and the full-text operators keep `*`.
- `ov.[1,4)`, `sl.[9,10)`, `sr.[3,4)`, `nxr.[4,7)`, and `nxl.[4,7)` are
  range literals in the value. PostgreSQL applies `&&`, `<<`, `>>`, `&<`,
  and `&>` after the value is cast to the column type (`int4range`,
  `numrange`, an array, and the other types that define those operators).
- `plfts.The Fat Rats`, `phfts(english).The Fat Cats`, and
  `wfts(french).amusant impossible` follow `fts`: optional `(language)`
  then `.` then the rest of the value, including spaces. `plfts.` is an
  empty query string. `plfts().x` is `PGRST100`, the same shape as
  `fts().x`.
- A catalog type that is not a safe cast target is not spliced into SQL.
- The value is cast to the column type. An operator that does not exist for
  that type is PostgreSQL `42883` (404). PostgREST's unknown literal can
  report `42725` instead when several candidates match (`text <@ unknown`).

## Out of scope

Embeds, vertical filtering, `order` / `limit` / `offset`, preferences,
object and CSV media types, and writes. Those return HTTP 501 via
`megabase_core::MegabaseNotImplemented`.

## Judge cases

`judge/cases/rest.toml`: `rest.filter.eq` (`GET /rest/v1/todos?done=eq.true`).
`rest.filter.gt-lt` (`priority=gt.1&priority=lt.3`) covers `gt` and `lt`.
`rest.filter.in` also sends `order=id`, so it stays 501 for
`rest:query-param:order` until that unit is served.
`rest.filter.unknown-operator` is the `PGRST100` shape for `id=nope.1`.
Issue 29 cases live on the judge review branch: `rest.filter.ov`,
`rest.filter.sl`, `rest.filter.sr`, `rest.filter.nxr`, `rest.filter.nxl`
against `public.spans.during`, and `rest.filter.plfts`,
`rest.filter.phfts`, `rest.filter.wfts` against `public.todos.title`.
Each request returns one row so unordered `json_agg` stays stable.
