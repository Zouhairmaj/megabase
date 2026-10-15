# REST: ordering (issue 36)

Status: draft
Unit ids (`coverage/units.json`): `rest:order-modifier:asc`,
`rest:order-modifier:desc`, `rest:order-modifier:nullsfirst`,
`rest:order-modifier:nullslast`
Level: 1

Modifiers of the `order=` query parameter on `GET /rest/v1/{relation}`.
Clients write `order=name.desc.nullslast` to sort rows and place NULLs.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| `nullsfirst`, `nullslast` | [`QueryParams.hs:846`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L846) |
| `asc`, `desc` | [`QueryParams.hs:851`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L851) |

Pins: `postgrest v16.4` (`vendor.toml`).

## Inputs

`order=<term>[,<term>...]`; a term is `column[.asc|.desc][.nullsfirst|.nullslast]`.
The direction comes before the nulls modifier; either may be omitted.

## Outputs

Rows sorted by `ORDER BY "col" [ASC|DESC] [NULLS FIRST|NULLS LAST]`. With no
modifier the SQL has none, so PostgreSQL defaults apply.

## Errors

A bad modifier is a `400` with `PGRST100` and the parser message
`"failed to parse order (...)" (line 1, column N)`.

## Edge cases

Modifiers combine per term; terms keep request order.

## Out of scope

Related-order terms (`rest:query-param:order`). Embedded ordering
(`books.order=title.desc`) is in [`embedding.md`](embedding.md).

## Judge cases

Separate `review/issue-<n>-cases` PR.
