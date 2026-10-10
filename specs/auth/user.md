# Auth: user (issue 19)

Status: complete
Unit ids (`coverage/units.json`): `auth:route:GET /auth/v1/user`,
`auth:route:PUT /auth/v1/user`,
`auth:route:GET /auth/v1/user/identities/authorize`,
`auth:route:DELETE /auth/v1/user/identities/{identity_id}`,
`auth:route:GET /auth/v1/user/oauth/grants`,
`auth:route:DELETE /auth/v1/user/oauth/grants`
Level: 1

These routes read and update the caller identified by a bearer access
token. `GET /user` returns the reloaded user. `PUT /user` changes
metadata, the password, and an autoconfirmed phone. Identity linking and
the OAuth grant list stay the upstream 404 until their flags are on.
An email change, a phone change that must send SMS, a password change
that must reauthenticate, and an external OAuth redirect return 501.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Route table under `/user` | [`vendor/auth/internal/api/api.go:281`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L281) |
| Bearer check, user load, ban | [`vendor/auth/internal/api/auth.go:20`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/auth.go#L20) |
| `GET /user` and `PUT /user` | [`vendor/auth/internal/api/user.go:64`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/user.go#L64) |
| Request audience and admin check | [`vendor/auth/internal/api/helpers.go:21`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/helpers.go#L21) |
| Phone format | [`vendor/auth/internal/api/phone.go:27`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/phone.go#L27) |
| Metadata merge, email promotion, password logout | [`vendor/auth/internal/models/user.go:230`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/user.go#L230) |
| Autoconfirm phone change | [`vendor/auth/internal/api/verify.go:402`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/verify.go#L402) |
| Manual-linking and OAuth-server flags | [`vendor/auth/internal/api/middleware.go:396`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/middleware.go#L396) |
| Unlink and link | [`vendor/auth/internal/api/identity.go:18`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/identity.go#L18) |
| Provider name check | [`vendor/auth/internal/api/external.go:37`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/external.go#L37) |
| Disabled provider | [`vendor/auth/internal/conf/configuration.go:1369`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/conf/configuration.go#L1369) |
| Grant list and revoke | [`vendor/auth/internal/api/oauthserver/handlers.go:539`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/oauthserver/handlers.go#L539) |
| Consent order | [`vendor/auth/internal/models/oauth_consent.go:128`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/oauth_consent.go#L128) |

Pins: Auth `v2.197.0` (`4eee58f296d9698a1c2c0ae14d7a0b379c7622d3`, MIT).
`GOTRUE_SECURITY_MANUAL_LINKING_ENABLED` defaults false.
`GOTRUE_OAUTH_SERVER_ENABLED` defaults false.
`GOTRUE_SECURITY_UPDATE_PASSWORD_REQUIRE_REAUTHENTICATION` and
`GOTRUE_SECURITY_UPDATE_PASSWORD_REQUIRE_CURRENT_PASSWORD` default false.
`GOTRUE_JWT_ADMIN_GROUP_NAME` defaults to `admin`. The reference stack
sets mailer and phone autoconfirm.

## Inputs

Every route requires `Authorization: Bearer <access token>`. The token is
HS256, `sub` is a UUID, and a non-empty `session_id` must name a session
row. The user is reloaded by `sub`. A future `banned_until` is rejected.

`GET /auth/v1/user` has no body. The request audience is `X-JWT-AUD` when
that header is non-empty, otherwise the token audience (the first string
when `aud` is an array) unless `role` is in `GOTRUE_JWT_ADMIN_ROLES`,
otherwise `GOTRUE_JWT_AUD`.

`PUT /auth/v1/user` is JSON, at most `1 << 20` bytes. Fields: `email`,
`password`, `nonce`, `data`, `app_metadata`, `phone`, `channel`. Unknown
fields are ignored. A JSON `null` for `email`, `nonce`, `phone`, or
`channel` decodes as `""`. `password` stays absent when the field is
omitted or JSON `null`. `data` and `app_metadata` are objects; a JSON
`null` value deletes that key, and any other value replaces it.

`GET /auth/v1/user/identities/authorize` reads `provider` from the query
string. The name is lowercased.

`DELETE /auth/v1/user/identities/{identity_id}` takes the identity UUID
from the path.

`GET /auth/v1/user/oauth/grants` has no body.
`DELETE /auth/v1/user/oauth/grants` requires query `client_id`, a UUID.

Lookup and writes use parameters only.

## Outputs

`GET /user` is 200 `application/json` (no charset): the reloaded user,
including generated `confirmed_at` (`LEAST(email_confirmed_at,
phone_confirmed_at)`, nulls skipped). `factors` is omitted. The email
identity keeps `email_verified: false`.

`PUT /user` is 200 with that same user object after the write.

Served writes:

- `data` merges into `raw_user_meta_data`. `app_metadata` merges into
  `raw_app_meta_data`. The caller must have `aud` equal to `GOTRUE_JWT_AUD`
  and `role` equal to `GOTRUE_JWT_ADMIN_GROUP_NAME`.
- A non-empty `password` is bcrypt cost 10. It must differ from the stored
  hash. Other sessions are deleted (`LogoutAllExceptMe` when the token has
  `session_id`, otherwise every session). Pending confirmation, recovery,
  email-change, phone-change, and reauthentication tokens are cleared, and
  `auth.one_time_tokens` rows for the user are deleted. Audit action
  `user_updated_password`.
- A new phone, while `GOTRUE_SMS_AUTOCONFIRM` is true (the reference
  stack), is stored without a leading `+`, `phone_confirmed_at` is set,
  `is_anonymous` becomes false, and a `phone` identity is inserted or
  updated (`phone`, `phone_verified: true`). Audit action `user_modified`
  is also written for every successful update.

`DELETE /user/identities/{identity_id}` is 200 `{}`. The identity row is
deleted. A `phone` identity clears `phone` and `phone_confirmed_at`. Any
other identity keeps `users.email` when a remaining identity has that
email. Otherwise the primary identity is the first remaining one ordered
by verified email, then unverified email, then no email, then
`created_at`, then id. An empty or unverified email is unconfirmed unless
mailer autoconfirm is on. `app_metadata.providers` is rebuilt from
remaining identities ordered by `created_at`, and `provider` is the first
of that list. Audit action `identity_unlinked` records `identity_id`,
`provider`, and `provider_id`.

`GET /user/oauth/grants` is 200, an array ordered by `granted_at`
descending. Each item is `client` (`id`, and `name`, `uri`, `logo_uri`
when non-empty), `scopes` (the stored string split on spaces), and
`granted_at`. Revoked consents and deleted clients are omitted.

`DELETE /user/oauth/grants` is 204 with an empty body. The active consent
for that user and client gets `revoked_at`. Sessions with that
`oauth_client_id` are deleted. Audit action `token_revoked` records
`oauth_client_id` and `action` `revoke_oauth_grant`.

## Errors

Legacy API shape: `code` is the HTTP status integer, `error_code`, `msg`,
and header `x-sb-error-code`.

| When | Status | Code / message |
|---|---|---|
| Missing or malformed bearer | 401 | `no_authorization` / `This endpoint requires a valid Bearer token` |
| Bad signature or expired token | 403 | `bad_jwt` / the shared JWT failure text |
| Missing `sub` | 403 | `bad_jwt` / `invalid claim: missing sub claim` |
| `sub` is not a UUID | 400 | `bad_jwt` / `invalid claim: sub claim must be a UUID` |
| User row missing | 403 | `user_not_found` / `User from sub claim in JWT does not exist` |
| `session_id` is not a UUID, or the session row is missing | 403 | `bad_jwt` or `session_not_found` |
| `banned_until` in the future | 403 | `user_banned` / `User is banned` |
| `GET /user` or unlink audience mismatch | 400 or 403 | `validation_failed` on GET, `unexpected_audience` on unlink, message `Token audience doesn't match request audience` |
| `PUT` email or phone format | 400 | `validation_failed` / the signup email message, or `Invalid phone number format (E.164 required)` |
| `PUT` phone channel | 400 | `validation_failed` / the signup channel message |
| `PUT` password longer than 72 or shorter than the minimum | 400 or 422 | `validation_failed` / `Password cannot be longer than 72 characters`, or `weak_password` |
| `PUT` `app_metadata` without the admin group | 403 | `not_admin` / `Updating app_metadata requires admin privileges` |
| Verified MFA factor and the session is not AAL2, on email, phone, or password | 401 | `insufficient_aal` / `AAL2 session is required to update email or password when MFA is enabled.` |
| Anonymous user sets a password with no email and no phone in the body | 422 | `validation_failed` / `Updating password of an anonymous user without an email or phone is not allowed` |
| SSO user changes email, phone, password, or sends `nonce` | 422 | `user_sso_managed` / `Updating email, phone, password of a SSO account only possible via SSO` |
| Email already registered | 422 | `email_exists` / `A user with this email address has already been registered` |
| Phone already registered | 422 | `phone_exists` / `A user with this phone number has already been registered` |
| New password equals the old one | 422 | `same_password` / `New password should be different from the old password.` |
| Manual linking off | 404 | `manual_linking_disabled` / `Manual linking is disabled` |
| `identity_id` is not a UUID | 404 | `validation_failed` / `identity_id must be an UUID` |
| One identity left | 422 | `single_identity_not_deletable` / `User must have at least 1 identity after unlinking` |
| Identity id not on this user | 422 | `identity_not_found` / `Identity doesn't exist` |
| Promoted email belongs to someone else | 422 | `email_conflict_identity_not_deletable` / `Unable to unlink identity due to email conflict` |
| Unknown provider | 400 | `validation_failed` / `Unsupported provider: Provider {name} could not be found` |
| Known provider with its settings flag off | 400 | `validation_failed` / `Unsupported provider: provider is not enabled` |
| `custom:` while custom OAuth is off | 400 | `validation_failed` / `Unsupported provider: custom OAuth providers are disabled` |
| OAuth server off | 404 | `feature_disabled` / `OAuth server is disabled` |
| Revoke without `client_id` | 400 | `validation_failed` / `client_id query parameter is required` |
| `client_id` is not a UUID | 400 | `validation_failed` / `invalid client_id format` |
| No active consent | 404 | `oauth_consent_not_found` / `No active grant found for this client` |
| Body is not JSON, or larger than 1 MiB | 400 / 413 | same codes as signup |
| No `JWT_SECRET` | 500 | `unexpected_failure` / `Server lacks JWT secret` |
| Database failure | 500 | `unexpected_failure` / the GoTrue database message for that step |
| Email change, phone change while SMS autoconfirm is off, password change while reauthentication or the current password is required | 501 | `MEGABASE_NOT_IMPLEMENTED`, unit `auth:route:PUT /auth/v1/user` |
| Enabled provider, or `custom:` while custom OAuth is on | 501 | `MEGABASE_NOT_IMPLEMENTED`, unit `auth:route:GET /auth/v1/user/identities/authorize` |

Authentication runs before the feature-flag 404, matching the chi parent
middleware.

## Edge cases

- `PUT` checks the duplicate email or phone before it returns 501, so the
  row is unchanged.
- An empty `password` string is a weak password and does not clear the
  hash. GoTrue's strength check rejects it before `SetPassword("")`.
- Password strength on this route is the length floor and the 72-byte
  ceiling. Required characters and HIBP are unset on the reference stack.
- Phone `123` matches `^[1-9][0-9]{1,14}$`. `0123` does not.
- Unlink of the email identity while a second identity still has the same
  address leaves `users.email` in place.
- A deleted OAuth client is skipped in the grant list. A revoked consent
  is not listed and cannot be revoked again.
- In-memory `confirmed_at` uses the same `LEAST` rule as the generated
  column. Postgres reads the column.

## Out of scope

Mail is not sent: password-changed, phone-changed, identity-linked, and
identity-unlinked notifications. The reference stack has no mail server,
and GoTrue logs those failures without failing the HTTP call.

Not served, and answered with 501 on the unit above:

- email change (confirmation mail, PKCE, and the anonymous autoconfirm
  path)
- phone change while `GOTRUE_SMS_AUTOCONFIRM` is false
- password reauthentication (`nonce`) and the current-password check
- the external provider redirect, including `skip_http_redirect`
- creating an email identity on the first password
  (`GOTRUE_EXPERIMENTAL_CREATE_EMAIL_IDENTITY_ON_PASSWORD_SET_ENABLED`
  defaults off, so the reference stack skips it too)

Rate limits, captcha, hooks, and account-linking domains are not served.
Linking domains are empty on the reference stack, so duplicate email uses
the default domain only.

## Judge cases

No case in `judge/cases/` names these unit ids yet. Cases belong in a
separate `review/issue-19-cases` pull request. Each case has to pass on
the reference stack before the feature pull request is rebased onto it.
