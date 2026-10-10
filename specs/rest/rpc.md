# REST: rpc (issue 45)

Status: complete
Unit ids (`coverage/units.json`): `rest:route:GET /rest/v1/rpc/{function}`, `rest:route:HEAD /rest/v1/rpc/{function}`, `rest:route:OPTIONS /rest/v1/rpc/{function}`, `rest:route:POST /rest/v1/rpc/{function}`
Level: 1

`GET`, `HEAD`, `OPTIONS`, and `POST` on `/rest/v1/rpc/{function}` call one
stored function. `GET` and `HEAD` pass query arguments. `POST` passes a JSON
object. `OPTIONS` reports the methods the volatility allows. The judge cases
call `public.add_numbers(a integer, b integer)`.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Path `["rpc", name]` and method to action | [`vendor/postgrest/src/library/PostgREST/ApiRequest.hs:147`](https://github.com/PostgREST/postgrest/blob/v16.4/src/library/PostgREST/ApiRequest.hs#L147) |
| Schema profile before the action | [`ApiRequest.hs:180`](https://github.com/PostgREST/postgrest/blob/v16.4/src/library/PostgREST/ApiRequest.hs#L180) |
| `GET`/`HEAD` query keys are arguments, not filters | [`QueryParams.hs:163`](https://github.com/PostgREST/postgrest/blob/v16.4/src/library/PostgREST/ApiRequest/QueryParams.hs#L163) |
| Overload match, unnamed JSON fallback | [`Plan.hs:284`](https://github.com/PostgREST/postgrest/blob/v16.4/src/library/PostgREST/Plan.hs#L284) |
| Which parameters are passed | [`Plan.hs:1157`](https://github.com/PostgREST/postgrest/blob/v16.4/src/library/PostgREST/Plan.hs#L1157) |
| Read-only vs volatile transaction | [`Plan.hs:246`](https://github.com/PostgREST/postgrest/blob/v16.4/src/library/PostgREST/Plan.hs#L246) |
| Call SQL | [`QueryBuilder.hs:192`](https://github.com/PostgREST/postgrest/blob/v16.4/src/library/PostgREST/Query/QueryBuilder.hs#L192) |
| `json_agg` shape, `json_to_record`, page count | [`SqlFragment.hs:230`](https://github.com/PostgREST/postgrest/blob/v16.4/src/library/PostgREST/Query/SqlFragment.hs#L230) |
| Routine row (volatility, scalar, set, composite, void) | [`SchemaCache.hs:442`](https://github.com/PostgREST/postgrest/blob/v16.4/src/library/PostgREST/SchemaCache.hs#L442), [`Routine.hs:109`](https://github.com/PostgREST/postgrest/blob/v16.4/src/library/PostgREST/SchemaCache/Routine.hs#L109) |
| JSON body, empty object, key uniformity | [`Payload.hs:40`](https://github.com/PostgREST/postgrest/blob/v16.4/src/library/PostgREST/ApiRequest/Payload.hs#L40) |
| 204 void, HEAD empty body, OPTIONS `Allow` | [`Response.hs:182`](https://github.com/PostgREST/postgrest/blob/v16.4/src/library/PostgREST/Response.hs#L182) |
| `PGRST101`, `PGRST102`, `PGRST202`, `PGRST203` | [`Error.hs:178`](https://github.com/PostgREST/postgrest/blob/v16.4/src/library/PostgREST/Error.hs#L178) |

Pins: `postgrest v16.4` (`vendor.toml`). Ported code carries the credit header
required by AGENTS.md.

## Inputs

One path segment after `/rest/v1/rpc/`. The segment is percent-decoded with
`+` left as `+`. A deeper path is `PGRST125`.

`Content-Profile` selects the schema for `POST`, `PUT`, `PATCH`, and
`DELETE`. `Accept-Profile` selects it for every other method, including
`GET`, `HEAD`, and `OPTIONS`. Absent means `public`. Any other schema is
`PGRST106`.

`GET` and `HEAD` arguments are non-reserved query pairs. `+` is a space.
A key with no `=` is ignored. `select=*` is the default and is ignored.
`POST` arguments are the keys of one JSON object. A missing `Content-Type`
is `application/json`. An empty body is `{}`.

The bearer token is the same HS256 session as a resource route. No
`Authorization`, or a non-bearer value, is the `anon` role. `OPTIONS` does
not `SET ROLE`.

## Outputs

A scalar or a single composite is one JSON value with `Content-Range:
0-0/*`. A set is a JSON array; an empty set is `[]` with `Content-Range:
*/*`. The status is 200, `Content-Type` is `application/json; charset=utf-8`,
and `Content-Profile` is the schema. `HEAD` sends those headers and an empty
body.

`void` is 204 with `Content-Range: 0-0/*`, an empty body, and no
`Content-Type` or `Content-Profile`.

`GET` and `HEAD` run in a read-only transaction. `POST` of a `stable` or
`immutable` function does too. `POST` of a `volatile` function is read-write.
The catalog lookup uses the pool connection so `SET TRANSACTION` is the first
statement of the call transaction. The transaction then sets `role`,
`request.jwt.claims`, `request.method`, `request.path` (`/rpc/{name}`), and
`search_path`.

`OPTIONS` is 200 with an empty body, `Access-Control-Allow-Origin: *`, and
`Allow: OPTIONS,POST` when the matched function is volatile. Otherwise
`Allow` is `OPTIONS,GET,HEAD,POST`.

`response.status` and `response.headers` set by the function override the
status and add headers that are not already present. An invalid status is
`PGRST112`. An invalid header list is `PGRST111`. Either one rolls the
transaction back.

## Errors

| When | Status | Code / message (upstream) |
|---|---|---|
| Schema not exposed | 406 | `PGRST106` `Invalid schema: {name}` |
| Method other than `GET`, `HEAD`, `POST`, `OPTIONS` | 405 | `PGRST101` `Cannot use the {METHOD} method on RPC` |
| No overload matches the keys | 404 | `PGRST202` `Could not find the function {schema}.{name}…` |
| Two overloads match | 300 | `PGRST203` and the rename hint |
| JSON body is not JSON | 400 | `PGRST102` `Empty or invalid json` |
| JSON array of objects whose keys differ | 400 | `PGRST102` `All object keys must match` |
| `Content-Type` is not a known media type | 400 | `PGRST102` `Content-Type not acceptable: {mime}` |
| Bearer token is bad | 401 | `PGRST301` or `PGRST303`, with `WWW-Authenticate` |
| Bearer sent and `JWT_SECRET` is unset | 500 | `PGRST300` |
| `DATABASE_URL` is unset | 503 | `PGRST000` `DATABASE_URL is unset` |
| `42501` as `anon` / any other role | 401 / 403 | the SQLSTATE |
| Role does not exist (`22023`) | 401 | the SQLSTATE |

`PGRST202` lists the keys in order. The `POST` JSON detail adds `or with a
single unnamed json/jsonb parameter`. The hint is null. There is no fuzzy
name suggestion.

Argument values are bound. The function name and argument names are
`quote_ident` of the `pg_catalog` row. A cast type is spliced only after
`sql_type_name`. `bit` and `character` become `bit varying` and `character
varying`, including the array forms.

## Edge cases

Argument types come from `proallargtypes` when that column is set, and
otherwise from `proargtypes`. Only modes `i`, `b`, and `v` are inputs.
`required` is `idx <= pronargs - pronargdefaults` after `idx` is counted
over those inputs. Optional arguments (the last `pronargdefaults` inputs)
may be omitted. Extra keys do not match. Repeated `GET` keys keep the last
value, except a `VARIADIC` argument, which keeps every value in order. `POST` of exactly one
unnamed `json` or `jsonb` argument receives the raw body when no named
overload matches. `GET` has no unnamed fallback. `OPTIONS` uses the query
keys and does not call the function, so a required argument that is absent
is `PGRST202`.

## Out of scope

Anything not listed above returns 501 `MEGABASE_NOT_IMPLEMENTED`. That
includes `Prefer`, a `select` other than `*`, `columns`, `on_conflict`,
`order`, `limit`, `offset`, `and`, `or`, embedded resources, a query value
that is a filter operator, a `Range` header on `GET`, a `POST` query string,
a JSON array or a JSON value that is not an object, and the media types that
already have their own unit ids (`text/csv`, `application/vnd.pgrst.object+json`,
and the rest of `reject_accept`).

## Judge cases

`judge/cases/rest.toml`: `rest.rpc.get` and `rest.rpc.post`. The hidden
suite calls `add_numbers` with small integers on `GET` and `POST`. `HEAD`
and `OPTIONS` have no case yet.
