# Auth: password recovery (issue 382)

Status: complete for the no-op mail client and an unreachable SMTP host
Unit ids (`coverage/units.json`): `auth:route:POST /auth/v1/recover`
Level: 2

`POST /auth/v1/recover` starts password recovery for an email address
(supabase-js `auth.resetPasswordForEmail`). It stores a recovery token that
`POST`/`GET /auth/v1/verify` with `type=recovery` redeems for a session; the
client then sets the new password with `PUT /auth/v1/user`
([`verify.md`](verify.md), [`user.md`](user.md)).

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Route, `requireEmailProvider` | [`internal/api/api.go:256`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L256), [`internal/api/middleware.go:231`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/middleware.go#L231) |
| `RecoverParams.Validate`, `Recover` | [`internal/api/recover.go:18`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/recover.go#L18), [`:33`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/recover.go#L33) |
| `sendPasswordRecovery` | [`internal/api/mail.go:398`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/mail.go#L398) |
| Frequency limit message | [`internal/api/mail.go:724`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/mail.go#L724), [`internal/api/errors.go:255`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/errors.go#L255) |
| `sendEmail`: authorized addresses, email limiter | [`internal/api/mail.go:762`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/mail.go#L762) |
| Mail client selection (no-op when `SMTP.Host` is empty) | [`internal/mailer/templatemailer/template.go:40`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/mailer/templatemailer/template.go#L40) |
| SMTP dial (gomail `DialAndSend`) | [`internal/mailer/mailmeclient/mailmeclient.go:47`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/mailer/mailmeclient/mailmeclient.go#L47) |
| `RateLimitEmailSent`, `SMTP.MaxFrequency` default | [`internal/conf/configuration.go:466`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/conf/configuration.go#L466), [`:1211`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/conf/configuration.go#L1211) |
| Interval and burst limiters | [`internal/ratelimit/interval.go`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/ratelimit/interval.go), [`internal/ratelimit/burst.go`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/ratelimit/burst.go) |

Pins: Auth `v2.197.0` (`4eee58f`, MIT). The reference stack sets
`SMTP_HOST=supabase-mail`, `SMTP_PORT=2500` (`vendor/supabase/docker/.env.example`)
but runs no mail server, so its SMTP dial always fails.

## Inputs

- JSON body (at most 1 MiB): `email`, `code_challenge`,
  `code_challenge_method`. `null` counts as empty, as in Go's decoder.
- Audience: `X-JWT-AUD` when non-empty, else `GOTRUE_JWT_AUD`.
- Configuration: `GOTRUE_SMTP_HOST` (unset is `supabase-mail`; empty selects
  the no-op client), `GOTRUE_SMTP_PORT` (unset is `2500`),
  `GOTRUE_SMTP_MAX_FREQUENCY` (Go duration; zero or unset is one minute),
  `GOTRUE_RATE_LIMIT_EMAIL_SENT` (`N` per hour or `N/duration`, default 30),
  `GOTRUE_EXTERNAL_EMAIL_AUTHORIZED_ADDRESSES`, `GOTRUE_MAILER_OTP_LENGTH`.

## Steps and outputs

1. Email provider off: 400 `email_provider_disabled` / `Email logins are disabled`.
2. Body not JSON: 400 `bad_json`. Empty `email`: 400 `validation_failed` /
   `Password recovery requires an email`. Then the signup email validator
   (lowercases the address) and the PKCE parameter check.
3. Look the user up by lowercased email and audience (non-SSO). No user: 200
   `{}`. Nothing else runs, so known and unknown addresses cannot be told
   apart by the success answer.
4. `recovery_sent_at + max_frequency` still in the future: 429
   `over_email_send_rate_limit` / `For security purposes, you can only request
   this after N seconds.` (whole seconds left, truncated).
5. Generate an OTP (`GOTRUE_MAILER_OTP_LENGTH` digits, 6 to 10) and the token
   hash `hex(SHA-224(email + otp))`.
6. `sendEmail` checks: an address outside a non-empty authorized list (a
   `+label` is ignored) is 400 `email_address_not_authorized` /
   `Email address "<email>" cannot be used as it is not authorized`. An email
   rate of 0 events is 429 `over_email_send_rate_limit` /
   `email rate limit exceeded`; with autoconfirm off, the process-wide limiter
   returns the same 429 once spent.
7. Delivery:
   - No SMTP host: GoTrue's no-op client succeeds without sending.
   - SMTP host set and the TCP dial fails or times out (10 s): 500
     `unexpected_failure` / `Error sending recovery email`. Nothing is stored.
     This is what the reference stack answers for every known email.
8. In one transaction: audit entry `user_recovery_requested` (log type
   `user`), `recovery_token` and `recovery_sent_at` (and `updated_at`) on the
   user, and a `recovery_token` row in `auth.one_time_tokens` (relates to the
   lowercased email) that replaces the previous one. 200 `{}`.

## Not served (501 `MEGABASE_NOT_IMPLEMENTED`, unit `auth:route:POST /auth/v1/recover`)

- An SMTP host that accepts the TCP connection: the SMTP exchange, STARTTLS
  and authentication, and the recovery template (`ConfirmationURL` built from
  `MAILER_URLPATHS_RECOVERY` and the `redirect_to` referrer) are not ported.
  Nothing is stored.
- The PKCE flow (`code_challenge` set): it writes `auth.flow_state` and a
  `pkce_` token prefix that verify does not exchange. Answered before the user
  lookup, so it does not reveal whether the account exists.
- The send-email hook (`GOTRUE_HOOK_SEND_EMAIL_*`).

## Edge cases

- The OTP and token hash use the stored email, so a mixed-case request finds
  and hashes the same account.
- A failed delivery leaves the previous recovery token and `recovery_sent_at`
  untouched, so it does not start the frequency window.
- `redirect_to` is only used in the email body, which the served paths never
  render.

## Judge cases (separate `review/` PR)

Recover with an unknown email (200 `{}`), an invalid email (400), an empty
body (400), and a known fixture email (500 on the reference stack, whose mail
server is missing). The full recover → verify → `PUT /user` → password sign-in
flow and the 429 frequency limit need a working mail client, which the
reference stack lacks; they are unit tests in `crates/megabase-auth/src/recover.rs`.
