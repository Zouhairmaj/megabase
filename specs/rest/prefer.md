# REST: prefer (issue 39)

Status: draft
Unit ids: `rest:prefer:` + `count=exact|planned|estimated`,
`handling=strict|lenient`, `max-affected=*`, `missing=default|null`,
`resolution=merge-duplicates|ignore-duplicates`,
`return=minimal|representation|headers-only`, `timezone=*`, `tx=commit|rollback`
Level: 1

The `Prefer` request header and its `Preference-Applied` response. This
revision serves `return=*`, `timezone=*` and `tx=*` (issue 39). The rest
(`count`, `handling`, `max-affected`, `missing`, `resolution`) answers
`MEGABASE_NOT_IMPLEMENTED` from `prefer::unserved`.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Parsing, first occurrence wins, case-sensitive | `vendor/postgrest/src/library/PostgREST/ApiRequest/Preferences.hs:106-148` |
| `Preference-Applied` order and tokens | `Preferences.hs:151-170` |
| Applied set per plan (`return` only for mutations) | `vendor/postgrest/src/library/PostgREST/Response.hs:285-306` |
| Status, headers and body per mutation | `Response.hs:92-200` |
| `Location` for `headers-only` on a create | `Query/Statements.hs:64-76`, `Query/SqlFragment.hs:257-268` |
| `timezone` as `set_config` | `Query/PreQuery.hs:57` |
| `tx` honoured only with `db-tx-end` `*-allow-override` | `ApiRequest.hs:141`, `MainTx.hs:254-261` |

Pin: `postgrest v16.4` (`vendor.toml`).

## Inputs

`Prefer` headers, any number, each a comma-separated list. Tokens are trimmed.
Matching is exact (case-sensitive). The first `return=*` and the first
`timezone=<tz>` win across all headers. Unknown tokens are ignored.

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
  percent-encoded, in column order.
- `return=` is ignored on reads and RPC, and is not echoed there.

`timezone=<tz>` on every request: `set_config('timezone', tz, true)` in the
request transaction, so `timestamptz` output uses it.

`Preference-Applied` (successful responses only): `return=…` (mutations) then
`timezone=…`, joined by `, `.

`tx=commit` and `tx=rollback`: the pinned stack leaves `PGRST_DB_TX_END` at
`commit` (no override), so PostgREST ignores the token. It neither rolls back
nor echoes it. Megabase does the same.

## Errors

| When | Status | Code / message (upstream) |
|---|---|---|
| Invalid timezone name | database error | PostgreSQL `22023`, mapped by `query_failure` |
| `count`, `handling`, `max-affected`, `missing`, `resolution` | `501` | `MEGABASE_NOT_IMPLEMENTED` with the unit id |

## Edge cases

- `return=representation` needs `SELECT` on the returned columns.
- Empty `PATCH` body keys: no row changes; `representation` answers `200 []`.
- The unit ids above are not served for `GET`/`HEAD` beyond `timezone`.

## Out of scope

`count`, `handling`, `max-affected`, `missing`, `resolution` (the epic's other issue).

## Judge cases

None yet; judge cases land in a separate `review/` PR (AGENTS.md step 8).
