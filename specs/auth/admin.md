# Auth: admin (issue #6, first batch)

Service-role admin API of GoTrue (`/auth/v1/admin/*`). This file covers the
units in issue #6. Other admin routes stay 501 until their issues land.
Pin: `vendor/auth` `v2.197.0` (`4eee58f296d9698a1c2c0ae14d7a0b379c7622d3`),
MIT.

All of these routes sit behind `requireAdminCredentials`
(`internal/api/middleware.go:187`) then `requireAdmin`
(`internal/api/auth.go:53`). Kong strips `/auth/v1` before GoTrue; Megabase
keeps the full path and matches on `/auth/v1/admin/...`.

## Shared: admin credentials

Upstream: `internal/api/auth.go:73` (`extractBearerToken`),
`internal/api/auth.go:83` (`parseJWTClaims`), `internal/api/auth.go:53`.

**Inputs.** `Authorization: Bearer <jwt>`. HS256 with `JWT_SECRET`. Admin
roles default to `service_role` and `supabase_admin`
(`internal/conf/configuration.go:1136`), overridable with
`GOTRUE_JWT_ADMIN_ROLES` (comma-separated). Session-bearing user JWTs
(`session_id` set and not nil) are out of this batch's judge cases; the
demo anon / service_role keys have no `session_id`.

**Errors (legacy API version, no `X-Supabase-Api-Version`).** Body shape
`{"code": <status>, "error_code": "...", "msg": "..."}` plus
`x-sb-error-code` (`internal/api/apierrors/apierrors.go:48`,
`internal/api/errors.go:151`).

| Status | `error_code` | `msg` | When |
|---|---|---|---|
| 401 | `no_authorization` | `This endpoint requires a valid Bearer token` | Missing / non-Bearer `Authorization` |
| 403 | `bad_jwt` | `invalid JWT: unable to parse or verify signature, …` | Signature / parse failure |
| 403 | `not_admin` | `User not allowed` | Valid JWT whose `role` is not an admin role |

Anon key is a valid JWT with `role=anon`, so it is 403 `not_admin`, not 401.

---

## `auth:route:GET /auth/v1/admin/audit`

Upstream: `internal/api/api.go:363`, handler `internal/api/audit.go:17`.

**Inputs.** Query `page` (default 1), `per_page` (default 50), optional
`query` as `author:<text>`, `action:<text>`, or `type:<text>`
(`filterColumnMap` in `audit.go:11`). `author` matches
`payload->>'actor_username'` or `payload->>'actor_name'` ILIKE.

**Output.** `200` JSON array of `{id, payload, created_at, ip_address}`
(`internal/models/audit_log_entry.go:94`), newest `created_at` first,
`instance_id = 00000000-0000-0000-0000-000000000000`. Headers
`X-Total-Count` and `Link` (`internal/api/pagination.go:34`). `Link` uses
the GoTrue-visible path `/admin/audit` (Kong has already stripped
`/auth/v1`). Empty result is `[]` with `X-Total-Count: 0` and
`Link: </admin/audit?page=0>; rel="last"`.

**Errors.** 400 `validation_failed` `Bad Pagination Parameters:
strconv.ParseUint: parsing "{value}": invalid syntax` if `page` /
`per_page` are not unsigned integers. 400 `validation_failed`
`Invalid query scope: {query}` if `query` is present and the scope is
not `author`, `action`, or `type`. An empty value after the colon
applies no filter (`FindAuditLogEntries`).

---

## `auth:route:GET /auth/v1/admin/custom-providers`

Upstream: `internal/api/api.go:434`, `requireCustomOAuthEnabled`
(`middleware.go:412`, default **enabled**), handler
`internal/api/custom_oauth_admin.go:75`.

**Inputs.** Optional `?type=oauth2` or `?type=oidc`.

**Output.** `200` `{"providers":[…]}`. Empty list is `"providers":[]` (not
omitted). Order `created_at DESC`. Client secret is never serialized
(`json:"-"`).

**Errors.** 404 `feature_disabled` `Custom OAuth providers are disabled`
when `GOTRUE_CUSTOM_OAUTH_ENABLED=false`. 400 `validation_failed`
`type must be either 'oauth2' or 'oidc'` for any other `type`.

---

## `auth:route:GET /auth/v1/admin/custom-providers/{identifier}`

Upstream: `internal/api/api.go:438`, handler `custom_oauth_admin.go:112`.

**Inputs.** Path `identifier`. Must start with `custom:`.

**Output.** `200` one provider object (secret omitted).

**Errors.** Same feature-disabled 404. 400 `validation_failed`
`identifier must start with 'custom:' prefix, e.g. 'custom:{identifier}'`
if the prefix is missing. 404 `custom_provider_not_found`
`Custom OAuth provider not found`.

---

## `auth:route:DELETE /auth/v1/admin/custom-providers/{identifier}`

Upstream: `internal/api/api.go:440`, handler `custom_oauth_admin.go:363`.

**Inputs.** Same identifier rules as GET.

**Output.** `204` empty body after delete.

**Errors.** Same validation / not-found / feature-disabled as GET.

---

## `auth:route:GET /auth/v1/admin/oauth/clients`

