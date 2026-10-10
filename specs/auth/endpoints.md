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
verified user access token. Invite, recover, resend, and reauthenticate
answer HTTP 501 with their unit id. They are not public: invite needs
admin credentials and reauthenticate needs a user session.

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

Health and settings take no body. Signup, invite, recover, and resend read
a JSON body of at most `1 << 20` bytes.

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
| `GET /auth/v1/settings` | yes | 200 provider booleans plus `disable_signup`, `mailer_autoconfirm`, `phone_autoconfirm`, `sms_provider`, `saml_enabled`, `saml_private_key_next_configured`, `passkeys_enabled` |
| `POST /auth/v1/signup` | email autoconfirm | 200 access-token response: `access_token`, `token_type` `bearer`, `expires_in` (JWT expiry, 3600 by default), `expires_at`, `refresh_token`, `user` |
| `POST /auth/v1/logout` | yes | 204 empty body |
| `GET /auth/v1/reauthenticate`, `POST /auth/v1/invite`, `POST /auth/v1/recover`, `POST /auth/v1/resend` | no | 501 `MEGABASE_NOT_IMPLEMENTED` with that unit id |

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
| Confirmed duplicate email | 422 | `user_already_exists` / `User already registered` |
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
- Phone signup, anonymous signup when that provider is enabled, and email
  signup when `mailer_autoconfirm` is false return 501 before any write.
- A wrong method on a registered path is 405. `GET`/`POST /verify` is
  specified in [`verify.md`](verify.md). Any other `/auth/v1` path is 501
  with unit `"<METHOD> <path>"`.

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
