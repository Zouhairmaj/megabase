# Adversarial Auth and RLS cases live in the judge

- Status: Accepted (agent coordinator)
- Date: 2026-10-10

Issue #117. Level 1 cannot be called done while a P0 Auth or RLS hole
is open, so the attacks are visible judge cases. The reference stack is
the oracle: a case records the request, and both stacks must answer the
same way.

The cases are `auth.adversarial.*` in `judge/cases/auth.toml` and
`rest.adversarial.*` in `judge/cases/rest.toml`. Fixtures
`public.profiles`, `public.notes`, and `public.note_comments` live in
`judge/fixtures/schema.sql`. Owner tokens are HS256 fixtures signed with
the public demo `JWT_SECRET` from `vendor/supabase/docker/.env.example`.
The forged token uses a different secret. The expired token uses the
demo secret with `exp` in 2001, so a verifier that skips expiry still
accepts it. `alg=none` is unsecured. The tampered token keeps the
owner signature after `role` was rewritten to `service_role`.

GoTrue's limiters run only when `GOTRUE_RATE_LIMIT_HEADER` is set and
the request carries it (`vendor/auth/internal/api/middleware.go`). The
judge overlay sets that name to `X-Megabase-Judge-Rate-Key`. Other cases
omit the header, so they are not counted. The rate-limit case sends
`judge-{{run}}`, one bucket per judge process. The pinned token limiter
burst is hardcoded at 30 (`internal/api/apilimiter/apilimiter.go`). The
default rate is `RateLimitTokenRefresh / (60 * 5)` per second, and the
default `RateLimitTokenRefresh` is 150, so one token refills about every
two seconds. A slow run can admit the 31st grant. The overlay sets
`GOTRUE_RATE_LIMIT_TOKEN_REFRESH=1`, which refills about once every five
minutes, so the 31st password grant is the 429. Lowering the burst would
mean editing `vendor/`, which agents cannot do.

`prepare` loads the fixtures into the empty `megabase` database before
Megabase installs Auth SQL. The file creates `auth.uid()` only when it
is missing, and does not replace the reference function that
`auth.sql.function.uid` compares.

The pinned cluster's default privileges grant `ALL` on new `public`
tables to `anon`. The fixture revokes that grant on the three tables,
then grants `anon` select on `profiles` only. Without the revoke, anon
would hold a table grant and RLS (no policy for anon) would hide rows
instead of returning permission denied.
