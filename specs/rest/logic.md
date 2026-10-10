# REST: logic (issue 31)

Status: draft
Unit ids (`coverage/units.json`): `rest:logic-operator:and`,
`rest:logic-operator:or`, `rest:logic-operator:not`
Level: 1

Logical operator trees on `GET /rest/v1/{relation}`: `and=(a,b)`, `or=(a,b)`,
and the negated forms `not.and=(...)` / `not.or=(...)`. Clients use them to
combine horizontal filters with boolean logic, for example
`or=(id.eq.1,and(done.eq.true,priority.gte.1))`.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| `and` / `or` / `not` query keys, `pLogicPath` | [`QueryParams.hs:308`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L308) |
| `pLogicTree` (nested expressions, leaf `field.op.value`) | [`QueryParams.hs:883`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L883) |
| `not` prefix | [`QueryParams.hs:892`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L892) |
| `and` / `or` operator tokens | [`QueryParams.hs:897`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/ApiRequest/QueryParams.hs#L897) |

Pins: `postgrest v16.4` (`vendor.toml`). Ported code carries the credit header
required by AGENTS.md.

## Inputs

Query keys `and`, `or`, `not.and`, `not.or`; the value is a parenthesised,
comma-separated list of leaf filters (`column.operator.value`, any served
filter operator, optionally `not.`-prefixed) or nested `and(...)`/`or(...)`
(optionally `not.`-prefixed). Several root trees are ANDed together and with
plain column filters.

## Outputs

Rows matching the boolean expression. SQL joins children with `AND` / `OR`
inside parentheses, `NOT (...)` for a negated tree, with every value a bound
parameter in left-to-right order.

## Errors

| When | Status | Code / message (upstream) |
|---|---|---|
| Malformed tree (`or=()`, missing parentheses, bad leaf) | 400 | `PGRST100`, Parsec message |
| Logic key on a non-embedded resource path | 400 | `PGRST108` |

## Edge cases

Whitespace around `(`, `,`, `)` is skipped. `not` anywhere in the key path
negates the tree. Quoted values may contain `,` and `)`.

## Out of scope

JSON-path leaves (`data->a.eq.1`) and embedded-resource logic stay 501
(routed through the read unit).

## Judge cases

Not yet written. Judge cases are protected paths and go in a separate
`review/issue-31-cases` PR.
