# Feature auth tables take the feature's level

- Status: Accepted (Zouhair, 2026-10-19)
- Date: 2026-10-19

This file is 0046. On `main`, 0045 is both
[0045-verify-otp-limits.md](0045-verify-otp-limits.md) from
[#483](https://github.com/Zouhairmaj/megabase/pull/483) and
[0045-bootstrap-supabase-admin-roles.md](0045-bootstrap-supabase-admin-roles.md)
from [#498](https://github.com/Zouhairmaj/megabase/pull/498). The next free
number is 0046.

## Context

`tools/megabase-coverage` classified every `auth.*` SQL table as Level 1.
Nineteen of those tables belong to features GOAL.md §7 places after Level 1:
`mfa_*`, `oauth_*`, `saml_*`, `sso_*`, `scim_*`, `webauthn_*`, and
`custom_oauth_providers`.

GOAL.md §7 names Level 1 Auth as `/auth/v1` email/password, JWT,
`auth.users`, `auth.uid()`, and `auth.jwt()`. Level 2 is OAuth providers,
magic links, OTP, and Storage. MFA, SAML, SSO, SCIM, passkeys, and custom
OAuth providers sit with those later Auth features. Their HTTP routes are
already Level 2 in the extractor.

[Decision 0036](0036-level-1-validation-requirements.md) requires three
judge cases for every in-scope Level 1 unit. Classifying these tables as
Level 1 would count them in that minimum.

## Decision

An `auth.*` table takes the level of the feature that owns it.

These nineteen tables are Level 2:

- `auth.mfa_amr_claims`, `auth.mfa_challenges`, `auth.mfa_factors`,
  `auth.mfa_recovery_code_sets`, `auth.mfa_recovery_codes`
- `auth.oauth_authorizations`, `auth.oauth_client_states`,
  `auth.oauth_clients`, `auth.oauth_consents`
- `auth.saml_providers`, `auth.saml_relay_states`
- `auth.sso_domains`, `auth.sso_providers`, `auth.sso_sessions`
- `auth.scim_tokens`, `auth.scim_users`
- `auth.webauthn_challenges`, `auth.webauthn_credentials`
- `auth.custom_oauth_providers`

The extractor applies that by name: prefixes `mfa_`, `oauth_`, `saml_`,
`sso_`, `scim_`, and `webauthn_`, plus the exact name
`custom_oauth_providers`. A later table with one of those prefixes is
Level 2.

Email/password and JWT objects stay Level 1. That is `auth.users` and the
functions `auth.uid()` and `auth.jwt()`, plus the session objects those
flows already use: `auth.identities`, `auth.sessions`,
`auth.refresh_tokens`, `auth.audit_log_entries`, `auth.one_time_tokens`,
`auth.flow_state`, `auth.instances`, `auth.schema_migrations`,
`auth.role()`, and `auth.email()`.

The tables stay in `coverage/units.json` at Level 2. The unit denominator
is unchanged. [0036](0036-level-1-validation-requirements.md) still requires
three judge cases for every unit whose level is 1.

## Consequences

The Level 1 Auth database epic lists the email/password and JWT objects.
Level 2 owns judge cases for the nineteen tables above. `docs/ROADMAP.md`
and the Level 1 section of `PROGRESS.md` point here.
