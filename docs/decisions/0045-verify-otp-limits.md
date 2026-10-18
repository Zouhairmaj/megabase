# Verify and OTP limits

- Status: Accepted (agent)
- Date: 2026-10-18

`GET` and `POST /auth/v1/verify` use `GOTRUE_RATE_LIMIT_VERIFY` (GoTrue
default 30 per five minutes, burst 30). `POST /auth/v1/otp` and
`POST /auth/v1/magiclink` use `GOTRUE_RATE_LIMIT_OTP` (same shape). The key
is a parsed `Sb-Forwarded-For` address, otherwise `X-Forwarded-For`,
otherwise `GOTRUE_RATE_LIMIT_HEADER`. A request with an email or phone also
spends a bucket on that address. The over-limit response is 429
`over_request_rate_limit` / `Request rate limit reached`.

Five failed checks of one stored email or phone code clear that code. Later
checks return the same 403 `otp_expired` GoTrue returns for a bad or expired
token. The count is process-local and keyed by the stored token text, so a
newly issued code starts again. It is not a column on `auth.users`.

`GOTRUE_MAILER_OTP_EXP` stays configurable. Unset or `0` is 3600 seconds.
GoTrue's `ApplyDefaults` uses 86400 when the value is `0`; Supabase project
config sets `otp_expiry` / `MAILER_OTP_EXP` to 3600, which is the default
here. A positive setting is kept, including `86400`.