Upstream: `internal/api/api.go:418`, `requireOAuthServerEnabled`
(`middleware.go:404`, default **disabled**), handler
`internal/api/oauthserver/handlers.go:232`.

**Output when disabled (self-hosted / judge default).** 404
`feature_disabled` `OAuth server is disabled`.

**Output when `GOTRUE_OAUTH_SERVER_ENABLED=true`.** `200` JSON. Empty list
is `{}` because `clients` is `omitempty`. Rows: `auth.oauth_clients`
where `deleted_at IS NULL`, `created_at DESC`. Each client is
`OAuthServerClientResponse` (`handlers.go:30`): `client_id`,
`client_type`, `redirect_uris`, `token_endpoint_auth_method`,
`grant_types`, `response_types` (always `["code"]`), `client_name`,
`client_uri`, `logo_uri`, `registration_type`, `created_at`,
`updated_at`. Secret is never listed. Empty slices and empty strings
are omitted (`omitempty`).

---

## `auth:route:DELETE /auth/v1/admin/oauth/clients/{client_id}`

Upstream: `internal/api/api.go:424`, load
`oauthserver/handlers.go:79`, delete `handlers.go:198`.

**Output when disabled.** Same 404 `feature_disabled` as list (middleware
runs first).

**When enabled.** `client_id` must be a UUID (400 `validation_failed`
`invalid client_id format` otherwise). Missing client: 404
`oauth_client_not_found` `OAuth client not found`. Success: `204`.

---

## `auth:route:DELETE /auth/v1/admin/sso/providers/{idp_id}`

Upstream: `internal/api/api.go:406`, load `internal/api/ssoadmin.go:25`,
delete `ssoadmin.go:449`. No `requireSAMLEnabled` on this admin group.

**Inputs.** `idp_id` is a UUID, or `resource_<resource_id>`.

**Output.** `200` the provider JSON (`id`, `resource_id`, `disabled`,
`saml`, `domains`, timestamps) after destroy.

**Errors.** 404 `sso_provider_not_found` `SSO Identity Provider not found`
if the UUID is invalid or no row exists.

---

## `auth:route:DELETE /auth/v1/admin/users/{user_id}`

Upstream: `internal/api/api.go:390`, load `internal/api/admin.go:52`,
handler `admin.go:595`.

**Inputs.** `user_id` UUID. Optional JSON `{ "should_soft_delete": true }`.
Empty body is hard delete (`ShouldSoftDelete` defaults false). Lookup:
`instance_id = 00000000-0000-0000-0000-000000000000 AND id = $user_id`
(`models/user.go:709`).

**Output.** `200` `{}`. Hard delete destroys the `auth.users` row. Soft
delete and the audit row run in one transaction (`admin.go:612`). Soft
delete hashes email / phone / email_change / phone_change with
`SHA-256(id || value)` as raw URL-safe Base64 (`user.go:1186`; phone
keeps 15 characters), sets `deleted_at`, clears tokens and metadata,
empties each identity's `identity_data` and hashes `provider_id`,
deletes one-time tokens, factors, WebAuthn credentials, and sessions.

**Errors.** 404 `validation_failed` `user_id must be an UUID`. 404
`user_not_found` `User not found`.

---

## `auth:route:DELETE /auth/v1/admin/users/{user_id}/factors/{factor_id}`

Upstream: `internal/api/api.go:376`, load user then
`admin.go:75` (`loadFactor`), handler `admin.go:661`.

**Inputs.** Both path ids are UUIDs. Factor must belong to that user
(`FindOwnedFactorByID`).

**Output.** `200` the factor JSON after destroy. In the same
transaction, `DowngradeSessionsToAAL1` (`factor.go:449`) deletes
`mfa_amr_claims` whose `authentication_method` is the factor type's
AMR (`totp`, `mfa/phone`, `mfa/webauthn`, `mfa/recovery_code`) for
sessions with that `factor_id`, then sets those sessions to `aal1`
and `factor_id = NULL`.

**Errors.** User load errors first (so a missing user is `user_not_found`,
not `mfa_factor_not_found`). Invalid `factor_id`: 404 `validation_failed`
`factor_id must be an UUID`. Missing factor: 404 `mfa_factor_not_found`
`Factor not found`.

---

## `auth:route:DELETE /auth/v1/admin/users/{user_id}/passkeys/{passkey_id}`

Upstream: `internal/api/api.go:384`, handler
`internal/api/passkey_admin.go:36`. User is loaded first. There is no
`requirePasskeyEnabled` on this admin route.

**Inputs.** `passkey_id` UUID. Row in `auth.webauthn_credentials` with
matching `user_id`.

**Output.** `204` empty body.

**Errors.** User load errors first. Invalid or missing passkey: 404
`validation_failed` `Passkey not found`
(`passkey_admin.go:45` and `:51`).

---

## Out of this issue

POST/PUT/GET for users, generate_link, SSO list/get/update, OAuth client
get/update/register, custom-provider create/update, factor update, passkey
list, and every non-admin Auth route remain `MEGABASE_NOT_IMPLEMENTED`.

## Judge cases

Cases for these unit ids live on `review/issue-6-cases` (protected
`judge/`). Visible cases use the anon key for 403 and the service role for
empty-list / not-found / feature-disabled paths so they pass on the
reference stack without creating fixtures this batch cannot insert.
