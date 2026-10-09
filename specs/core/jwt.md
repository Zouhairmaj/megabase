# Core: HS256 JWT validation and config

Shared `megabase-core` types so Auth and REST enforce the same JWT and role
claims as GoTrue and PostgREST. This is library code, not an HTTP unit.
Auth signup signs with `Hs256::sign` and logout verifies with `Hs256::verify`.
REST handlers still return 501.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Parse Bearer, verify signature, HS256 secret fallback | [`vendor/auth/internal/api/auth.go:73`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/auth.go#L73) (`parseJWTClaims` at line 83) |
| Allowed algorithms from configured keys | [`vendor/auth/internal/conf/configuration.go:1161`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/conf/configuration.go#L1161) |
| Access-token claims (`role`, registered `sub`/`exp`) | [`vendor/auth/internal/tokens/service.go:78`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/tokens/service.go#L78) |
| Bearer header `^(?i)bearer (\S+)$` | [`vendor/auth/internal/api/api.go:38`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L38) |
| Three-part token, HMAC verify, `exp` with 30s skew | [`vendor/postgrest/src/library/PostgREST/Auth/Jwt.hs:46`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Auth/Jwt.hs#L46) |
| JWT error messages / codes | [`vendor/postgrest/src/library/PostgREST/Error.hs:638`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Error.hs#L638) |
| Demo `JWT_SECRET` / `ANON_KEY` / `SERVICE_ROLE_KEY` | [`vendor/supabase/docker/.env.example:33`](https://github.com/supabase/supabase/blob/86854671e95be31e24fa0785cded0579fe692fcb/docker/.env.example#L33) |

Pins: Auth `v2.197.0`, PostgREST `v16.4`, supabase `v1.26.08` (`vendor.toml`).

## Inputs

- Compact JWT string (Auth `Authorization: Bearer …`; REST uses the same
  token on `Authorization` / `apikey` once those units exist).
- HS256 secret from `JWT_SECRET` (raw UTF-8 bytes, not base64). Same name
  as the self-hosted demo stack. No default secret.

## Outputs

On success, `JwtClaims`:

| Field | Source | Notes |
|---|---|---|
| `role` | payload `role` if it is a JSON string | Demo anon/service_role keys set this; missing is allowed (PostgREST later defaults to the anon DB role). |
| `sub` | payload `sub` if it is a JSON string | Present on user access tokens; **absent** on the demo `ANON_KEY` / `SERVICE_ROLE_KEY`. UUID checks belong to Auth, not this verifier. |
| `exp` | payload `exp` if it is a JSON number | Unix seconds. Missing `exp` is allowed (PostgREST only rejects when the claim is present and invalid or elapsed). |
| `raw` | full payload object | For `auth.jwt()` / `request.jwt.claims` later. |

## Verification (order)

1. Reject an empty token (`Empty`).
2. Split on `.`. Require exactly three parts (`UnexpectedParts`).
3. Decode the header as JSON. `alg` must be exactly `HS256`. Any other
   value — including `none`, `None`, `RS256`, missing `alg` — is
   `BadAlgorithm`. This runs **before** HMAC so `alg=none` cannot skip
   the signature. Matches GoTrue `jwt.WithValidMethods` plus the HS256
   secret fallback when `kid` is absent or unknown
   (`auth.go:102–106`). `kid` is ignored in this unit (no JWKS).
4. HMAC-SHA256 over the original `header.payload` bytes with `JWT_SECRET`.
   Constant-time compare to the decoded signature (`BadCrypto` on mismatch
   or undecodable signature).
5. Decode the payload as a JSON object (`MalformedPayload`).
6. If `exp` is present and not a number: `ExpNotNumber`. If
   `now - 30 > exp`: `Expired`. The 30-second skew is PostgREST
   `allowedSkewSeconds` (`Jwt.hs:70`).

## Errors

`JwtError` kinds use PostgREST’s JWT messages so REST can map them to
`PGRST301` / `PGRST303` later. Auth logout maps the same kinds to GoTrue
`bad_jwt` (`invalid JWT: unable to parse or verify signature, …`).

| Kind | When | PostgREST message |
|---|---|---|
| `SecretMissing` | `JWT_SECRET` unset or empty | Server lacks JWT secret |
| `Empty` | token is empty | Empty JWT is sent in Authorization header |
| `UnexpectedParts(n)` | not 3 segments | Expected 3 parts in JWT; got n |
| `BadAlgorithm` | `alg` ≠ `HS256` | Wrong or unsupported encoding algorithm |
| `BadCrypto` | HMAC mismatch / bad signature b64 | JWT cryptographic operation failed |
| `MalformedHeader` / `MalformedPayload` | JSON not an object | Parsing claims failed |
| `ExpNotNumber` | `exp` present but not numeric | The JWT 'exp' claim must be a number |
| `Expired` | `exp` in the past beyond skew | JWT expired |

Bearer extraction (`bearer_token`) is separate: scheme `Bearer`
case-insensitive, then a single non-whitespace token
(`api.go:38`). Missing/malformed Authorization is not a JWT crypto error.

## Config

`Config::from_env` reads `JWT_SECRET` (already named). The process may
start without it so `/_megabase/health` and CI image smoke tests work.
`Config::jwt_hs256()` fails with `SecretMissing` instead of inventing a
key. Empty string is the same as unset.

## Edge cases

- Demo `ANON_KEY` / `SERVICE_ROLE_KEY` (whitespace in the JSON payload)
  verify with the demo `JWT_SECRET`.
- Token signed with a different secret: `BadCrypto`.
- `alg=none` with empty or leftover signature: `BadAlgorithm`.
- Two-part token: `UnexpectedParts(2)`.
- Expired `exp` (well before `now - 30`): `Expired`.
- Header `kid` present: still verified with `JWT_SECRET` (GoTrue HS256
  fallback). JWKS lookup is out of scope.

## Out of scope

Asymmetric keys (RS256, ES256, EdDSA), JWKS rotation, `aud` matching,
`nbf` / `iat` checks. `Hs256::sign` covers the signup access token.
`POST /token` is still out of scope.
