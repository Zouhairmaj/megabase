# Auth: custom access token hook (issue 339)

Status: complete for `pg-functions://`; HTTP(S) hooks are HTTP 501
Unit ids (`coverage/units.json`): `auth:hook:custom_access_token`
Level: 1

When the hook is enabled, every access token Auth issues is signed from the
claims a Postgres function returns. Apps use it for role-based access
control: the function adds a claim such as `user_role`, and RLS policies
read it from `auth.jwt()`.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Config fields | [`internal/conf/configuration.go:937`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/conf/configuration.go#L937) |
| URI validation | [`internal/conf/configuration.go:985`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/conf/configuration.go#L985) |
| Function name (`"schema"."fn"`) | [`internal/conf/configuration.go:1040`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/conf/configuration.go#L1040) |
| Hook call in token generation | [`internal/tokens/service.go:735`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/tokens/service.go#L735) |
| Claims schema | [`internal/tokens/service.go:1006`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/tokens/service.go#L1006), [`:1033`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/tokens/service.go#L1033) |
| Input and output types | [`internal/hooks/v0hooks/v0hooks.go:168`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/hooks/v0hooks/v0hooks.go#L168), [`:193`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/hooks/v0hooks/v0hooks.go#L193) |
| Dispatch by URI prefix, error wrap | [`internal/hooks/v0hooks/manager.go:167`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/hooks/v0hooks/manager.go#L167), [`:191`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/hooks/v0hooks/manager.go#L191) |
| Postgres call | [`internal/hooks/hookspgfunc/hookspgfunc.go:73`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/hooks/hookspgfunc/hookspgfunc.go#L73) |
| Hook `error` object | [`internal/hooks/hookserrors/hookserrors.go:56`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/hooks/hookserrors/hookserrors.go#L56) |
| Public Postgres errors | [`internal/utilities/postgres.go:23`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/utilities/postgres.go#L23), [`internal/api/errors.go:193`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/errors.go#L193) |

Pins: Auth `v2.197.0` (MIT). The reference stack leaves the hook off
(`vendor/supabase/docker/docker-compose.yml` has the variables commented
out).

## Inputs

| Env | Meaning |
|---|---|
| `GOTRUE_HOOK_CUSTOM_ACCESS_TOKEN_ENABLED` | Go `strconv.ParseBool`; default false |
| `GOTRUE_HOOK_CUSTOM_ACCESS_TOKEN_URI` | for example `pg-functions://postgres/public/custom_access_token_hook` |

A non-empty URI is validated at startup even when the hook is disabled.
An invalid URI stops the process:

- `pg-functions` (scheme matched case-insensitively): the path needs at
  least two segments and each of the first two matches
  `^[a-zA-Z_][a-zA-Z0-9_]{0,62}$`. They become the function
  `"<schema>"."<function>"`.
- `http`: only `localhost`, `127.0.0.1`, `::1`, `host.docker.internal`.
- `https`: accepted.
- Anything else is rejected.

The dispatcher matches the prefix `pg-functions:` exactly (case-sensitive).
A URI that passed validation with another case, such as `PG-FUNCTIONS://…`,
fails at token time with 500 `error generating jwt token`. An `http:` or
`https:` hook returns HTTP 501 `MEGABASE_NOT_IMPLEMENTED` with unit
`auth:hook:custom_access_token`. `GOTRUE_HOOK_CUSTOM_ACCESS_TOKEN_SECRETS`
only applies to HTTP hooks and is not read.

## When the hook runs

Every session issuance, inside the transaction that writes the session:

| Route | `authentication_method` |
|---|---|
| `POST /token?grant_type=password` | `password` |
| `POST /token?grant_type=refresh_token` (rotation and reuse of a parent within the interval) | `token_refresh` |
| `POST /signup` with autoconfirm | `password` |
| `GET` / `POST /verify` that returns a session | `otp` |

## Call

In the session transaction, as the last statements before commit:

1. `set local statement_timeout TO '2000'`.
2. GoTrue connects as `supabase_auth_admin`. Megabase may connect as a
   superuser, so it runs `set local role supabase_auth_admin` when that role
   exists and the current user may assume it.
3. `select ("<schema>"."<function>"($1::jsonb))::text` with the event.
4. `reset role` and `set local statement_timeout TO default`.

Any database error in these steps rolls the transaction back, so the
session, refresh token, and `last_sign_in_at` update are not written.

Event (`CustomAccessTokenInput`):

```json
{
  "metadata": {
    "uuid": "<v4>",
    "time": "<RFC 3339, nanoseconds, trailing zeros trimmed>",
    "name": "customize-access-token",
    "ip_address": "<first parseable X-Forwarded-For entry>"
  },
  "user_id": "<uuid>",
  "claims": { "...": "the claims the token would carry without the hook" },
  "authentication_method": "password"
}
```

`claims` has `iss` (when set), `sub`, `aud` (a string), `exp`, `iat`,
`email`, `phone`, `app_metadata`, `user_metadata`, `role`, `aal`, `amr`,
`session_id`, and `is_anonymous`, the same as the standard token.

## Output

Checked in this order:

1. **Hook error.** A top-level `error` object with a non-empty `message`
   (keys matched case-insensitively, like Go `encoding/json`) fails with
   status `http_code`, or 500 when it is 0 or absent. A non-integer
   `http_code`, a non-string `message`, or a non-object `error` is ignored.
2. **Shape.** SQL `NULL` keeps the claims nil. JSON that is not an object
   is 500 `Error unmarshaling JSON output.` (`error_code`
   `unexpected_failure`). A top-level JSON `null`, or an object without the
   exact key `claims`, is 500 `output claims field is missing`. `claims`
   that is neither an object nor `null` is 500 `Error unmarshaling JSON
   output.`.
3. **Schema** (`MinimumViableTokenSchema`). Required: `aud`, `exp`, `iat`,
   `sub`, `email`, `phone`, `role`, `aal`, `session_id`, `is_anonymous`.
   Types: `aud` string or array; `exp`, `iat`, `nbf` integer; `jti`, `iss`,
   `sub`, `email`, `phone`, `role`, `aal`, `session_id`, `client_id`
   string; `app_metadata`, `user_metadata` object; `amr` array whose items
   are strings or objects. A failure is 500 with message
   `output claims do not conform to the expected schema: \n` followed by one
   `- <field>: <description>\n` line per gojsonschema error: required
   errors first (`(root): aud is required`), then type errors
   (`exp: Invalid type. Expected: integer, given: string`;
   multi-type expectations print as `[string,array]`; nil claims print
   `(root): Invalid type. Expected: object, given: null`). A bad `amr` item
   adds `amr.<i>: Must validate at least one schema (anyOf)` followed by
   `amr.<i>: Invalid type. Expected: string, given: <type>`.
   gojsonschema walks properties in Go map order, so GoTrue's order of
   several type errors is not stable; Megabase uses schema order.

The token is then signed from the returned claims exactly
(`jwt.MapClaims`). The response `expires_in` / `expires_at` still come from
`GOTRUE_JWT_EXP`, not from a changed `exp`.

## Errors

| Cause | Status | Body |
|---|---|---|
| Hook `error` object | `http_code` or 500 | `{"code":<status>,"error_code":"unexpected_failure" (500) or "unknown","msg":<message>}`, no `x-sb-error-code` header |
| Schema failure, missing `claims` | 500 | same shape, `error_code` `unexpected_failure`, no header |
| Not JSON object / bad `claims` type | 500 | `msg` `Error unmarshaling JSON output.`, header `x-sb-error-code: unexpected_failure` |
| Function raised a public Postgres error (`P0001`, `23000`, `23001`, `23502`, `23503`, `23505`, `23514`, `23P01` → 500; `PTnnn` → `nnn`) | that status | `{"code":"<sqlstate>","message":…,"hint"?:…,"detail"?:…}`, header `unexpected_failure` |
| Any other database error (missing function, permission, timeout) | 500 | `msg` `Error running hook URI: <uri>`, `error_code` `unexpected_failure` |

`GET /verify` redirects with these as the fragment (`error`,
`error_code`, `error_description`) like any other verify failure. The
redirect uses GoTrue's raw `ErrorCode`, so hook-object and schema failures
send an empty `error_code`, and a public Postgres error redirects as the
wrapping `Error running hook URI` failure.

## Known differences

- **Signup rollback.** GoTrue creates the autoconfirmed user in one
  transaction and issues the session in a second one, so a hook failure
  leaves the user in place. Megabase writes both in one transaction, so the
  user is rolled back as well. The HTTP response is the same.
- **`ip_address`.** Megabase handlers do not see the socket address, so
  GoTrue's `RemoteAddr` fallback is missing; without `X-Forwarded-For` the
  field is omitted. `Sb-Forwarded-For` is not read (off in the reference
  stack).
- **`error_id`.** GoTrue adds the request id to 5xx bodies; no Megabase
  Auth error does yet.
- **In-memory backend.** Tests use a Rust closure as the hook function.
  Memory writes are not transactional, so a failure keeps the session.

## Tests

`crates/megabase-auth/src/hooks.rs`: config parsing and validation, output
parsing, every error shape, schema messages, IP and time formatting, and
HTTP tests on the memory backend for signup, password, refresh, verify, a
hook error, a schema failure, a missing function, and an HTTPS hook (501).
The judge overlay enables this hook
(`pg-functions://postgres/public/custom_access_token_hook`) on Auth and
on Megabase. Cases live in `judge/cases/auth.toml`.
