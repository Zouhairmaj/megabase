# Auth: admin (issues #6 and #7)

Service-role admin API of GoTrue (`/auth/v1/admin/*`). This file covers the
units in issues #6 and #7. Later admin routes stay 501 until their issues land.
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

## `auth:route:GET /auth/v1/admin/users`

Upstream: `internal/api/api.go:367` (`adminUsers`), offset pagination
`internal/api/pagination.go`, sort `internal/api/sorting.go`.

**Inputs.** Admin JWT. Query `page` (default 1) and `per_page` (default 50)
are Go `ParseUint`. `sort` is repeated `created_at` plus optional `asc` or
`desc` (default `DESC`). `filter` is `email LIKE %f% OR raw_user_meta_data->>'full_name' ILIKE %f%`.
Audience is `requestAud`: `X-JWT-AUD`, else `GOTRUE_JWT_AUD` (default
`authenticated`). Rows are `instance_id` nil and that `aud`. Soft-deleted
users stay in the list.

**Output.** `200` `{"users":[...],"aud":"..."}`. Each user is the GoTrue user
object. List does not eager-load identities, so `identities` is JSON `null`
and empty `factors` are omitted. `Link` uses the Kong-stripped path
(`/admin/users?...`) with Go query encoding (sorted keys, space as `+`) and
`rel="next"` when another page exists, plus `rel="last"`. `X-Total-Count` is
the filtered count. Zero users and `per_page=1` yields
`</admin/users?page=0&per_page=1>; rel="last"` and `X-Total-Count: 0`.

**Cursor mode.** Only when `GOTRUE_EXPERIMENTAL_CURSOR_PAGINATION_ENABLED` is
true and neither `page` nor `per_page` is set. `limit` default 50, max 1000,
must be greater than 0. Cursor is base64url JSON `{created_at,id}`. No
`X-Total-Count`. `Link` `rel="next"` only when another row exists. Body adds
`pagination: {has_more, next_cursor?}`.

**Errors.** Bad page or per_page: 400 `validation_failed`
`Bad Pagination Parameters: strconv.ParseUint: parsing "{value}": invalid syntax`.
Bad sort field: `Bad Sort Parameters: bad field for sort '{field}'`.
Bad direction: `bad direction for sort '{dir}', only 'asc' and 'desc' allowed`.
`limit` 0 in cursor mode: `Bad Pagination Parameters: limit must be greater than 0`.

---

## `auth:route:GET /auth/v1/admin/users/{user_id}`

Upstream: `internal/api/api.go:388` (`adminUserGet`) after `loadUser`
(`admin.go`).

**Inputs.** `user_id` UUID.

**Output.** `200` the user object itself. Identities are loaded (empty is
`[]`). Factors are included when present.

**Errors.** Invalid id: 404 `validation_failed` `user_id must be an UUID`.
Missing user: 404 `user_not_found` `User not found`.

---

## `auth:route:GET /auth/v1/admin/users/{user_id}/factors`

Upstream: `internal/api/api.go:373` (`adminUserGetFactors`).

**Inputs.** User is loaded first. No extra feature gate.

**Output.** `200` a JSON array. `secret` is omitted. `friendly_name`,
`web_authn_aaguid`, and `last_webauthn_challenge_data` are omitted when empty.
`phone` and `last_challenged_at` are always present (`null` when unset).
No rows is `[]`.

**Errors.** Same user-load errors as GET user.

---

## `auth:route:GET /auth/v1/admin/users/{user_id}/passkeys`

Upstream: `internal/api/api.go:382` (`AdminPasskeyList`). No
`requirePasskeyEnabled`.

**Inputs.** User is loaded first.

**Output.** `200` `[]` of `{id, friendly_name?, created_at, last_used_at?}`
ordered by `created_at ASC` from `auth.webauthn_credentials`.

**Errors.** Same user-load errors as GET user.

---

## `auth:route:GET /auth/v1/admin/sso/providers`

Upstream: `internal/api/api.go:398` (`adminSSOProvidersList`). No
`requireSAMLEnabled`.

**Inputs.** Optional `resource_id` (exact) else `resource_id_prefix`
(`LIKE prefix%`).

**Output.** `200` `{"items":[...]}`. List clears `metadata_xml` (omitted).
Each item has `id`, optional `resource_id`, `disabled` (null or bool),
optional `saml`, `domains` (`[{domain}]` or `[]`), `created_at`, `updated_at`.

---

## `auth:route:GET /auth/v1/admin/sso/providers/{idp_id}`

