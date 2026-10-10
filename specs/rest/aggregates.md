# REST: aggregates (issue 23)

Status: draft
Unit ids: `rest:aggregate:avg`, `rest:aggregate:count`, `rest:aggregate:max`, `rest:aggregate:min`, `rest:aggregate:sum`
Level: 1

Aggregate functions in the `select=` query parameter of a table read
(`GET` / `HEAD /rest/v1/{relation}`). Clients use them for totals and
group-by summaries without an RPC.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Select field grammar: `alias:field::cast.agg()::aggCast` | `vendor/postgrest/src/library/PostgREST/ApiRequest/QueryParams.hs:577-616` |
| `count()` form (no field, optional alias and cast) | `QueryParams.hs:583-590` |
| Function names `sum avg count max min` | `QueryParams.hs:606-616` |
| SQL: `AGG(CAST(field AS cast))`, then `CAST(... AS aggCast)` | `vendor/postgrest/src/library/PostgREST/Query/SqlFragment.hs:305-321` |
| `GROUP BY` over the non-aggregated fields | `SqlFragment.hs:493-522` |
| Disabled unless `db-aggregates-enabled` | `vendor/postgrest/src/library/PostgREST/Plan.hs:895-898` |
| `PGRST123` text | `vendor/postgrest/src/library/PostgREST/Error.hs:169,195` |
| No implicit alias for an aggregate | `Plan.hs:462-480` |

Pin: `postgrest v16.4` (`vendor.toml`).

## Inputs

`select=` items: `field.sum()`, `field.avg()`, `field.max()`, `field.min()`,
`field.count()`, and `count()`. A field may have a JSON path, a `::cast`
before the function and a `::cast` after it. `alias:` is optional.
`count()` is `COUNT(table.*)`.

## Outputs

One row per group. Non-aggregated select fields form the `GROUP BY`
(by output alias when there is one); with none, the result is one row.
`*` next to an aggregate expands to the table columns. An aggregate has no
implicit alias, so PostgreSQL names the column after the function
(`sum`, `count`, ...) unless the client gave an alias.

## Errors

| When | Status | Code / message (upstream) |
|---|---|---|
| Aggregate in `select` and aggregates are disabled (the default) | 400 | `PGRST123` "Use of aggregate functions is not allowed" |
| Relation does not exist | 404 | table lookup runs before the aggregate check |
| Text after the function or cast | 400 | `PGRST100` parse error |

The pinned Supabase compose file does not set `PGRST_DB_AGGREGATES_ENABLED`,
so the reference stack answers `PGRST123`. Megabase reads the same variable
(`Config::db_aggregates_enabled`, default `false`).

## Edge cases

- Aggregates in embedded resources and spreads stay with the embed units
  (still 501).
- `ORDER BY` on a non-grouped column fails in PostgreSQL, as upstream.
- Filters apply before grouping (`WHERE` precedes `GROUP BY`).

## Out of scope

`db-max-rows`, `Prefer: count`, and aggregates inside embeds.

## Judge cases

Reference behavior with the default env is `PGRST123`; cases for the enabled
path need a reference stack with `PGRST_DB_AGGREGATES_ENABLED=true`, which is a
`review/*` change to the judge compose override.
