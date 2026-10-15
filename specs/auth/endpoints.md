# Auth: endpoints (issue 15)

Status: complete
Unit ids (`coverage/units.json`): `auth:route:GET /auth/v1/health`,
`auth:route:GET /auth/v1/settings`, `auth:route:GET /auth/v1/reauthenticate`,
`auth:route:POST /auth/v1/signup`, `auth:route:POST /auth/v1/invite`,
`auth:route:POST /auth/v1/logout`, `auth:route:POST /auth/v1/recover`,
`auth:route:POST /auth/v1/resend`
Level: 1

These are the GoTrue routes in the Level 1 endpoints group. The gateway
keeps the Kong prefix, so clients call `/auth/v1/...`. Health and settings
are public. Signup is public and, with the judge's autoconfirm overlay,
creates an `auth.users` row and a session. Logout revokes sessions for a
verified user access token. Recover is specified in
[`recover.md`](recover.md) and invite in [`invite.md`](invite.md). Resend
and reauthenticate answer HTTP 501 with their unit id after any implemented
validation. Resend is public; reauthenticate needs a user session in GoTrue.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Route table | [`vendor/auth/internal/api/api.go:202`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L202) (health), [`:222`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L222) (settings), [`:228`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L228) (invite), [`:229`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L229) (signup), [`:256`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L256) (recover), [`:259`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L259) (resend), [`:275`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L275) (logout), [`:277`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L277) (reauthenticate) |
| 1 MiB body limit | [`vendor/auth/internal/api/api.go:182`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L182) |
| Health body | [`vendor/auth/internal/api/api.go:492`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L492) |
| Settings body | [`vendor/auth/internal/api/settings.go:45`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/settings.go#L45) |
| Signup | [`vendor/auth/internal/api/signup.go:32`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/signup.go#L32) and [`:110`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/signup.go#L110) |
| Logout scopes | [`vendor/auth/internal/api/logout.go:21`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/logout.go#L21) |
| Bearer and session checks | [`vendor/auth/internal/api/auth.go:20`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/auth.go#L20) |
| JSON `application/json` | [`vendor/auth/internal/api/shared/http.go:8`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/shared/http.go#L8) |

Pins: Auth `v2.197.0` (`4eee58f296d9698a1c2c0ae14d7a0b379c7622d3`, MIT).
Reference-stack flags come from `vendor/supabase/docker/.env.example` and
`judge/compose.override.yml` (`GOTRUE_MAILER_AUTOCONFIRM=true`). Decision
20 in `PROGRESS.md`.

## Inputs

Health and settings take no body. They are registered with chi's `r.Get`
only (`vendor/auth/internal/api/api.go:202`, `:222`), so `HEAD` and every
other method get chi's `methodNotAllowedHandler`: 405, `Allow: GET`, empty
body. chi does not answer `HEAD` from a `GET` route. Without an `apikey`
the gateway answers first (`specs/server/key-auth.md`). Signup, invite,
recover, and resend read a JSON body of at most `1 << 20` bytes.

Signup JSON: `email`, `phone`, `password`, `data` (object), `channel`,
`code_challenge_method`, `code_challenge`. Audience is the `X-JWT-AUD`
header when it is non-empty, otherwise `GOTRUE_JWT_AUD` (default
`authenticated`). Signup does not read `Authorization`.

Logout reads `Authorization: Bearer <access token>` and optional query
`scope` (`global`, `local`, `others`, or empty).

## Outputs

| Route | Served | Success |
|---|---|---|
| `GET /auth/v1/health` | yes | 200 `{"version":"v2.197.0","name":"GoTrue","description":"GoTrue is a user registration and authentication API"}` |
| `GET /auth/v1/settings` | yes | 200 provider booleans plus `disable_signup`, `mailer_autoconfirm`, `phone_autoconfirm`, `sms_provider`, `saml_enabled`, `saml_private_key_next_configured`, `passkeys_enabled`. `saml_private_key_next_configured` is `true` while SAML is disabled: envconfig allocates `SAML.CertificateNext`, and only `PopulateFields` (called by `ApplyDefaults` when SAML is enabled) resets it to nil (`internal/conf/configuration.go:1094`, `internal/conf/saml.go:115`) |
| `POST /auth/v1/signup` | email | Autoconfirm on: 200 access-token response: `access_token`, `token_type` `bearer`, `expires_in` (JWT expiry, 3600 by default), `expires_at`, `refresh_token`, `user`. Autoconfirm off: 200 user object (see [Email signup without autoconfirm](#email-signup-without-autoconfirm)) |
| `POST /auth/v1/logout` | yes | 204 empty body |
| `POST /auth/v1/recover` | see [`recover.md`](recover.md) | 200 `{}` |
| `POST /auth/v1/invite` | see [`invite.md`](invite.md) | 200 user object |
| `GET /auth/v1/reauthenticate`, `POST /auth/v1/resend` | no | 501 `MEGABASE_NOT_IMPLEMENTED` with that unit id |

Signup user JSON matches GoTrue's user object for an autoconfirmed email
user: `phone` is `""`, and `confirmed_at` is omitted. Signup reloads the
row before `Confirm`, so the generated column is still null on the
in-memory user. `app_metadata` is
`{"provider":"email","providers":["email"]}`. `user_metadata` is the
identity claims (`sub`, `email`, `email_verified: true`,
`phone_verified: false`) plus request `data` keys that do not collide.
The signup response identity shares that map. The identity row was
inserted first, so it keeps `email_verified: false`. The refresh token is
the legacy 12-character lowercase base32 form. The access token is HS256
(`typ` `JWT`) with `aal` `aal1`, `amr`
`[{"method":"password","timestamp":...}]`, and `session_id`.

The row written to `auth.users` uses the nil `instance_id`, bcrypt cost 10,
phone SQL NULL, empty confirmation and recovery tokens (`''`, not NULL),
and `is_super_admin` NULL. Sessions, refresh tokens, AMR claims, and audit
rows are written in the same transaction. Logout deletes sessions; refresh
tokens and AMR rows follow `ON DELETE CASCADE`.

## Errors

GoTrue's default API shape (no `X-Supabase-Api-Version: 2024-01-01`):
`{"code":<http status integer>,"error_code":"...","msg":"..."}` and header
`x-sb-error-code`. Weak passwords add `weak_password.reasons`.

| When | Status | `error_code` / `msg` |
|---|---|---|
| Body larger than 1 MiB | 413 | `request_entity_too_large` / `Request body too large (max 1048576 bytes)` |
| Body is not JSON | 400 | `bad_json` |
| Email and phone both empty, anonymous disabled | 422 | `anonymous_provider_disabled` / `Anonymous sign-ins are disabled` |
| `disable_signup` | 422 | `signup_disabled` / `Signups not allowed for this instance` |
| Empty password | 400 | `validation_failed` / `Signup requires a valid password` |
| Password longer than 72 bytes | 400 | `validation_failed` / `Password cannot be longer than 72 characters` |
| Password shorter than the minimum (floor 6) | 422 | `weak_password` / `Password should be at least N characters.` |
| Both email and phone | 400 | `validation_failed` / `Only an email address or phone number should be provided on signup.` |
| Bad email | 400 | `validation_failed` / `Unable to validate email address: invalid format` |
| Email provider off | 400 | `email_provider_disabled` |
| Phone provider off | 400 | `phone_provider_disabled` |
| Confirmed duplicate email, autoconfirm on | 422 | `user_already_exists` / `User already registered` |
| No `JWT_SECRET` | 500 | `unexpected_failure` / `Server lacks JWT secret` |
| No database | 500 | `unexpected_failure` / `Database error saving new user` |
| Logout without Bearer | 401 | `no_authorization` / `This endpoint requires a valid Bearer token` |
| Bad JWT | 403 | `bad_jwt` / `invalid JWT: unable to parse or verify signature, ...` |
| Missing `sub` | 403 | `bad_jwt` / `invalid claim: missing sub claim` |
| `sub` not a UUID | 400 | `bad_jwt` / `invalid claim: sub claim must be a UUID` |
| Bad `session_id` | 403 | `bad_jwt` / `invalid claim: session_id claim must be a UUID` |
| Unknown user | 403 | `user_not_found` / `User from sub claim in JWT does not exist` |
| `banned_until` in the future | 403 | `user_banned` / `User is banned` |
| Session id not in the database | 403 | `session_not_found` / `Session from session_id claim in JWT does not exist` |
| Bad logout scope | 400 | `validation_failed` / `Unsupported logout scope "<scope>"` |

## Edge cases

- Email is lowercased. A confirmed duplicate does not change the password.
  An unconfirmed duplicate in the same audience is confirmed and given a
  session; the stored password hash stays as it was. A non-SSO email is
  unique across audiences (`users_email_partial_key` on `email` where
  `is_sso_user = false`). The same email with a different `x-jwt-aud` is
  `user_already_exists`.
- `scope=local` deletes that session. `scope=others` deletes the user's
  other sessions. Empty or `global` deletes every session for the user.
  A second global logout with the same token is `session_not_found`.
- `session_id` missing, empty, or the nil UUID means there is no session
  to check; `local` and `others` then log out everywhere.
- Phone signup and anonymous signup when that provider is enabled return
  501 before any write.
- A wrong method on a registered path is 405. `GET`/`POST /verify` is
  specified in [`verify.md`](verify.md). Any other `/auth/v1` path is 501
  with unit `"<METHOD> <path>"`.

## Email signup without autoconfirm

Source: `internal/api/signup.go:109-345` (`Signup`, `sanitizeUser`),
`internal/api/mail.go:319-358` (`sendConfirmation`), and
`internal/api/pkce.go` (`generateFlowState`, `addFlowPrefixToToken`), pin
v2.197.0. With `GOTRUE_MAILER_AUTOCONFIRM=false`, after the checks in
[Errors](#errors):

1. The user is looked up by email in the request audience
   (`IsDuplicatedEmail`, non-SSO users). A database error is 500
   `Database error finding user`.
2. A confirmed user: one `user_repeated_signup` audit entry and 200 with
   `sanitizeUser`: a random `id`, `role` `""`, `phone` `""`, `created_at`,
   `updated_at` and `confirmation_sent_at` now, no confirmation or sign-in
   timestamps, `identities` `[]`, the request `data` as `user_metadata`,
   and `app_metadata` `{"provider":"email","providers":["email"]}`. No
   email is sent and nothing else changes.
3. A new address: the password is bcrypt-hashed and the user is created
   unconfirmed with an email identity. As `RemoveUnconfirmedIdentities`
   does, `user_metadata` is the identity data: `sub`, `email`,
   `email_verified: false`, `phone_verified: false`, then the `data` keys
   that do not collide. An unconfirmed user is reused as it is (password and
   data unchanged); only a missing email identity is created, which then
   replaces its other identities, `user_metadata`, and `providers`.
4. A `user_confirmation_requested` audit entry (`traits.provider` `email`).
5. PKCE (`code_challenge`): an `auth.flow_state` row with `provider_type`
   `email`, `authentication_method` `email/signup`, the user id, the
   challenge, the lowercased method, and a fresh UUID `auth_code`. A method
   other than `s256` or `plain` is GoTrue's default 500 and nothing is kept.
6. `sendConfirmation`: the `GOTRUE_SMTP_MAX_FREQUENCY` limit on
   `confirmation_sent_at` (429 `over_email_send_rate_limit` /
   `For security purposes, you can only request this after N seconds.`),
   a new OTP, and the token SHA-224(email + OTP), prefixed `pkce_` in the
   PKCE flow. Then the mail client as in [`recover.md`](recover.md): the
   authorized-address list, the email limiter, no SMTP host is the no-op
   client, an unreachable host is 500 `Error sending confirmation email`,
   and a reachable host is 501 because the SMTP exchange is not ported.
7. One transaction stores steps 3 to 6: `confirmation_token`,
   `confirmation_sent_at`, and a `confirmation_token` one-time token that
   replaces the previous one. A mail or write failure keeps nothing; a
   write failure is 500 `Database error saving new user`.
8. 200 with the user object (no session), its email identity, and
   `confirmation_sent_at`. An invited user (`invited_at` set) is shown with
   `invited_at`, `user_metadata` `{}`, and `identities` `[]`.

`POST /verify` or `GET /verify` with `type=signup` redeems the token
([`verify.md`](verify.md)); a `pkce_` link redirects with the auth code
that `grant_type=pkce` exchanges ([`token.md`](token.md)). The judge stack
sets autoconfirm on, so it never takes this path.

## Partial: POST /resend

Ported validation source: `internal/api/resend.go:15-77` in Supabase Auth
`v2.197.0` (MIT). The public endpoint reads JSON fields `type`, `email`,
`phone`, `code_challenge`, and `code_challenge_method` with the existing
1 MiB body limit and `bad_json` errors. Missing or null strings are empty.
Keys match case-insensitively and the last duplicate wins, like Go
`encoding/json`.

Validation follows upstream order. All errors below are HTTP 400:

1. `type` must be `signup`, `email_change`, `sms`, or `phone_change`;
   otherwise `validation_failed` / `Missing one of these types: signup, email_change, sms, phone_change`.
   Signup and email change then use the existing PKCE validation, including
   `PKCE flow requires code_challenge_method and code_challenge` for an
   incomplete pair. SMS and phone change do not validate PKCE.
2. Signup with empty email returns `validation_failed` /
   `Type provided requires an email address`. SMS with empty phone returns
   `validation_failed` / `Type provided requires a phone number`.
3. Both addresses present returns `validation_failed` /
   `Only an email address or phone number should be provided.`
4. A nonempty email requires the email provider; otherwise
   `email_provider_disabled` / `Email logins are disabled`. It then uses
   the existing email validator (format and length only; the normalized
   value is discarded because the route stops at 501 before any lookup).
   A nonempty phone requires the phone provider; otherwise
   `phone_provider_disabled` / `Phone logins are disabled`. It then uses
   the existing E.164 validator (format only; the normalized value is
   discarded); invalid format returns `validation_failed` /
   `Invalid phone number format (E.164 required)`.
5. Both addresses empty returns `validation_failed` /
   `Missing email address or phone number`.

After validation, the endpoint returns HTTP 501 `MEGABASE_NOT_IMPLEMENTED`
with unit `auth:route:POST /auth/v1/resend`. It performs no user lookup,
enumeration, writes, or mail/SMS delivery. No coverage marker is added
until delivery is ported.

## Level 2 (issue 46): authorize, callback, magiclink, otp

Unit ids: `auth:route:GET /auth/v1/authorize`,
`auth:route:GET /auth/v1/callback`, `auth:route:POST /auth/v1/callback`,
`auth:route:POST /auth/v1/magiclink`, `auth:route:POST /auth/v1/otp`.
`POST /magiclink` carries a `megabase:unit` marker: with autoconfirm on (the
judge stack sets `GOTRUE_MAILER_AUTOCONFIRM=true`) its email path goes
through the same mail client as `POST /recover`. The others have none yet:
their success paths need an OAuth provider exchange or an SMS sender, which
stay 501. `POST /otp` serves its email path but not its phone path, which
the reference stack enables.

| Behavior | File:line (pin) |
|---|---|
| Route table | `vendor/auth/internal/api/api.go:210-216` (callback), `:226` (authorize), `:262` (magiclink), `:265` (otp) |
| Authorize | `vendor/auth/internal/api/external.go:27-139` |
| Callback | `vendor/auth/internal/api/external.go:142-283`, `:534-599` (state), `:821-897` (`redirectErrors`); `external_oauth.go:31-123` |
| Magic link | `vendor/auth/internal/api/magic_link.go:25-150`, `mail.go:480-521` (`sendMagicLink`), `crypto/password.go:394` (`GeneratePassword`) |
| OTP | `vendor/auth/internal/api/otp.go:37-238` |

### GET /authorize

Served up to the provider lookup (unchanged by issue 46): an unknown,
disabled, or (custom OAuth off) `custom:` provider is 400
`validation_failed` / `Unsupported provider: <reason>`. An enabled provider
would create an `auth.flow_state` row and 302 to the provider; that is 501.

### GET and POST /callback

`state` comes from the query (`GET`) or `r.FormValue` (`POST`: a urlencoded
body before the query). Query parsing follows `url.ParseQuery`: pairs with
`;` or a bad `%` escape are dropped. A multipart `POST` body is 501.

The `loadFlowState` middleware checks, in order:

| When | Status | `error_code` / `msg` |
|---|---|---|
| `state` empty | 400 | `bad_oauth_callback` / `OAuth state parameter missing` |
| `state` not a UUID | 400 | `bad_oauth_state` / `OAuth state parameter is invalid` |
| No `auth.flow_state` row | 400 | `bad_oauth_state` / `OAuth state not found or expired` |
| Database error | 500 | `unexpected_failure` / `Error loading flow state` |
| Older than 300 s (`created_at`, or `auth_code_issued_at` for `magiclink` rows) | 400 | `bad_oauth_state` / `OAuth state has expired` |
| PKCE row (`code_challenge` set) with `user_id` set | 400 | `flow_state_already_used` / `State has already been used` |
| `linking_target_id` user missing | 422 | `user_not_found` / `Linking target user not found` |

A middleware failure is a 303 to `SITE_URL` with `error`, `error_code`, and
`error_description` merged into its query (`url.Values.Encode`, keys
sorted). `error` maps 400 `invalid_request`, 401 `unauthorized_client`, 403
`access_denied`, 500 `server_error`, 503 `temporarily_unavailable`, anything
else `server_error`; `signup_disabled`, `user_banned`, and
`provider_email_needs_verification` are `access_denied`. GoTrue then writes
its JSON error into the already-sent response, so the body is Go's redirect
HTML (`GET` only) followed by `{"code":…,"error_code":…,"msg":…}`. The
headers are those of the redirect: `GET` has `Content-Type: text/html;
charset=utf-8`; `POST` has Go's sniffed `text/plain; charset=utf-8`; there
is no `x-sb-error-code`.

The handler then redirects errors with a 302 to `GOTRUE_EXTERNAL_REDIRECT_URL`,
else the row's `referrer`, else `SITE_URL`. The error is set in the query
(as above) and in the fragment: `error`, `error_code`, `error_description`
(each only when non-empty) and `sb=`, escaped as `URL.EscapedFragment`
(so `%` from the query encoding becomes `%25`). `POST` redirects have no
body and no `Content-Type`. In order:

- `error` parameter present: OAuth error, `error` and `error_description`
  copied, no `error_code`.
- `code` empty: 400 `bad_oauth_callback` / `OAuth callback with missing authorization code missing`.
- Provider unknown or disabled: 400 `oauth_provider_not_supported` /
  `Unsupported provider: <reason>` (same reasons as authorize).
- Enabled provider, or a `twitter` row (OAuth 1.0): 501, since the code
  exchange and account linking are not ported.

### POST /magiclink

1. Email provider off: 422 `email_provider_disabled` / `Email logins are disabled`.
   `GOTRUE_EXTERNAL_EMAIL_MAGIC_LINK_ENABLED=false`: 422
   `email_provider_disabled` / `Login with magic link is disabled`.
2. Decode with `json.NewDecoder`: only the first JSON value is read and
   trailing bytes are ignored. Errors are 400 `bad_json` /
   `Could not read verification params: <detail>`, where an empty body is
   `EOF`, a truncated value `unexpected EOF`, and a body over 1 MiB
   `http: request body too large`. Top-level `null` is an empty request.
3. Empty email: 422 `validation_failed` / `Password recovery requires an email`.
   Then the signup email validator and PKCE checks.
4. Autoconfirm off: 501. GoTrue would sign a new address up and send the
   signup confirmation email, which is not ported. No user lookup runs
   first, so the answer does not reveal whether the account exists.
5. The user is looked up by email in the request audience (a database error
   is 500 `unexpected_failure` / `Database error finding user`). No row, or
   an unconfirmed email, is a new user: GoTrue runs `Signup` with a
   33-character random password (letters and digits) and the request's
   `data`, then calls `MagicLink` again on the same params.
   - Signups disabled: 422 `signup_disabled` /
     `Signups not allowed for this instance`.
   - `GOTRUE_PASSWORD_MIN_LENGTH` above 33: 422 `weak_password` /
     `Password should be at least <n> characters.`
   - The signup is the autoconfirm email signup (`token.md`, `POST /signup`):
     a new user is created confirmed, an unconfirmed one is confirmed
     without changing its password, and a session is issued and discarded.
     Its errors are `Signup`'s.
6. PKCE (`code_challenge` set): an `auth.flow_state` row with `provider_type`
   and `authentication_method` `magiclink` is written and committed before
   the email step, as for PKCE `/recover` (`recover.md` step 3a).
7. `sendMagicLink` is `sendPasswordRecovery` with the magic link template
   (`recover.md`): the `recovery_sent_at` frequency limit, a new OTP whose
   hash (`pkce_`-prefixed for PKCE) is stored as `recovery_token` with a
   `recovery_token` one-time token and a `user_recovery_requested` audit
   entry, and the same mail client. The only difference is the failure
   message: 500 `unexpected_failure` / `Error sending magic link email`.
   A reachable SMTP host is 501, after the signup and flow state writes.
8. Success: 200 `{}`. The link is `GET /verify?type=magiclink&token=<hash>`
   (`verify.md`).

### POST /otp

1. Decode `OtpParams` with `json.Unmarshal` (`create_user` defaults to true,
   and `null` keeps it). Errors are the usual `bad_json` text.
2. Email and phone both set: 400 `Only an email address or phone number should be provided`.
   Email with `channel`: 400 `Channel should only be specified with Phone OTP`.
   Then PKCE.
3. `create_user: false`: the email (or phone) is validated and looked up in
   the request audience; no row is 422 `otp_disabled` /
   `Signups not allowed for otp`. A database error lets the request through,
   like GoTrue.
4. Email: `MagicLink` on the same body, steps 1 to 8 above (its 501 names
   unit `auth:route:POST /auth/v1/otp`). Phone: phone provider off is
   400 `phone_provider_disabled` / `Unsupported phone provider`; then E.164
   format and the channel check (`channel` defaults to `sms`), then 501.
   Neither: 400 `One of email or phone must be set`.

The magic link and OTP rate limiters (`limitHandler`) are not served.

## Out of scope

`GET /user`, admin routes, captcha, rate limits, hooks, mail, and SMS.
`POST /token` for the password and refresh-token grants is specified in
[`token.md`](token.md). The other grant types on that route stay 501.
Judge cases `auth.user.*` and `auth.admin.*` still fail until those units
are ported.

## Judge cases

Already in `judge/cases/auth.toml` (not edited here): `auth.health`,
`auth.settings`, `auth.signup.password`, `auth.signup.weak-password`,
`auth.logout`. Conformance was not measured in this change (no Docker).