Upstream: `internal/api/api.go:404` after `loadSSOProvider` (`ssoadmin.go`).

**Inputs.** UUID, or a `resource_` prefixed resource id.

**Output.** `200` the provider, including `metadata_xml` when non-empty.

**Errors.** Non-uuid (unless `resource_` prefix) or missing row: 404
`sso_provider_not_found` `SSO Identity Provider not found`. The uuid check
happens before the database.

---

## `auth:route:GET /auth/v1/admin/oauth/clients/{client_id}`

Upstream: `internal/api/api.go:422` (`OAuthServerClientGet`).
`requireOAuthServerEnabled` runs first (`GOTRUE_OAUTH_SERVER_ENABLED`, default
false) so a bad id is still 404 `feature_disabled` `OAuth server is disabled`.

**Inputs.** `client_id` UUID. Deleted rows are excluded.

**Output.** `200` the client JSON from issue #6 (`client_secret` omitted).

**Errors.** Empty id: 400 `validation_failed` `client_id is required`.
Bad UUID: 400 `invalid client_id format`. Missing: 404
`oauth_client_not_found` `OAuth client not found`.

---

## `auth:route:POST /auth/v1/admin/oauth/clients`

Upstream: `internal/api/api.go:416` (`AdminOAuthServerClientRegister`).
Feature flag first, then JSON. Decode failure is exactly `Invalid JSON body`
(`bad_json`), not the shared body-parser message. `registration_type` is
forced to `manual`.

**Inputs.** `redirect_uris` (required, max 10, absolute, no fragment; `http`
only for `localhost`, `127.0.0.1`, `::1`; schemes `javascript`, `data`,
`file`, `vbscript`, `about`, `blob` rejected). `grant_types` empty becomes
`authorization_code` and `refresh_token`; only those two are allowed.
`client_type` explicit or inferred from `token_endpoint_auth_method`
(`none` → public, otherwise confidential). Auth method explicit or `none`
for public and `client_secret_basic` for confidential. Disagreeing pair:
`client_type '{t}' is inconsistent with token_endpoint_auth_method '{m}' (expected client_type '{expected}')`.
`client_name` ≤ 1024. `client_uri` and `logo_uri` empty or a URL ≤ 2048.

**Output.** `201` client JSON. Confidential clients include `client_secret`
once (32 random bytes, base64 raw URL). The stored hash is SHA-256 of that
secret, base64 raw URL. Redirects and grants are comma-joined. Response
`response_types` is `["code"]`.

**Errors.** Validation is wrapped again, so the `msg` is `400: {reason}`
with `validation_failed`. Example: `400: redirect_uris is required` and
`400: invalid redirect_uri '{uri}': {reason}`. A database failure is 400
`failed to create OAuth client: ...`, not 500.

---

## `auth:route:POST /auth/v1/admin/custom-providers`

Upstream: `internal/api/api.go:435` (`adminCustomOAuthProviderCreate`).
`requireCustomOAuthEnabled` first (`GOTRUE_CUSTOM_OAUTH_ENABLED`, default
true).

