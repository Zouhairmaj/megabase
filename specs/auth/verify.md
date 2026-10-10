# Auth: verify (issue 21)

Status: complete
Unit ids (`coverage/units.json`): `auth:route:GET /auth/v1/verify`,
`auth:route:POST /auth/v1/verify`, `auth:verify-type:signup`,
`auth:verify-type:invite`, `auth:verify-type:recovery`,
`auth:verify-type:email_change`
Level: 1

`GET` and `POST /auth/v1/verify` redeem a one-time email token and, when the
token is still valid, issue the same session shape as signup. GET is the
link clients open from email: validation failures are JSON, and every later
failure is a 303 redirect back to the site URL. POST is the API form and
returns JSON, including the session.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Routes `GET`/`POST /verify` | [`internal/api/api.go:271`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L271) |
| `Verify`, `verifyGet`, `verifyPost` | [`internal/api/verify.go:95`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/verify.go#L95) |
| signup / invite | [`internal/api/verify.go:152`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/verify.go#L152) |
| recovery | [`internal/api/verify.go:154`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/verify.go#L154) |
| email_change | [`internal/api/verify.go:156`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/verify.go#L156) |
| Token hash SHA-224 | [`internal/crypto/crypto.go:45`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/crypto/crypto.go#L45) |
| Redirect target | [`internal/utilities/request.go:75`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/utilities/request.go#L75) |

Pins: `auth v2.197.0` (`vendor.toml`, commit `4eee58f`). Ported code carries
the credit header required by AGENTS.md.

## Inputs

Both methods take `type` (`signup`, `invite`, `recovery`, `email_change`).

GET reads `token` and `redirect_to` from the query. `token` is stored as the
token hash (GoTrue copies it onto `token_hash`). The redirect target is the
first allowed URL among `redirect_to`, the `Referer` header, and the site URL.
The site URL is `GOTRUE_SITE_URL`, or `SITE_URL` when that is unset
(`http://localhost:3000` when both are unset). A candidate is allowed when it
matches the site (same scheme and host; loopback may use another port), is a
loopback IP, or matches `GOTRUE_URI_ALLOW_LIST` (`ADDITIONAL_REDIRECT_URLS`
when that list is unset). The host is the authority after any userinfo, so
`http://127.0.0.1:1@evil.com/` is not loopback. An allow-list pattern this
port cannot compile (`[`, `]`, `{`, `}`, or a dangling `\`) is HTTP 501
`GOTRUE_URI_ALLOW_LIST` instead of a guessed site-URL fallback.

POST reads a JSON object: `type`, `token`, `token_hash`, `email`, `phone`,
`redirect_to`. Exactly one of `token` and `token_hash` is set. With `token`,
exactly one of `email` and `phone` is set. Email is validated and lowercased,
then the lookup hash is the lowercase hex SHA-224 of `email + token`. With
only `token_hash`, `email`, `phone`, and `redirect_to` must be empty.

`X-JWT-Aud` overrides the audience for the email-plus-OTP lookup. The hash
lookup does not check audience.

## Outputs

POST success is 200 JSON: `access_token`, `token_type` `bearer`,
`expires_in`, `expires_at`, `refresh_token`, and `user`. The access token's
`amr[0].method` is `otp`. Response headers are `sb-auth-user-id`,
`sb-auth-session-id`, and `sb-auth-refresh-token-prefix` (first five
characters of the legacy 12-character refresh token). The user row is
confirmed when it was not, `last_sign_in_at` is set, and one-time tokens for
that user are cleared. Invite with an empty password and `invited_at` set
stores a generated password. Email change copies `email_change` onto `email`
and the email identity.

The first half of a secure email change (`mailer` autoconfirm off, secure
email change on, confirm status 0, user already has an email) is 200
`{"msg":"Confirmation link accepted. Please proceed to confirm link sent to the other email","code":"200"}`
and does not issue a session. The judge overlay turns autoconfirm on, so that
path does not run there.

GET success and GET failures after validation are 303 See Other. The body is
Go's redirect HTML: `<a href="ESCAPED">See Other</a>.` plus a blank line.
`Location` is the raw URL. An error fragment is
`error`, `error_code`, `error_description`, and empty `sb`, query-escaped and
sorted. 403 is `access_denied`. 400 is `invalid_request`. 500 is
`server_error`. A success fragment carries the session fields plus `type` and
empty `sb`. `SITE_URL` `http://localhost:3000` has an empty path, so the
redirect has no slash before `#`.

Signup and invite write a `user_signedup` / `team` audit. Recovery of an
already confirmed user writes `login` / `account`. Email change writes
`user_modified` / `user`.

## Errors

| When | Status | Code / message (upstream) |
|---|---|---|
| `type` empty | 400 | `validation_failed` / `Verify requires a verification type` |
| GET `token` empty | 400 | `validation_failed` / `Verify requires a token or a token hash` |
| POST token and token_hash both set or both empty | 400 | `validation_failed` / `Verify requires either a token or a token hash` |
| POST token with both or neither of email and phone | 400 | `validation_failed` / `Only an email address or phone number should be provided on verify` |
| POST token_hash plus email, phone, or redirect_to | 400 | `validation_failed` / `Only the token_hash and type should be provided` |
| POST email that fails format checks | 422 | `validation_failed` / `Invalid email format` |
| Hash lookup, unknown type | 400 | `validation_failed` / `Invalid email verification type` (GET redirects this) |
| Hash not found, expired, or `sent_at` missing | 403 | `otp_expired` / `Email link is invalid or has expired` |
| Email OTP not found or not valid | 403 | `otp_expired` / `Token has expired or is invalid` |
| Banned (`banned_until` in the future), checked before expiry | 403 | `user_banned` / `User is banned` |
| No database | 500 | `unexpected_failure` / `Database error finding user from email link` on the hash path, `Database error finding user` on the email path (GET redirects this) |

Expiry is strict: the token is expired only when now is after `sent_at + GOTRUE_MAILER_OTP_EXP` seconds. `0` means 86400.

## Edge cases

A stored hash of `pkce_` plus the SHA-224 still matches an email OTP. GET
`token` values that start with `pkce_` are the PKCE grant and return 501.
The hash path reads `auth.one_time_tokens` (`confirmation_token`,
`recovery_token`, or either email-change type). The email path reads the
user's token columns. Email change with secure email change on tries the
current token, then the new token, and hides a row whose audience differs
when the compared address is equal.

## Out of scope

`auth:verify-type:email`, `auth:verify-type:magiclink`,
`auth:verify-type:sms`, `auth:verify-type:phone_change`, and
`auth:grant-type:pkce` return HTTP 501 via
`megabase_core::MegabaseNotImplemented`. Phone OTP on an in-scope type does
too. Anything not served returns the body
`{"code":"MEGABASE_NOT_IMPLEMENTED","component":"auth","message":"<unit> is not implemented by Megabase yet","unit":"<unit>"}`.

## Judge cases

`judge/cases/auth.toml` on `review/issue-21-verify-cases` exercises the
validation errors, the unknown-type 400, and the not-found 403/303 for each
in-scope type. A success path needs a token the reference stack and Megabase
both store; autoconfirm and the lack of a mail catcher make that a crate
test on the memory backend instead.
