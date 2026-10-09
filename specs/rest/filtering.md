# REST: filtering

Status: stub
Unit ids: `rest:filter-operator:` + `eq`, `neq`, `gt`, `gte`, `lt`, `lte`,
`like`, `ilike`, `match`, `imatch`, `in`, `is`, `isdistinct`, `fts`, `plfts`,
`phfts`, `wfts`, `cs`, `cd`, `ov`, `sl`, `sr`, `nxl`, `nxr`, `adj`, `not`,
`any`, `all`
Level: 1

Column filter operators in the query string (issues split 1/3 to 3/3 in the
backlog plan). Template: [`specs/_template.md`](../_template.md).

## Upstream

- `vendor/postgrest/src/library/PostgREST/ApiRequest/QueryParams.hs`
- Pin: `postgrest v16.4` (`vendor.toml`).

## To write

Inputs, outputs, errors, edge cases, out of scope, judge cases.