**Inputs.** JSON. `provider_type` is `oauth2` or `oidc`. Identifier, name,
`client_id`, and `client_secret` are required. Identifier must start with
`custom:`. OIDC requires `issuer`. OAuth2 requires `authorization_url`,
`token_url`, and `userinfo_url`. Reserved authorization keys (`client_id`,
`client_secret`, `redirect_uri`, `response_type`, `state`, `code_challenge`,
`code_challenge_method`, `code_verifier`, `nonce`) are rejected. Mapping
targets `id`, `aud`, `role`, `app_metadata`, `created_at`, `updated_at`,
`confirmed_at`, `email_confirmed_at`, `phone_confirmed_at`, `email_verified`,
`phone_verified`, `banned_until`, and `is_super_admin` are rejected.
URLs must be HTTPS. The host is the authority after the last `@`
(Go `url.URL.Hostname`); a `\` or space in that host is rejected. The host
must not be localhost, and name lookup (non-blocking, 5 second timeout)
must not return loopback, private, link-local, multicast, or unspecified
addresses. IPv4-mapped IPv6 (`::ffff:a.b.c.d`) is checked as IPv4, matching
Go `net.IP.To4`. `pkce_enabled` defaults true, `enabled`
defaults true, `email_optional` defaults false. Empty maps are stored as `{}`.
Client secret is stored in plaintext when database encryption is disabled
(the default).

**Output.** `201` the reloaded provider JSON (empty arrays are `[]`).

**Errors.** Bad JSON: 400 `bad_json`
`Could not parse request body as JSON: {go message}` (empty body is
`unexpected end of JSON input`). Bad type: `provider_type must be either 'oauth2' or 'oidc'`.
Duplicate identifier: 400 `conflict`
`A custom OAuth provider with this identifier already exists`
(upstream `NewBadRequestError`, HTTP 400 with error code `conflict`, not 409).
OIDC create validates, then returns 400 `validation_failed`
`OIDC discovery from {quoted url} failed: OIDC discovery fetch is not available`.
Discovery over HTTPS needs a TLS client that is not on the dependency allow
list, so OIDC rows are not inserted. OAuth2 create does persist.
Database failure: 500 `Error creating custom OAuth provider`.

---

## `auth:route:POST /auth/v1/admin/generate_link`

Upstream: `internal/api/api.go:394` (`adminGenerateLink` in `mail.go`).

**Inputs.** `{type, email, new_email, password, data, redirect_to}`. Types:
`magiclink`, `recovery`, `invite`, `signup`, `email_change_current`,
`email_change_new`. Email is required, at most 255 characters, and must
contain a local part and a dotted domain. The redirect follows GoTrue
`GetReferrer` (`internal/utilities/request.go`), then a valid JSON
`redirect_to` replaces it. A non-empty `redirect_to` header wins over the
query `redirect_to` (percent-decoded, `+` as space) even when the header is
invalid; otherwise the query value is used. An invalid candidate falls
through to `Referer`, then `GOTRUE_SITE_URL` (default `http://localhost:3000`).
A URL is accepted when its scheme and host match the site URL and the port
matches, except a localhost host (`localhost`, `127.0.0.1`, `::1`, `0.0.0.0`,
`::`, or a `.localhost` suffix) may use another port. A loopback IP is
accepted. Any other IP, and an all-digit hostname, are rejected before the
allow list. An `http` or `https` hostname must match GoTrue's regular
hostname pattern. Otherwise the URL, with its fragment removed, must match
`GOTRUE_URI_ALLOW_LIST` (comma-separated globs; `ADDITIONAL_REDIRECT_URLS`
when that variable is unset or blank). `*` and `?` do not cross `.` or `/`;
`**` does. A pattern with an unescaped character class or `{` alternative is
not implemented. The external URL default is `http://localhost:8000/auth/v1`.
Mailer paths default to
`/auth/v1/verify`. OTP length defaults to 6 (clamped 6–10). Secure email
change defaults on. Reading `/dev/urandom` for the OTP or the generated
signup password fails the request when that device is unavailable.

**Output.** `200` the user object plus `action_link`, `email_otp`,
`hashed_token`, `verification_type`, and `redirect_to`. The link query is
`token`, `type`, and `redirect_to`. Email-change wire type is `email_change`.
Magic link uses the recovery path and query type `magiclink`. `hashed_token`
is SHA-224 hex of `email+otp` (`sha256.Sum224`). A missing user on
`magiclink` becomes `signup` with a generated password. Invite of a missing
user does not require a password. Signup of a missing user checks the
password (empty 400, longer than 72 400, shorter than the minimum 422
`weak_password` with reason `length`). New users are unconfirmed, role
`authenticated`, with an email identity.

**Errors.** Bad JSON uses the same `Could not parse request body as JSON`
message as custom providers. Empty email: 400 `validation_failed`
`An email address is required`. Bad format:
`Unable to validate email address: invalid format`. Unknown type, after the
user lookup: 400 `Invalid email action link type requested: {type}`.
Missing user for recovery or email change: 404 `user_not_found`
`User with this email not found`. Confirmed user on signup or invite, or a
duplicate new email: 422 `email_exists`
`A user with this email address has already been registered`.
`email_change_current` with secure email change off: 400
`Enable secure email change to generate link for current email`.
Body larger than 1 MiB: 413 `request_entity_too_large`. Random-byte
failure: 500 `unexpected_failure` `failed to generate random bytes`.
An allow-list pattern this build cannot evaluate: 501
`MEGABASE_NOT_IMPLEMENTED` for unit `GOTRUE_URI_ALLOW_LIST`. Same-origin and
loopback redirects do not consult that list.

---

## Issue #8 writes (`admin_batch3.rs`)

Upstream: `internal/api/admin.go` (`adminUserCreate`, `adminUserUpdate`,
`adminUserUpdateFactor`), `ssoadmin.go`, `custom_oauth_admin.go`,
`oauthserver/handlers.go`, `oauthserver/service.go`. All sit behind the admin
credential check above. Audit rows use a nil actor id and the admin role as
the username.

