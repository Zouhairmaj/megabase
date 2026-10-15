# Auth: invite by email (issue 401)

Status: complete for the no-op mail client and an unreachable SMTP host
Unit ids (`coverage/units.json`): `auth:route:POST /auth/v1/invite`
Level: 2

`POST /auth/v1/invite` backs supabase-js
`auth.admin.inviteUserByEmail(email, { data })`. An admin creates (or
re-invites) an unconfirmed user and the invite email carries a confirmation
token. `POST`/`GET /auth/v1/verify` with `type=invite` redeems it for a
session ([`verify.md`](verify.md)); the user then sets a password with
`PUT /auth/v1/user` ([`user.md`](user.md)).

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Route with `requireAdminCredentials` | [`internal/api/api.go:228`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L228), [`internal/api/middleware.go:187`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/middleware.go#L187) |
| `InviteParams`, `Invite` | [`internal/api/invite.go:14`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/invite.go#L14), [`:20`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/invite.go#L20) |
| `sendInvite` | [`internal/api/mail.go:360`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/mail.go#L360) |
| `sendEmail`: authorized addresses, email limiter | [`internal/api/mail.go:762`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/mail.go#L762) |
| New user model (`ToUserModel`, `NewUser`) | [`internal/api/signup.go:75`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/signup.go#L75), [`internal/models/user.go:110`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/user.go#L110) |
| `DuplicateEmailMsg` | [`internal/api/errors.go:20`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/errors.go#L20) |
| `user_invited` audit action (`team`) | [`internal/models/audit_log_entry.go:28`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/audit_log_entry.go#L28) |

Pins: Auth `v2.197.0` (`4eee58f`, MIT). Mail configuration, the mail client
selection, and the SMTP dial are the same as for recovery
([`recover.md`](recover.md)). The stock reference stack sets
`SMTP_HOST=supabase-mail` but runs no mail server, so an invite there ends
with the 500 in step 6.

## Inputs

`Authorization: Bearer <admin JWT>` (role in `GOTRUE_JWT_ADMIN_ROLES`,
`service_role` by default). JSON body, at most 1 MiB: `email` (string),
`data` (object). Unknown keys are ignored, keys match case-insensitively, a
top-level `null` or `null` field is empty, and a type mismatch is the Go
`bad_json` text (`gojson::INVITE_PARAMS`). Audience is `X-JWT-AUD` when
non-empty, else `GOTRUE_JWT_AUD`.

## Steps and outputs

1. Admin check (before the body is read): no Bearer is 401
   `no_authorization`; an unverifiable token is 403 `bad_jwt`; a non-admin
   role (`anon`, `authenticated`) is 403 `not_admin` / `User not allowed`.
2. Body errors: 413 `request_entity_too_large`, 400 `bad_json`.
3. `validateEmail`: empty is 400 `validation_failed` /
   `An email address is required`; over 255 bytes `An email address is too long`;
   bad format `Unable to validate email address: invalid format`. The email
   is lowercased.
4. Look the user up by email and audience (non-SSO). A lookup failure is 500
   `Database error finding user`. A confirmed user is 422 `email_exists` /
   `A user with this email address has already been registered`.
5. Generate an OTP (`GOTRUE_MAILER_OTP_LENGTH`) and its SHA-224 hash of
   email + OTP. `sendEmail` checks: an address outside
   `GOTRUE_EXTERNAL_EMAIL_AUTHORIZED_ADDRESSES` is 400
   `email_address_not_authorized`; a zero email rate, or a spent limiter
   while `mailer_autoconfirm` is off, is 429 `over_email_send_rate_limit` /
   `email rate limit exceeded`. Invite has no per-user frequency limit.
6. Mail: no SMTP host is the no-op client (success). An unreachable host is
   500 `unexpected_failure` / `Error sending invite email`, and nothing is
   written (GoTrue rolls the transaction back). A reachable host is 501 with
   the invite unit: the SMTP exchange is not ported.
7. One transaction: a new user is inserted (nil `instance_id`, role
   `GOTRUE_JWT_DEFAULT_GROUP_NAME`, empty `encrypted_password`, phone NULL,
   `raw_app_meta_data` `{"provider":"email","providers":["email"]}`,
   `raw_user_meta_data` = `data` or `{}`) with an email identity whose data is
   `{"sub":<id>,"email":<email>,"email_verified":false,"phone_verified":false}`.
   An existing unconfirmed user is reused and `data` is ignored. Then the
   `user_invited` audit row (actor nil id, username = admin role, traits
   `user_id`, `user_email`), and `confirmation_token`, `confirmation_sent_at`,
   `invited_at`, `updated_at` on the user, and a `confirmation_token`
   one-time token that replaces any earlier one. A write failure is 500
   `Error inviting user`.
8. 200 with the user object (same shape as signup's `user`, see
   [`endpoints.md`](endpoints.md)), plus `invited_at` and
   `confirmation_sent_at` (both the send time). `email_confirmed_at` and
   `confirmed_at` are absent.

## Edge cases

- Re-invite of an unconfirmed user returns the same `id` and stores a fresh
  token; the old one no longer verifies.
- Verify with `type=invite` on a user with no password sets a random one
  (`verify.md`), so the confirmed user signs in only after `PUT /user`.
- After verify the user is confirmed, so another invite is 422.
- The before/after user-created hooks are not configurable in Megabase and
  are not run.

## Judge cases

To add on `review/issue-401-cases`: anon key → 403 `not_admin`, no bearer
→ 401, invalid email → 400, and invite of a confirmed (signed-up) user → 422
`email_exists` (no mail is sent, so it matches the stock stack). The success
path, re-invite, and invite → verify → `PUT /user` need a reference stack
with a reachable mail server or an empty `GOTRUE_SMTP_HOST`.
