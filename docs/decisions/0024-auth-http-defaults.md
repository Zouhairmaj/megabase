# Auth HTTP defaults follow the reference stack

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

Issue #15, recorded on `main` as decision 21 in #171. The judge starts
Megabase with `JWT_SECRET` and `DATABASE_URL` only, so unset `GOTRUE_*`
must match `vendor/supabase/docker/.env.example` plus
`judge/compose.override.yml` (`GOTRUE_MAILER_AUTOCONFIRM=true`), not
GoTrue's zero values. Email and phone providers default on, anonymous
users off, signup enabled, phone autoconfirm on, audience and default
group `authenticated`, expiry 3600, issuer `http://localhost:8000/auth/v1`.
Signup settings do not parse `GOTRUE_JWT_ADMIN_ROLES`. `/auth/v1/admin`
keeps the GoTrue unset default from issue #6
(`service_role,supabase_admin`). bcrypt cost is 10 (Go's `DefaultCost`).
An unset `JWT_SECRET` fails signup before insert. A present secret shorter
than 32 bytes aborts startup (decision in #170). Phone signup and email
signup with autoconfirm off stay HTTP 501. With anonymous users disabled,
anonymous signup returns HTTP 422 (`anonymous_provider_disabled`); when
anonymous users are enabled, it returns HTTP 501.