**`POST /admin/users`.** Needs an email or phone (400 `Cannot create a user
without either an email or phone`). Email uses the `generate_link` format
check, lower-cased; a duplicate (identity or user, same audience) is 422
`email_exists`. Phone drops one `+` and spaces, then must match
`^[1-9][0-9]{1,14}$` (400 `Invalid phone number format (E.164 required)`);
a duplicate is 422 `phone_exists`. `password` and `password_hash` together
are 400. A plaintext password is length-checked (max 72, minimum from config,
422 `weak_password`); with neither, a random 64 character password is stored.
Only bcrypt hashes are accepted; argon2 and Firebase scrypt hashes answer 501.
`id` must be a non-nil UUID. `ban_duration` is `none` or a Go duration. The
role defaults to `GOTRUE_JWT_DEFAULT_GROUP_NAME`. `app_metadata` merges over
`{provider, providers}`. `email_confirm` / `phone_confirm` confirm the contact
after the identity rows (email, phone) are written. Audit `user_signedup`.
**Output** `200` user JSON. A database failure is 500
`Database error creating new user`.

**`PUT /admin/users/{user_id}`.** The user is loaded first (404
`validation_failed` for a bad id, `user_not_found`). Then email, phone, ban
duration, and password are validated as above (a supplied password, even empty,
is strength-checked; empty clears it). In one transaction: role, confirms,
password (clears pending tokens, one-time tokens, and sessions), email and
phone identities (update `identity_data`, or create when missing), pending
token reset when email or phone changed, metadata merges (`null` deletes a
key), ban. Audit `user_modified`. Failure is 500 `Error updating user`.

**`PUT /admin/users/{user_id}/factors/{factor_id}`.** Loads user then factor
(404 `mfa_factor_not_found`). `friendly_name` and, for phone factors only,
`phone` (validated) are updated; audit `factor_updated`. Returns the factor.

**`POST` / `PUT /admin/sso/providers`.** Validation order follows
`CreateSSOProviderParams.validate`: type `saml` (create), one of
`metadata_xml` / `metadata_url`, HTTPS URL, `name_id_format`. Create rejects a
known EntityID (422 `saml_idp_already_exists`) and a taken domain (400
`sso_domain_already_exists`); 201 with the provider. Update diffs domains,
attribute mapping, name id format, resource id, and disabled; a changed
EntityID is 400 `saml_entity_id_mismatch`. A failed write is 422 `conflict`.
There is no XML parser or TLS client on the dependency allow list, so
`metadata_url` answers 501 and `metadata_xml` is scanned for the root
`EntityDescriptor`, its `entityID`, and one `IDPSSODescriptor`; XML that
scan cannot place also answers 501.

**`PUT /admin/oauth/clients/{client_id}`.** Same id and lookup errors as GET.
Bad JSON is 400 `Invalid JSON body`; no fields is 400 `No fields provided for
update`. Validation messages are the unwrapped `oauthserver/service.go`
strings (`redirect_uris cannot be empty`, and so on). A
`token_endpoint_auth_method` must suit the stored client type (public: `none`;
confidential: `client_secret_basic`, `client_secret_post`). Returns the client.

**`POST /admin/oauth/clients/{client_id}/regenerate_secret`.** Public clients
are 400 `Cannot regenerate secret for public client`. Otherwise a new 32 byte
base64 raw URL secret replaces the SHA-256 hash and is returned once.

**`PUT /admin/custom-providers/{identifier}`.** Same identifier checks as GET.
Only supplied fields change (`updateProviderFromParams`); OIDC scopes keep
`openid` first; URLs pass the HTTPS and address checks of create. For an OIDC
provider an `issuer` or `discovery_url` change needs discovery, which this
build cannot fetch, so a non-empty change answers 501
`MEGABASE_NOT_IMPLEMENTED`. Returns the provider.

---

## Still unimplemented

Every non-admin Auth route other than signup, logout, and the password and
refresh-token grants remains `MEGABASE_NOT_IMPLEMENTED`. Unimplemented
methods on a registered admin path also return that 501, not Axum 405.

## Judge cases

Issue #6 cases live on `review/issue-6-cases` and are on `main`.
`auth.admin.users.list` and `auth.admin.users.anon` already cover
`GET /auth/v1/admin/users`. New cases for the other nine units belong on
`review/issue-7-cases` (protected `judge/`). A `generate_link` case must not
compare the full body: OTP, token, user id, and timestamps change per call.
