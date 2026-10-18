# Verify and OTP limits

- Status: Accepted (agent)
- Date: 2026-10-18

`GET` and `POST /auth/v1/verify` use `GOTRUE_RATE_LIMIT_VERIFY` (GoTrue
default 30 per five minutes, burst 30). `POST /auth/v1/otp` and
`POST /auth/v1/magiclink` use `GOTRUE_RATE_LIMIT_OTP` (same shape). The key
matches GoTrue `performRateLimiting`: the leftmost `Sb-Forwarded-For`
address when `GOTRUE_SECURITY_SB_FORWARDED_FOR_ENABLED` is true (default
false) and that value is an IP, otherwise `GOTRUE_RATE_LIMIT_HEADER`. The
over-limit response is 429 `over_request_rate_limit` /
`Request rate limit reached`.

`GOTRUE_MAILER_OTP_EXP` stays configurable. Unset or `0` is 86400 seconds,
matching GoTrue `ApplyDefaults` and the reference stack. A positive value
is kept.

`MEGABASE_AUTH_OTP_MAX_FAILED_ATTEMPTS` defaults to `0`. That leaves a bad
code in place, matching GoTrue. A positive value, after that many failed
checks of one user, clears every pending verification token for that user
(confirmation, recovery, both email-change tokens, and the phone-change
token) and every `auth.one_time_tokens` row of that user. Later checks
return the same 403 `otp_expired` GoTrue returns for a bad or expired
token. Each count lives until that code's OTP deadline. The map is capped
at 1024 entries, dropping the soonest expiry, and storing a new code drops
that user's counts. The count is process-local and is not a column on
`auth.users`.
