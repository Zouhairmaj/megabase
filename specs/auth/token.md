# Auth: token (issues 17 and 53)

Status: complete
Unit ids (`coverage/units.json`): `auth:route:POST /auth/v1/token`,
`auth:grant-type:password`, `auth:grant-type:refresh_token` (Level 1);
`auth:grant-type:pkce`, `auth:grant-type:id_token`,
`auth:grant-type:web3` (Level 2)

`POST /auth/v1/token` exchanges credentials for a session. Clients send
`grant_type` on the query string and a JSON body. The password grant
checks the stored bcrypt hash and returns an access token. The
refresh-token grant rotates the legacy 12-character token. The PKCE grant
redeems an auth code with its code verifier. The `id_token` grant signs in
with a provider's OIDC ID token, and the `web3` grant with a signed Solana
or Ethereum message. Each Level 2 grant has its own section below.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Route `POST /token` | [`vendor/auth/internal/api/api.go:268`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L268) |
| Grant dispatch | [`vendor/auth/internal/api/token.go:40`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/token.go#L40) |
| Password grant | [`vendor/auth/internal/api/token.go:71`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/token.go#L71) |
| Password strength on login | [`vendor/auth/internal/api/password.go:28`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/password.go#L28) |
| Refresh grant and token shape | [`vendor/auth/internal/api/token_refresh.go:20`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/token_refresh.go#L20) |
| Rotation, reuse, and errors | [`vendor/auth/internal/tokens/service.go:194`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/tokens/service.go#L194) |
| Email and phone lookup | [`vendor/auth/internal/models/user.go:699`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/user.go#L699) |
| bcrypt compare | [`vendor/auth/internal/models/user.go:467`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/user.go#L467) |
| Swap and family revoke | [`vendor/auth/internal/models/refresh_token.go:67`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/refresh_token.go#L67) |
| Phone normalization | [`vendor/auth/internal/api/phone.go:41`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/phone.go#L41) |
| Per-grant limiter (`web3` has its own) | [`vendor/auth/internal/api/token.go:44`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/token.go#L44) |
| PKCE grant | [`vendor/auth/internal/api/token.go:214`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/token.go#L214) |
| Flow state lookup and expiry | [`vendor/auth/internal/models/flow_state.go`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/flow_state.go) |
| Code verifier check | [`vendor/auth/internal/security/pkce.go:16`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/security/pkce.go#L16) |
| Authentication method parse | [`vendor/auth/internal/models/factor.go:117`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/factor.go#L117) |
| `id_token` grant | [`vendor/auth/internal/api/token_oidc.go:202`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/token_oidc.go#L202) |
| `id_token` provider resolution | [`vendor/auth/internal/api/token_oidc.go:32`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/token_oidc.go#L32) |
| Apple and Azure issuer detection | [`vendor/auth/internal/api/provider/apple.go:25`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/provider/apple.go#L25), [`azure.go:105`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/provider/azure.go#L105) |
| `web3` grant | [`vendor/auth/internal/api/web3.go:25`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/web3.go#L25) |
| OAuth error body | [`vendor/auth/internal/api/errors.go:207`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/errors.go#L207) |

Pins: Auth `v2.197.0` (`4eee58f296d9698a1c2c0ae14d7a0b379c7622d3`, MIT).
The reference stack does not set
`GOTRUE_SECURITY_REFRESH_TOKEN_ALGORITHM_VERSION`, so the algorithm is the
Go zero value `0` (legacy 12-character tokens). JWT expiry is 3600 seconds
(`GOTRUE_JWT_EXP` unset). Audience defaults to `authenticated`.

## Inputs

`POST /auth/v1/token`. `grant_type` is the query parameter (Go
`Request.FormValue` for a JSON body). The body is JSON, at most `1 << 20`
bytes. Unknown JSON fields are ignored. A JSON `null` for `email`, `phone`,
`password`, or `refresh_token` decodes as `""`, matching Go `encoding/json`
on a `string` field.

Password body: `email`, `phone`, `password`. Audience is `X-JWT-AUD` when
that header is non-empty, otherwise `GOTRUE_JWT_AUD`.

Refresh body: `refresh_token`.

Rate limit: once `grant_type` is known and before the body is read, the
request takes a token from the grant's limiter, keyed by the first value of
the `GOTRUE_RATE_LIMIT_HEADER` header (no header, no limit). `password`,
`refresh_token`, `id_token`, and `pkce` share the token limiter
(`GOTRUE_RATE_LIMIT_TOKEN_REFRESH` per five minutes, burst 30). `web3` has
its own (`GOTRUE_RATE_LIMIT_WEB3`, default 30, per five minutes, burst 30).

Lookup SQL uses parameters only. Email match is
`instance_id` nil, `lower(email)`, `aud`, `is_sso_user = false`. Phone
match is the same with `phone` after `formatPhoneNumber` (one leading `+`
removed, then spaces removed).

## Outputs

Success is 200 `application/json` (no charset): `access_token`,
`token_type` `bearer`, `expires_in` (the configured JWT lifetime, 3600 by
default, taken from one clock), `expires_at`, `refresh_token`, `user`.
The access token is HS256 (`typ` `JWT`) with `aal` `aal1` and `amr`
`[{"method":"password","timestamp":...}]`. Password login writes that AMR
on the new session. Refresh keeps the session's existing AMR timestamp.
The user object is reloaded, so `confirmed_at` is present. Postgres reads
the generated column. The in-memory store fills that same
`LEAST(email_confirmed_at, phone_confirmed_at)` value on the reload (nulls
skipped, as in PostgreSQL). The email identity still has
`email_verified: false` (the signup response had flipped that flag only in
memory).

Headers (not compared by the judge unless a case opts in):
`sb-auth-user-id`, `sb-auth-session-id`, `sb-auth-refresh-token-prefix`
(first five characters). A real rotation also sets
`sb-auth-refresh-token-reuse: false`.

Password success updates `auth.users.last_sign_in_at` only. It inserts a
new `auth.sessions` row (`aal1`), a legacy refresh token (`parent` null),
an `auth.mfa_amr_claims` row (`password`), and a `login` audit entry with
`traits.provider` `email` or `phone`.

Refresh success revokes the presented token, inserts a new 12-character
token with `parent` set to the old token and the same `session_id`, writes
`token_revoked` and `token_refreshed` audit rows, and sets
`auth.sessions.refreshed_at`. It does not change `last_sign_in_at`.
If the presented token is already revoked and is the parent of the current
active token, the response returns that active token and does not mint
another one.

A password that matches and is shorter than `GOTRUE_PASSWORD_MIN_LENGTH`
still returns 200, with `weak_password: {message, reasons: ["length"]}`.
A strong password still includes `weak_password: null`, because Go stores a
nil `*WeakPasswordError` in an `interface{}`. Refresh omits the field.
Required-character sets and HIBP are unset on the reference stack, so they
add no reasons.

## Errors

Legacy API shape: `code` is the HTTP status integer, `error_code`, `msg`,
and header `x-sb-error-code`.

| When | Status | Code / message |
|---|---|---|
| `grant_type` missing or unknown | 400 | `invalid_credentials` / `unsupported_grant_type` |
| Email and phone both set | 400 | `validation_failed` / `Only an email address or phone number should be provided on login.` |
| Neither email nor phone | 400 | `validation_failed` / `missing email or phone` |
| Email provider disabled | 422 | `email_provider_disabled` / `Email logins are disabled` |
| Phone provider disabled | 422 | `phone_provider_disabled` / `Phone logins are disabled` |
| Unknown user, empty password hash, or wrong password | 400 | `invalid_credentials` / `Invalid login credentials` |
| `banned_until` in the future (checked before the password compare) | 400 | `user_banned` / `User is banned` |
| Password matches and is longer than 72 bytes | 400 | `validation_failed` / `Password cannot be longer than 72 characters` |
| Email not confirmed | 400 | `email_not_confirmed` / `Email not confirmed` |
| Phone not confirmed | 400 | `phone_not_confirmed` / `Phone not confirmed` |
| Refresh token shorter than 12, not `^[a-z0-9]{12}$`, or a longer token that fails the version-2 checksum | 400 | `validation_failed` / `Refresh token is not valid` |
| Refresh token not in the store, or a version-2 token whose session has no HMAC key | 400 | `refresh_token_not_found` / `Invalid Refresh Token: Refresh Token Not Found` |
| Refresh token whose session row is gone | 400 | `session_not_found` / `Invalid Refresh Token: No Valid Session Found` |
| Banned user on refresh | 400 | `user_banned` / `Invalid Refresh Token: User Banned` |
| Revoked refresh token that is not the parent of the active token (reuse interval 0, rotation on; the family is revoked) | 400 | `refresh_token_already_used` / `Invalid Refresh Token: Already Used` |
| Body is not JSON, or larger than 1 MiB | 400 / 413 | same codes as signup (`bad_json`, `request_entity_too_large`) |
| No database | 500 | `unexpected_failure` / `Database error querying schema` |
| No `JWT_SECRET` | 500 | `unexpected_failure` / `Server lacks JWT secret` |
| Over the grant's rate limit (see Inputs) | 429 | `over_request_rate_limit` / `Request rate limit reached` |

The PKCE, `id_token`, and `web3` errors are listed in their sections.

## Edge cases

- Email is lowercased before lookup. Phone keeps digits after the `+` and
  spaces are stripped.
- A banned user gets `user_banned` even when the password is wrong. A user
  with an empty password hash gets `invalid_credentials` even when banned.
- bcrypt comparison uses the first 72 bytes, matching Go. The 72-byte limit
  is then applied to the original password, so a longer password that
  matches the prefix is `validation_failed`, not a session.
- The reference stack does not enable database encryption, so a bcrypt cost
  other than 10 is not rewritten on login. GoTrue only persists that
  rewrite when encryption asks for it.
- `expires_in` is the configured lifetime. It is not recomputed from a
  second clock read, which in GoTrue can yield 3599 across a second boundary.
- Version-2 refresh tokens are not issued. A structurally valid one is
  looked up as a legacy token string, misses, and returns
  `refresh_token_not_found`, which is what GoTrue returns when the session
  has no HMAC key.
- Presenting the parent of the active refresh token returns the active
  token. Presenting an older revoked token revokes the family and returns
  `refresh_token_already_used`.

## PKCE grant (`grant_type=pkce`)

Body (`PKCEGrantParams`): `auth_code`, `code_verifier`.

1. Both fields must be non-empty.
2. `auth.flow_state` row where `auth_code` matches (`FindFlowStateByAuthCode`).
   No row, or a row with a NULL `user_id`, is not found.
3. Expiry (`FlowState.IsExpired`, 300 s, GoTrue's floor for
   `GOTRUE_EXTERNAL_FLOW_STATE_EXPIRY_DURATION`): a `magiclink` flow with
   `auth_code_issued_at` ages from that time; every other flow from
   `created_at`. A NULL timestamp is always expired.
4. The user is loaded by `user_id`.
5. `VerifyPKCE`: a flow without a code challenge fails. Method `s256`
   (case-insensitive) compares the challenge with base64url-no-pad of
   SHA-256 of the verifier; `plain` compares the verifier itself. Both use
   a constant-time compare.
6. `authentication_method` is parsed (`ParseAuthenticationMethod`: any
   value ending in `signup` is `email/signup`; `anonymous` and unknown
   values are errors). It becomes the session's AMR method and the custom
   access token hook's `authentication_method`.
7. One transaction: delete the flow state row, set `last_sign_in_at`,
   write a `login` audit entry with `traits.provider_type`, insert the
   `aal1` session, legacy refresh token and AMR claim, then run the hook.

Success is the session response above, without `weak_password`, plus
`provider_token` and `provider_refresh_token` from the flow state when
they are non-empty.

| When | Status | Code / message |
|---|---|---|
| `auth_code` or `code_verifier` empty | 400 | `validation_failed` / `invalid request: both auth code and code verifier should be non-empty` |
| No flow state for the code, or its `user_id` is NULL | 404 | `flow_state_not_found` / `invalid flow state, no valid flow state found` |
| Flow state expired | 422 | `flow_state_expired` / `invalid flow state, flow state has expired` |
| Verifier does not match | 400 | `bad_code_verifier` / `code challenge does not match previously saved code verifier` |
| Stored method neither `s256` nor `plain` | 400 | `bad_code_verifier` / `code challenge method not supported` |
| Flow has no code challenge | 400 | `bad_code_verifier` / `PKCE verification not applicable for implicit flow` |
| User row gone, or unparseable `authentication_method` | 500 | `unexpected_failure` / `Unexpected failure, please check server logs for more information` |

Edge case: the flow state is deleted first in the transaction and the
delete must hit a row. A second request racing on the same code gets
`flow_state_not_found` instead of a second session. GoTrue reads, issues,
then deletes, so two concurrent requests could both succeed there; this is
not observable by a sequential judge.

## `id_token` grant (`grant_type=id_token`)

Body (`IdTokenGrantParams`): `id_token`, `access_token`, `nonce`,
`provider`, `client_id`, `issuer` (strings), `link_identity` (bool).

Request errors are `OAuthError`: HTTP 400, body
`{"error": ..., "error_description": ...}`, no `x-sb-error-code` header.

| Order | When | Response |
|---|---|---|
| 1 | `id_token` empty | `invalid request` / `id_token required` |
| 2 | `provider` empty and `client_id` or `issuer` empty | `invalid request` / `provider or client_id and issuer required` |
| 3 | `link_identity` and no `Authorization` header | `invalid request` / `Linking requires a valid user access token in Authorization` |
| 4 | `link_identity` | `requireAuthentication` (the `GET /user` checks and errors) |

Provider resolution (`getProvider`), first match wins:

| Match | Issuer used in errors | Enabled flag |
|---|---|---|
| `provider` `apple`, or `issuer` `https://appleid.apple.com` / `https://account.apple.com` | `iss` read from the unverified token | `GOTRUE_EXTERNAL_APPLE_ENABLED` |
| `google`, or `https://accounts.google.com` | that issuer | `..._GOOGLE_ENABLED` |
| `azure`, or `issuer` matching `^https://login[.]microsoftonline[.]com/([^/]+)/v2[.]0/?$` | `iss` from the token | `..._AZURE_ENABLED` |
| `facebook`, or `https://www.facebook.com` | that issuer | `..._FACEBOOK_ENABLED` |
| `keycloak`, or Keycloak enabled with a non-empty `GOTRUE_EXTERNAL_KEYCLOAK_URL` equal to `issuer` | the Keycloak URL | `..._KEYCLOAK_ENABLED` |
| `kakao`, or `https://kauth.kakao.com` | that issuer | `..._KAKAO_ENABLED` |
| `vercel_marketplace`, or `https://marketplace.vercel.com` | that issuer | `..._VERCEL_MARKETPLACE_ENABLED` |
| `snapchat`, or `https://accounts.snapchat.com/accounts/oauth2/token` | that issuer | `..._SNAPCHAT_ENABLED` |
| `provider` starting with `custom:` | — | `auth.custom_oauth_providers` row |
| `issuer` in `GOTRUE_EXTERNAL_ALLOWED_ID_TOKEN_ISSUERS` (comma-separated; empty is the Apple and Google issuers) | `issuer` | always enabled |

Apple and Azure read `iss` from the token without verifying it: three
`.`-separated segments, the middle one base64url without padding (CR and
LF skipped, non-zero trailing bits accepted), and a JSON payload that Go
`json.Unmarshal` accepts into `struct{Issuer string "json:\"iss\""}`: an
object or `null`, with `iss` matched case-insensitively (also through the
`ſ` fold) and a string or `null` value. A later matching key overwrites
an earlier one.

| When | Status | Code / message |
|---|---|---|
| Apple: `iss` cannot be read | 400 | `validation_failed` / `Unable to detect issuer in ID token for Apple provider` |
| Apple: `iss` is not an Apple issuer | 400 | `validation_failed` / `Detected ID token issuer is not an Apple ID token issuer` |
| Apple or Azure: `issuer` set and not equal to `iss` | 400 | `validation_failed` / `Provided issuer does not match ID token issuer` |
| Azure: `iss` cannot be read | 400 | `validation_failed` / `Unable to detect issuer in ID token for Azure provider` |
| Azure: `iss` starts with neither `https://login.microsoftonline.com/` nor `https://sts.windows.net/` | 400 | `validation_failed` / `Detected ID token issuer is not an Azure ID token issuer` |
| `custom:` with `GOTRUE_CUSTOM_OAUTH_ENABLED=false` | 400 | `validation_failed` / `Custom OAuth providers are disabled` |
| `custom:` row missing | 400 | `validation_failed` / `Custom provider "<provider>" not found` |
| `custom:` lookup fails (no database) | 500 | `unexpected_failure` / `Error finding custom provider` |
| `custom:` row disabled | 400 | `provider_disabled` / `Custom provider "<provider>" is disabled` |
| `custom:` row is `oauth2` | 400 | `validation_failed` / `Provider "<provider>" is not an OIDC provider` |
| `custom:` row has no issuer | 400 | `validation_failed` / `OIDC provider "<provider>" missing issuer` |
| No branch matches | 400 | `validation_failed` / `Custom OIDC provider "<provider>" not allowed` |
| Matched provider is disabled | 400 | `provider_disabled` / `Provider (issuer "<issuer>") is not enabled` |
| Matched provider is enabled | 501 | `MEGABASE_NOT_IMPLEMENTED` / `auth:grant-type:id_token` |

Quoted values use Go `%q`. The reference stack enables no external
provider, so every request there ends in one of the 400 rows. After an
enabled provider, GoTrue fetches OIDC discovery and JWKS, verifies the
token, audience, and nonce, creates or links the user, and issues an
`oauth` session. Megabase makes no outbound OIDC requests yet, so that is
the 501.

## `web3` grant (`grant_type=web3`)

Body (`Web3GrantParams`): `message`, `signature`, `chain`.

| Order | When | Status | Code / message |
|---|---|---|---|
| 1 | `GOTRUE_EXTERNAL_WEB3_SOLANA_ENABLED` and `..._ETHEREUM_ENABLED` both off (checked before the body is read) | 422 | `web3_provider_disabled` / `Web3 provider is disabled` |
| 2 | Body is not JSON or a field has the wrong type | 400 | `bad_json` (`Web3GrantParams`) |
| 3 | `chain` is neither `solana` nor `ethereum` | 400 | `web3_unsupported_chain` / `Unsupported chain` |
| 4 | `solana` while Solana is off | 422 | `web3_provider_disabled` / `Solana Web3 provider is disabled` |
| 4 | `ethereum` while Ethereum is off | 422 | `web3_provider_disabled` / `Ethereum Web3 provider is disabled` |
| 5 | `message` under 64 bytes | 400 | `validation_failed` / `message is too short` |
| 5 | `message` over 20 KiB | 400 | `validation_failed` / `message must not exceed 20KB` |
| 6 | `solana`: `signature` length neither 86 nor 88 | 400 | `validation_failed` / `signature must be 64 bytes encoded as base64 with or without padding` |
| 6 | `solana`: `signature` (trailing `=` trimmed, `+/` mapped to `-_`) is not raw base64url | 400 | `validation_failed` / `signature does not contain valid base64 characters` |
| 6 | `ethereum`: `signature` without a `0x` prefix is not 130 bytes | 400 | `validation_failed` / `signature must be 65 bytes long, encoded as a 130 character-long hexadecimal string` |
| 7 | Fields pass | 501 | `MEGABASE_NOT_IMPLEMENTED` / `auth:grant-type:web3` |

The reference stack enables neither chain, so every `web3` request there
is the first row. Sign in with Solana (ed25519) and Sign in with Ethereum
(EIP-4361, secp256k1) message parsing and signature checks are the 501.

## Out of scope

Captcha, password-verification hooks, HIBP, session timebox,
single-session-per-user, and refresh-token algorithm version 2 issuance
are not served. The reference stack leaves those flags off. The
`id_token` and `web3` verification steps after an enabled provider are
HTTP 501 (see their sections).

## Judge cases

Already in `judge/cases/auth.toml` (not edited here): `auth.token.password`,
`auth.token.wrong-password`, `auth.token.refresh`. Each case signs up
first, so it also depends on `POST /signup` from issue 15.

Proposed for the Level 2 grants (protected-path PR on
`review/issue-53-cases`): PKCE empty body (400 `validation_failed`),
unknown `auth_code` (404 `flow_state_not_found`); `id_token` empty body
(400 `id_token required`), `provider: "google"` (400 `provider_disabled`),
arbitrary `issuer` with `client_id` (400 `Custom OIDC provider "" not
allowed`), `provider: "custom:missing"` (400 not found), Apple with a
malformed token (400 `Unable to detect issuer ...`); `web3` with any body
(422 `web3_provider_disabled`). A successful PKCE exchange needs a flow
state with an auth code, which the reference stack only creates through
PKCE signup, magic link, or OAuth flows.
