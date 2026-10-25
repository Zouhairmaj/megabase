# Auth: verify (issue 21)

Status: complete
Unit ids (`coverage/units.json`): `auth:route:GET /auth/v1/verify`,
`auth:route:POST /auth/v1/verify`, `auth:verify-type:signup`,
`auth:verify-type:invite`, `auth:verify-type:recovery`,
`auth:verify-type:email_change` (Level 1); `auth:verify-type:magiclink`,
`auth:verify-type:email`, `auth:verify-type:sms`,
`auth:verify-type:phone_change` (Level 2, issue 54)
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
| recovery, magiclink | [`internal/api/verify.go:154`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/verify.go#L154) |
| email_change | [`internal/api/verify.go:156`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/verify.go#L156) |
| `email` OTP type (`verifyTokenHash`, `verifyUserAndToken`) | [`internal/api/verify.go:650`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/verify.go#L650), [`:676`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/verify.go#L676), [`:735`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/verify.go#L735) |
| `sms`, `phone_change` (`smsVerify`, phone branch of `verifyUserAndToken`) | [`internal/api/verify.go:402`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/verify.go#L402), [`:753`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/verify.go#L753) |
| `ConfirmPhone`, `ConfirmPhoneChange`, phone lookups | [`internal/models/user.go:547`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/user.go#L547), [`:614`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/user.go#L614), [`:704`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/user.go#L704), [`:1205`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/user.go#L1205) |
| `validatePhone` | [`internal/api/phone.go:27`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/phone.go#L27) |
| Token hash SHA-224 | [`internal/crypto/crypto.go:45`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/crypto/crypto.go#L45) |
| Redirect target | [`internal/utilities/request.go:75`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/utilities/request.go#L75) |

Pins: `auth v2.197.0` (`vendor.toml`, commit `4eee58f`). Ported code carries
the credit header required by AGENTS.md.

## Inputs

Both methods take `type` (`signup`, `invite`, `recovery`, `magiclink`,
`email_change`, `email`, `sms`, `phone_change`). `magiclink` is `recovery` in every step (the
`recovery_token`, its expiry, `recoverVerify`); only the `type` in the GET
success fragment differs.

`email` (`EmailOTPVerification`, what supabase-js `verifyOtp({ type:
'email' })` sends after `signInWithOtp`) resolves to one of those types and
then continues as it:

- Hash path (GET, or POST with `token_hash`): the `auth.one_time_tokens` row
  may be a `confirmation_token` or a `recovery_token`. When the user's
  `recovery_token` equals the hash, the type becomes `magiclink` and expiry
  uses `recovery_sent_at`; otherwise `signup` with `confirmation_sent_at`.
- Email path (POST with `email` and `token`): the confirmation token is
  checked first (`signup`), then the recovery token (`magiclink`).

The GET success fragment carries the resolved type (`signup` or
`magiclink`), not `email`.

`sms` and `phone_change` are served only by POST with `phone` and `token`.
`verifyTokenHash` does not know them, so GET and POST with `token_hash`
give the `Invalid email verification type` 400, as an unknown type does.
POST with `email` and `token` looks up an empty phone and gets the 403
`Token has expired or is invalid`. With `phone`:

- The phone is validated as in signup (`+` and spaces removed, then
  `^[1-9][0-9]{1,14}$`), and the hash is SHA-224 of `phone + token`.
- `sms` finds the non-SSO user in the request audience whose `phone` is
  that number, and checks the hash against `confirmation_token` and
  `confirmation_sent_at`. `phone_change` matches `phone_change` instead, and
  checks `phone_change_token` and `phone_change_sent_at`. Expiry is
  `GOTRUE_SMS_OTP_EXP` seconds (`0` or unset means 60), not the mailer
  window. A stored `pkce_` prefix still matches.
- `sms` (`ConfirmPhone`): writes a `user_signedup` / `team` audit with
  `provider` `phone`, clears `confirmation_token`, sets
  `phone_confirmed_at`, and clears every one-time token of the user.
- `phone_change` (`ConfirmPhoneChange`): writes a `user_modified` / `user`
  audit. The `phone` identity (provider id = user id) gets `phone` and
  `phone_verified: true`, or is created with them. `phone_change` moves onto
  `phone`, `phone_change` and `phone_change_token` are cleared,
  `phone_confirmed_at` is set, and every one-time token of the user is
  cleared.
- Both turn `is_anonymous` off, refresh `app_metadata.providers`, and issue
  the same OTP session as the email types.

Phone plus OTP on an email type looks up an empty email and gets the 403
`Token has expired or is invalid`.

GET reads `token` and `redirect_to` from the query. `token` is stored as the
token hash (GoTrue copies it onto `token_hash`). The redirect target is the
first allowed URL among `redirect_to`, the `Referer` header, and the site URL.
The site URL is `GOTRUE_SITE_URL`, or `SITE_URL` when that is unset
(`http://localhost:3000` when both are unset). A candidate is parsed with
the WHATWG URL parser and allowed when it matches the site (same scheme
and written host, as in Go `URL.Hostname`; a localhost host may use
another port), is a
dotted-decimal or IPv6 loopback address, or matches `GOTRUE_URI_ALLOW_LIST`
(`ADDITIONAL_REDIRECT_URLS` when that list is unset) on the URL with its
fragment removed and its query kept. Userinfo does not reject the URL; the
host is the host after it. A backslash in the authority is rejected. An
all-digit host is rejected (a non-empty host of only digits). An IPv4
spelling that Go keeps as a hostname is matched as written, including
against the allow list. An `http` or `https` domain is checked against
GoTrue's regular hostname pattern on that written host, the same host
the allow-list match text contains. A non-HTTP(S) URL with no host, including an
empty host, is same-site when the site host is also empty; otherwise it
is matched against the allow list. An allow-list pattern
this port cannot compile
(`[`, `]`, `{`, `}`, or a dangling `\`) is HTTP 501 `GOTRUE_URI_ALLOW_LIST`
instead of a guessed site-URL fallback.

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
empty `sb`. Any `#` already on the redirect base is removed first. `SITE_URL`
`http://localhost:3000` has an empty path, so the redirect has no slash before
`#`.

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
| POST phone that fails the E.164 check | 400 | `validation_failed` / `Invalid phone number format (E.164 required)` |
| Phone OTP: no user for the phone, wrong token, or expired | 403 | `otp_expired` / `Token has expired or is invalid` |
| Hash lookup, unknown type | 400 | `validation_failed` / `Invalid email verification type` (GET redirects this) |
| Hash not found, expired, or `sent_at` missing | 403 | `otp_expired` / `Email link is invalid or has expired` |
| Email OTP not found or not valid | 403 | `otp_expired` / `Token has expired or is invalid` |
| Banned (`banned_until` in the future), checked before expiry | 403 | `user_banned` / `User is banned` |
| No database | 500 | `unexpected_failure` / `Database error finding user from email link` on the hash path, `Database error finding user` on the email path (GET redirects this) |
| `JWT_SECRET` missing | 500 | `unexpected_failure` / `Server lacks JWT secret`, before the one-time token is read (GET redirects this) |
| `GOTRUE_RATE_LIMIT_VERIFY` (default 30 per five minutes, burst 30) | 429 | `over_request_rate_limit` / `Request rate limit reached` |

The verify limiter runs before the body is read, as `limitHandler` on `/verify`
([`internal/api/api.go:270`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L270),
[`internal/api/apilimiter/apilimiter.go:202`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/apilimiter/apilimiter.go#L202)).
`performRateLimiting` ([`internal/api/middleware.go:123`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/middleware.go#L123))
keys on the leftmost `Sb-Forwarded-For` address when
`GOTRUE_SECURITY_SB_FORWARDED_FOR_ENABLED` is true and that value is an IP.
Otherwise the key is `GOTRUE_RATE_LIMIT_HEADER`. The flag defaults to false.
A request with neither key is not counted. GET failures after this check stay
303 redirects; the 429 itself is JSON on both methods.

`MEGABASE_AUTH_OTP_MAX_FAILED_ATTEMPTS` defaults to `0`, which leaves a bad
code in place, as GoTrue does. A positive value, after that many failed
checks of one user, clears every pending verification token for that user
(confirmation, recovery, both email-change tokens, and the phone-change
token) and every `auth.one_time_tokens` row of that user. Later checks,
including a code that was still valid, get the same 403 `otp_expired` as a
bad token. Each count lives until that code's OTP deadline, the map drops
the soonest expiry past 1024 entries, and storing a new code drops that
user's counts. A hash that matches nobody is not counted.

Expiry is strict: the token is expired only when now is after `sent_at + GOTRUE_MAILER_OTP_EXP` seconds. Unset or `0` means 86400. A positive value is kept.

## Edge cases

A stored hash of `pkce_` plus the SHA-224 still matches an email OTP, and
`POST` always issues a session, whatever the prefix.

### PKCE flow (`GET` with a `pkce_` token)

`verifyGet` and `issueAuthCode`
([`internal/api/verify.go:137`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/verify.go#L137),
[`internal/api/pkce.go:42`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/pkce.go#L42)).
Such tokens come from PKCE `POST /recover` ([`recover.md`](recover.md)) and
PKCE `POST /magiclink` or `POST /otp` ([`endpoints.md`](endpoints.md)).

1. After the redirect target, `type` is parsed with
   `ParseAuthenticationMethod` (any value ending in `signup` is
   `email/signup`). A type it rejects (`sms`, `phone_change`, `email`, …) is
   a JSON 500 `unexpected_failure` / `Unexpected failure, please check server
   logs for more information`, before the 501 checks. `magiclink` parses.
2. The token is verified and the user updated exactly as in the implicit
   flow, except that `last_sign_in_at` is not set and no session is issued.
   A missing `JWT_SECRET` does not matter, since nothing is signed.
3. In the same transaction, the user's latest `auth.flow_state` row for that
   method (`user_id` and `authentication_method`, `created_at DESC, id
   DESC`) must exist and have a code challenge. Its `auth_code_issued_at`
   (and `updated_at`) becomes now. Otherwise the transaction rolls back
   (the token stays usable) and the error is 400 `flow_state_not_found` /
   `No associated flow state found. 422: No valid flow state found for
   user.` or `… 422: Flow state does not have an auth code (not a PKCE
   flow).`.
4. Success: 303 to the redirect target with `code=<auth_code>` set in its
   query (`prepPKCERedirectURL`). Go re-encodes the query: pairs are parsed
   (a pair with `;` or a bad `%` escape is dropped), `code` replaces any
   earlier one, and keys are sorted. The fragment is kept.
5. Error redirects (`prepErrorRedirectURL`) and the email-change
   single-confirmation message (`prepRedirectURL`) also set `error`,
   `error_code`, `error_description` (or `message`) in the re-encoded
   query, besides the usual fragment.

The client then redeems the code with `POST /token?grant_type=pkce`
([`token.md`](token.md)).
The hash path reads `auth.one_time_tokens` (`confirmation_token`,
`recovery_token`, or either email-change type). The email path reads the
user's token columns. Email change with secure email change on tries the
current token, then the new token, and hides a row whose audience differs
when the compared address is equal.

PostgreSQL locks `auth.users` with `SELECT … FOR UPDATE` after the first
lookup, then looks the token up again before it changes token columns or
deletes `auth.one_time_tokens`. A second redemption of the same token waits
for that lock and then sees the cleared token, so one token issues one
session. The memory backend re-reads the token under its mutex after any
invite password hash, which is the same order.

## Out of scope

Phone OTP (`sms`, `phone_change` with `phone`) returns HTTP 501 via
`megabase_core::MegabaseNotImplemented` when `GOTRUE_SMS_TEST_OTP` is set
(unit `GOTRUE_SMS_TEST_OTP`) or `GOTRUE_SMS_PROVIDER` is `twilio_verify`
(unit `GOTRUE_SMS_PROVIDER=twilio_verify`). GoTrue accepts the test OTP map
or asks Twilio Verify before the stored token there, and neither is ported.
The phone-changed and identity-linked notification emails are not sent
(both are off by default). Anything not served returns the body
`{"code":"MEGABASE_NOT_IMPLEMENTED","component":"auth","message":"<unit> is not implemented by Megabase yet","unit":"<unit>"}`.

## Judge cases

`judge/cases/auth.toml` on `review/issue-21-verify-cases` exercises the
validation errors, the unknown-type 400, and the not-found 403/303 for each
in-scope type. A success path needs a token the reference stack and Megabase
both store; autoconfirm and the lack of a mail catcher make that a crate
test on the memory backend instead. The same holds for `sms` and
`phone_change`: the reference stack autoconfirms phones and has no SMS
provider, so no OTP is ever stored. Their judge cases can cover the hash
path 400/303 and the unknown-phone 403.
