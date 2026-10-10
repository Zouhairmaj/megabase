# Core: HS256 JWT validation and config

Shared `megabase-core` types so Auth and REST enforce the same JWT and role
claims as GoTrue and PostgREST. This is library code, not an HTTP unit.
Auth signs with `Hs256::sign` and verifies bearer tokens with
`Hs256::verify_gotrue` (GoTrue's golang-jwt order and messages). REST
verifies with `Hs256::verify` (PostgREST's order and messages).

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Parse Bearer, verify signature, HS256 secret fallback | [`vendor/auth/internal/api/auth.go:73`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/auth.go#L73) (`parseJWTClaims` at line 83) |
| Allowed algorithms from configured keys | [`vendor/auth/internal/conf/configuration.go:1161`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/conf/configuration.go#L1161) |
| Access-token claims (`role`, registered `sub`/`exp`) | [`vendor/auth/internal/tokens/service.go:78`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/tokens/service.go#L78) |
| Bearer header `^(?i)bearer (\S+)$` | [`vendor/auth/internal/api/api.go:38`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L38) |
| Three-part token, HMAC verify, `exp` with 30s skew | [`vendor/postgrest/src/library/PostgREST/Auth/Jwt.hs:46`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Auth/Jwt.hs#L46) |
| JWT error messages / codes | [`vendor/postgrest/src/library/PostgREST/Error.hs:638`](https://github.com/PostgREST/postgrest/blob/0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7/src/library/PostgREST/Error.hs#L638) |
| golang-jwt parse order, errors, claim validation (v5.3.1 from `vendor/auth/go.mod`) | `parser.go` `ParseWithClaims`, `errors.go`, `validator.go`, `hmac.go`, `types.go` |
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
`PGRST301` / `PGRST303` later, except `SecretTooShort`, which is Megabase's
32-byte key floor. Auth uses the GoTrue variant below instead.

| Kind | When | PostgREST message |
|---|---|---|
| `SecretMissing` | `JWT_SECRET` unset or empty when building a verifier | Server lacks JWT secret |
| `SecretTooShort` | HMAC key of 1–31 bytes (`Hs256::new`) | Megabase only: HMAC-SHA-256 keys shorter than 32 bytes are disabled (got n). Not a PostgREST message. |
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

## GoTrue variant (`verify_gotrue`)

Auth (`GET`/`PUT /user`, logout, admin routes) answers 403 `bad_jwt` with
`invalid JWT: unable to parse or verify signature, <parser error>`, where
the parser error is golang-jwt's `ParseWithClaims` with
`WithValidMethods(["HS256"])` and GoTrue's `AccessTokenClaims`. Checks run
in this order; the first failure wins:

| Step | Failure message (after the prefix) |
|---|---|
| 1. Exactly 3 `.` segments | `token is malformed: token contains an invalid number of segments` |
| 2. Header base64 (RawURL; padding is not allowed, CR/LF skipped) | `token is malformed: could not base64 decode header: illegal base64 data at input byte N` |
| 3. Header JSON into `map[string]interface{}` | `token is malformed: could not JSON decode header: <encoding/json error>` |
| 4. Claims base64 | same, with `claim` |
| 5. Claims JSON into `AccessTokenClaims` (string, map, bool and NumericDate field types) | `token is malformed: could not JSON decode claim: <encoding/json error>` |
| 6. `alg` present and a string | `token is unverifiable: signing method (alg) is unspecified` |
| 7. `alg` registered (HS*, RS*, PS*, ES*, EdDSA, `none`) | `token is unverifiable: signing method (alg) is unavailable` |
| 8. Signature base64 | `token is malformed: could not base64 decode signature: …` |
| 9. `alg` is `HS256` | `token signature is invalid: signing method X is invalid` |
| 10. HMAC-SHA256 | `token signature is invalid: signature is invalid` |
| 11. `now >= exp`, `now < nbf` (no skew), joined with `, ` | `token has invalid claims: token is expired` / `token is not valid yet` |

`alg` is case-sensitive, so `None` and `NONE` fail at step 7 and `none`
at step 9. Claims are decoded before HMAC only to reproduce that order;
no claim is returned until step 10 passes. `exp` is optional, as in
golang-jwt without `WithExpirationRequired`.

## Config

`Config::from_env` reads `JWT_SECRET` (already named). Length is UTF-8
bytes, not characters, and not an entropy check.

Unset: the process starts so `/_megabase/health` and CI image smoke tests
work. `Config::jwt_hs256()` then fails with `SecretMissing`. Megabase does
not invent a key.

A present value shorter than 32 bytes, including the empty string, is a
configuration error. `from_env` returns before the process listens:

`configuration error: JWT_SECRET is N bytes; HMAC-SHA-256 keys shorter than 32 bytes are disabled`

That floor is stricter than GoTrue and PostgREST, which accept any
non-empty symmetric secret. It matches the "at least 32 characters" note
for `AUTH_JWT_SECRET` and `PGRST_JWT_SECRET` in
`vendor/supabase/docker/CONFIG.md`. The demo secret in `.env.example` is
ASCII and longer than 32 bytes.

`Hs256::new` rejects an empty key as `SecretMissing` and a key of 1–31
bytes as `SecretTooShort`. A key of 32 bytes or more is accepted.

## Edge cases

- `JWT_SECRET` unset: process starts; verification returns `SecretMissing`.
- `JWT_SECRET` of 0 or 31 bytes: `Config::from_env` fails before listen.
- `Hs256::new` with 31 bytes: `SecretTooShort`. With 32 bytes: accepted.
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
`iat` checks, and `nbf` for REST. `Hs256::sign` covers the signup access token.
`POST /token` is still out of scope.
