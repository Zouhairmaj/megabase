# Auth: token (issue 17)

Status: complete
Unit ids (`coverage/units.json`): `auth:route:POST /auth/v1/token`,
`auth:grant-type:password`, `auth:grant-type:refresh_token`
Level: 1

`POST /auth/v1/token` exchanges a password or a legacy refresh token for a
session. Clients send `grant_type` on the query string and a JSON body.
The password grant checks the stored bcrypt hash and returns an access
token. The refresh-token grant rotates the legacy 12-character token.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Route `POST /token` | [`vendor/auth/internal/api/api.go:268`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L268) |
| Grant dispatch | [`vendor/auth/internal/api/token.go:40`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/token.go#L40) |
| Password grant | [`vendor/auth/internal/api/token.go:71`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/token.go#L71) |
| Password strength on login | [`vendor/auth/internal/api/password.go:28`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/password.go#L28) |
| Refresh grant and token shape | [`vendor/auth/internal/api/token_refresh.go:20`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/token_refresh.go#L20) |
| Rotation, reuse, and errors | [`vendor/auth/internal/tokens/service.go:194`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/tokens/service.go#L194) |
| Email and phone lookup | [`vendor/auth/internal/models/user.go:699`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/user.go#L699) |
| bcrypt compare | [`vendor/auth/internal/models/user.go:467`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/user.go#L467) |
| Swap and family revoke | [`vendor/auth/internal/models/refresh_token.go:67`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/refresh_token.go#L67) |
| Phone normalization | [`vendor/auth/internal/api/phone.go:41`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/phone.go#L41) |

Pins: Auth `v2.197.0` (`4eee58f296d9698a1c2c0ae14d7a0b379c7622d3`, MIT).
The reference stack does not set
`GOTRUE_SECURITY_REFRESH_TOKEN_ALGORITHM_VERSION`, so the algorithm is the
Go zero value `0` (legacy 12-character tokens). JWT expiry is 3600 seconds
(`GOTRUE_JWT_EXP` unset). Audience defaults to `authenticated`.

## Inputs

`POST /auth/v1/token`. `grant_type` is the query parameter (Go
`Request.FormValue` for a JSON body). The body is JSON, at most `1 << 20`
bytes. Unknown JSON fields are ignored. A JSON `null` for `email`, `phone`,
`password`, or `refresh_token` decodes as `""`, matching Go `encoding/json`
on a `string` field.

Password body: `email`, `phone`, `password`. Audience is `X-JWT-AUD` when
that header is non-empty, otherwise `GOTRUE_JWT_AUD`.

Refresh body: `refresh_token`.

Lookup SQL uses parameters only. Email match is
`instance_id` nil, `lower(email)`, `aud`, `is_sso_user = false`. Phone
match is the same with `phone` after `formatPhoneNumber` (one leading `+`
removed, then spaces removed).

## Outputs

Success is 200 `application/json` (no charset): `access_token`,
`token_type` `bearer`, `expires_in` (the configured JWT lifetime, 3600 by
default, taken from one clock), `expires_at`, `refresh_token`, `user`.
The access token is HS256 (`typ` `JWT`) with `aal` `aal1` and `amr`
`[{"method":"password","timestamp":...}]`. Password login writes that AMR
on the new session. Refresh keeps the session's existing AMR timestamp.
The user object is reloaded, so `confirmed_at` is present and the email
identity still has `email_verified: false` (the signup response had
flipped that flag only in memory).

Headers (not compared by the judge unless a case opts in):
`sb-auth-user-id`, `sb-auth-session-id`, `sb-auth-refresh-token-prefix`
(first five characters). A real rotation also sets
`sb-auth-refresh-token-reuse: false`.

Password success updates `auth.users.last_sign_in_at` only. It inserts a
new `auth.sessions` row (`aal1`), a legacy refresh token (`parent` null),
an `auth.mfa_amr_claims` row (`password`), and a `login` audit entry with
`traits.provider` `email` or `phone`.

Refresh success revokes the presented token, inserts a new 12-character
token with `parent` set to the old token and the same `session_id`, writes
`token_revoked` and `token_refreshed` audit rows, and sets
`auth.sessions.refreshed_at`. It does not change `last_sign_in_at`.
If the presented token is already revoked and is the parent of the current
active token, the response returns that active token and does not mint
another one.

A password that matches and is shorter than `GOTRUE_PASSWORD_MIN_LENGTH`
still returns 200, with `weak_password: {message, reasons: ["length"]}`.
A strong password still includes `weak_password: null`, because Go stores a
nil `*WeakPasswordError` in an `interface{}`. Refresh omits the field.
Required-character sets and HIBP are unset on the reference stack, so they
add no reasons.

## Errors

Legacy API shape: `code` is the HTTP status integer, `error_code`, `msg`,
and header `x-sb-error-code`.

| When | Status | Code / message |
|---|---|---|
| `grant_type` missing or unknown | 400 | `invalid_credentials` / `unsupported_grant_type` |
| Email and phone both set | 400 | `validation_failed` / `Only an email address or phone number should be provided on login.` |
| Neither email nor phone | 400 | `validation_failed` / `missing email or phone` |
| Email provider disabled | 422 | `email_provider_disabled` / `Email logins are disabled` |
| Phone provider disabled | 422 | `phone_provider_disabled` / `Phone logins are disabled` |
| Unknown user, empty password hash, or wrong password | 400 | `invalid_credentials` / `Invalid login credentials` |
| `banned_until` in the future (checked before the password compare) | 400 | `user_banned` / `User is banned` |
| Password matches and is longer than 72 bytes | 400 | `validation_failed` / `Password cannot be longer than 72 characters` |
| Email not confirmed | 400 | `email_not_confirmed` / `Email not confirmed` |
| Phone not confirmed | 400 | `phone_not_confirmed` / `Phone not confirmed` |
| Refresh token shorter than 12, not `^[a-z0-9]{12}$`, or a longer token that fails the version-2 checksum | 400 | `validation_failed` / `Refresh token is not valid` |
| Refresh token not in the store, or a version-2 token whose session has no HMAC key | 400 | `refresh_token_not_found` / `Invalid Refresh Token: Refresh Token Not Found` |
| Refresh token whose session row is gone | 400 | `session_not_found` / `Invalid Refresh Token: No Valid Session Found` |
| Banned user on refresh | 400 | `user_banned` / `Invalid Refresh Token: User Banned` |
| Revoked refresh token that is not the parent of the active token (reuse interval 0, rotation on; the family is revoked) | 400 | `refresh_token_already_used` / `Invalid Refresh Token: Already Used` |
| Body is not JSON, or larger than 1 MiB | 400 / 413 | same codes as signup (`bad_json`, `request_entity_too_large`) |
| No database | 500 | `unexpected_failure` / `Database error querying schema` |
| No `JWT_SECRET` | 500 | `unexpected_failure` / `Server lacks JWT secret` |
| `grant_type` `id_token`, `pkce`, or `web3` | 501 | `MEGABASE_NOT_IMPLEMENTED` with that unit id |

## Edge cases

- Email is lowercased before lookup. Phone keeps digits after the `+` and
  spaces are stripped.
- A banned user gets `user_banned` even when the password is wrong. A user
  with an empty password hash gets `invalid_credentials` even when banned.
- bcrypt comparison uses the first 72 bytes, matching Go. The 72-byte limit
  is then applied to the original password, so a longer password that
  matches the prefix is `validation_failed`, not a session.
- The reference stack does not enable database encryption, so a bcrypt cost
  other than 10 is not rewritten on login. GoTrue only persists that
  rewrite when encryption asks for it.
- `expires_in` is the configured lifetime. It is not recomputed from a
  second clock read, which in GoTrue can yield 3599 across a second boundary.
- Version-2 refresh tokens are not issued. A structurally valid one is
  looked up as a legacy token string, misses, and returns
  `refresh_token_not_found`, which is what GoTrue returns when the session
  has no HMAC key.
- Presenting the parent of the active refresh token returns the active
  token. Presenting an older revoked token revokes the family and returns
  `refresh_token_already_used`.

## Out of scope

`id_token`, `pkce`, and `web3` return 501 with their unit ids. Captcha,
rate limits, password-verification hooks, HIBP, session timebox,
single-session-per-user, and refresh-token algorithm version 2 issuance
are not served. The reference stack leaves those flags off.

## Judge cases

Already in `judge/cases/auth.toml` (not edited here): `auth.token.password`,
`auth.token.wrong-password`, `auth.token.refresh`. Each case signs up
first, so it also depends on `POST /signup` from issue 15.
