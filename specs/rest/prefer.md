# REST: prefer (issue 39)

Status: draft
Unit ids: `rest:prefer:` + `count=exact|planned|estimated`,
`handling=strict|lenient`, `max-affected=*`, `missing=default|null`,
`resolution=merge-duplicates|ignore-duplicates`,
`return=minimal|representation|headers-only`, `timezone=*`, `tx=commit|rollback`
Level: 1

The `Prefer` request header and its `Preference-Applied` response. Resource
routes serve every unit. RPC routes serve `timezone=*` and ignore `return=*`
and `tx=*`; the other preferences answer `MEGABASE_NOT_IMPLEMENTED` there
(`Prefer::rpc_unserved`).

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Parsing, first occurrence wins, case-sensitive | `vendor/postgrest/src/library/PostgREST/ApiRequest/Preferences.hs:106-148` |
| `Preference-Applied` order and tokens | `Preferences.hs:151-192` |
| Applied set per plan (`return` only for mutations) | `vendor/postgrest/src/library/PostgREST/Response.hs:285-306` |
| Status, headers and body per mutation | `Response.hs:92-200` |
| `Location` for `headers-only` on a create | `Query/Statements.hs:64-76`, `Query/SqlFragment.hs:257-268` |
| `timezone` as `set_config` | `Query/PreQuery.hs:57` |
| `tx` honoured only with `db-tx-end` `*-allow-override` | `ApiRequest.hs:141`, `MainTx.hs:254-261` |
| Strict check, `max-affected`, `ON CONFLICT`, `missing=default`, count total | `Plan.hs` 214-230, `MainTx.hs` 245, `Query/QueryBuilder.hs` 128, `Query/SqlFragment.hs` 344, `RangeQuery.hs` 96 |

Pin: `postgrest v16.4` (`vendor.toml`).

## Parsing

Every `Prefer` header is split on `,` and trimmed. Matching is exact and
case-sensitive. For each kind the first recognised value in order wins across
all headers, so `tx=commit, tx=rollback` is commit and the first `return=*`
and the first `timezone=<tz>` win. `max-affected=<n>` takes the first
occurrence and ignores a non-integer value. A token that is neither a known
value nor a `timezone=` / `max-affected=` prefix is *invalid*.

## Outputs

`return=` on `POST`, `PATCH`, `PUT`, `DELETE`:

| Preference | Create | Update | Delete | Upsert (`PUT`) |
|---|---|---|---|---|
| none, `minimal` | `201`, empty | `204` | `204` | `204` |
| `headers-only` | `201`, empty, `Location` | `204` | `204` | `204` |
| `representation` | `201`, rows | `200`, rows | `200`, rows | `201` if inserted, else `200`; rows |

- The body honours `select=` and `order=`. Aggregates stay `501`.
- `Content-Range`: create and delete `*/*`, update `0-(n-1)/*`, upsert none.
- `Location` (create, `headers-only`): only when exactly one row was inserted
  and the table has a primary key: `/{relation}?{pk}=eq.{value}`, pairs
  percent-encoded, in primary-key constraint order.
- `return=` is ignored on reads and RPC, and is not echoed there.
- The default and `minimal` use `RETURNING 1`, so a write-only role works.

`timezone=<tz>` on every request (resource, RPC, root): `set_config('timezone',
tz, true)` in the request transaction, so `timestamptz` output uses it.

`tx=commit` and `tx=rollback`: the pinned stack leaves `PGRST_DB_TX_END` at
`commit` (no override), so PostgREST ignores the token. It neither rolls back
nor echoes it. Megabase does the same.

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

`Preference-Applied` (successful responses only) echoes every parsed
preference, like `prefAppliedHeader`, in order: resolution (writes: only POST
with a conflict target), missing, return (mutations), count (reads), handling,
`timezone`, `max-affected` (under strict). Tokens are joined by `, `.

## Errors

| When | Status | Code / message (upstream) |
|---|---|---|
| Invalid timezone name | database error | PostgreSQL `22023`, mapped by `query_failure` |
| Any preference other than `return`, `tx`, `timezone` on RPC | `501` | `MEGABASE_NOT_IMPLEMENTED` with the unit id |

## Edge cases

- Offset past a counted total: `416`, `PGRST103`, details
  `An offset of N was requested, but there are only M rows.`
- Zero total: `Content-Range: */0`.
- `count` with an aggregate `select`, and `count` on writes, are `501` with
  the count unit id (the total is not the plain row count).
- `return=representation` needs `SELECT` on the returned columns.
- Empty `PATCH` body keys: no row changes; `representation` answers `200 []`.
- `PUT` ignores `resolution` and always merges, like upstream.

## Judge cases

None yet; judge cases land in a separate `review/` PR (AGENTS.md step 8).
Conformance is not measured.
