# REST: prefer

Status: partial (10/16 served)
Unit ids: `rest:prefer:` + `count=exact|planned|estimated`,
`handling=strict|lenient`, `max-affected=*`, `missing=default|null`,
`resolution=merge-duplicates|ignore-duplicates`,
`return=minimal|representation|headers-only`, `timezone=*`, `tx=commit|rollback`
Level: 1

The `Prefer` request header and its `Preference-Applied` response. Template:
[`specs/_template.md`](../_template.md).

## Upstream

- `vendor/postgrest/src/library/PostgREST/ApiRequest/Preferences.hs`
  (`fromHeaders` line 130, `prefAppliedHeader` line 173)
- `Plan.hs` 214-230 (strict check), `MainTx.hs` 245 (`max-affected`),
  `Query/QueryBuilder.hs` 128 (`ON CONFLICT`), `Query/SqlFragment.hs` 344
  (`missing=default`), `RangeQuery.hs` 96 (`Content-Range` total).
- Pin: `postgrest v16.4` (`vendor.toml`).

## Parsing

Every `Prefer` header is split on `,` and trimmed. Matching is exact and
case-sensitive. For each kind the first recognised value in order wins, so
`tx=commit, tx=rollback` is commit. `max-affected=<n>` takes the first
occurrence and ignores a non-integer value. A token that is neither a known
value nor a `timezone=` / `max-affected=` prefix is *invalid*.

## Units served in this slice (resource routes only)

| Unit | Behavior |
|---|---|
| `handling=strict` | An invalid token gives `400` `PGRST122`, message `Invalid preferences given with handling=strict`, details `Invalid preferences: a, b`. |
| `handling=lenient` | Invalid tokens are ignored. Without `handling` they are ignored too. |
| `count=exact` | `GET`/`HEAD`: `Content-Range` carries the filtered row count, e.g. `0-1/5`. `206` when the page is shorter than the total, else `200`. |
| `count=planned` | Same, with the planner's `Plan Rows` from `EXPLAIN (FORMAT JSON)`. |
| `count=estimated` | Exact up to `db-max-rows` (1000, as in the reference stack); above that, the planner's estimate. |
| `max-affected=*` | `PATCH`/`DELETE` under `handling=strict`: more affected rows than `n` rolls the write back and gives `400` `PGRST124`, details `The query affects N rows`. Without strict it is ignored. |
| `missing=default` | `POST`/`PATCH`: payload keys that are absent take the column default (`pg_get_expr`) instead of `NULL`. |
| `missing=null` | The default behavior: absent keys are `NULL`. |
| `resolution=merge-duplicates` | `POST` with `on_conflict` (or the primary key): `ON CONFLICT (...) DO UPDATE SET col = EXCLUDED.col`. |
| `resolution=ignore-duplicates` | Same target, `DO NOTHING`. |

`Preference-Applied` echoes every parsed preference on a successful resource
response, like `prefAppliedHeader`, in order: resolution (writes: only POST
with a conflict target), missing, count (reads), handling, `max-affected`
(under strict).

## Edge cases

- Offset past a counted total: `416`, `PGRST103`, details
  `An offset of N was requested, but there are only M rows.`
- Zero total: `Content-Range: */0`.
- `count` with an aggregate `select`, and `count` on writes, are `501` with
  the count unit id (the total is not the plain row count).

## Out of scope (still 501)

`return=*`, `tx=*`, `timezone=*` (any route), every preference on
`/rest/v1/rpc/*`.

`PUT` ignores `resolution` and always merges, like upstream.

## Judge cases

Cases for these ids belong in a separate `review/issue-<n>-cases` PR
(AGENTS.md step 8). This slice has unit tests only; conformance is not
measured.
