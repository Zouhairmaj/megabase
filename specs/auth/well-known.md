# Auth: well-known discovery (issue 55)

Status: complete for `JWT_SECRET` signing; `GOTRUE_JWT_KEYS` stays 501
Unit ids (`coverage/units.json`):
`auth:route:GET /auth/v1/.well-known/jwks.json`,
`auth:route:GET /auth/v1/.well-known/openid-configuration`,
`auth:route:GET /.well-known/oauth-authorization-server`
Level: 2

The JWKS lists the public keys that verify Auth access tokens. The
discovery document (OpenID Connect Discovery 1.0, which extends RFC 8414) is
served on two paths by one handler.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Routes | [`internal/api/api.go:203`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L203), [`:207`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L207), [`:208`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L208) |
| `WellKnownJwks` | [`internal/api/jwks.go:15`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/jwks.go#L15) |
| `OpenIDConfigurationResponse`, `WellKnownOpenID` | [`internal/api/jwks.go:39`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/jwks.go#L39), [`:66`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/jwks.go#L66) |
| `requireOAuthServerEnabled` | [`internal/api/middleware.go:404`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/middleware.go#L404) |
| `SupportedOAuthScopes` | [`internal/models/oauth_scope.go:15`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/oauth_scope.go#L15) |
| `JWT_SECRET` becomes the only key when `JWT.Keys` is empty | [`internal/conf/configuration.go:1144`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/conf/configuration.go#L1144) |
| `GOTRUE_JWT_KEYS` decoding (JSON array) | [`internal/conf/jwk.go:25`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/conf/jwk.go#L25) |
| Kong routes | `vendor/supabase/docker/volumes/api/kong.yml` services `auth-v1-open-jwks`, `auth-v1` and `well-known-oauth` |

Pins: Auth `v2.197.0` (`4eee58f`, MIT).

## Gateway

Kong strips `/auth/v1` and forwards to GoTrue. `/auth/v1/.well-known/jwks.json`
is an open route (no `key-auth`). `/auth/v1/.well-known/openid-configuration`
falls under `auth-v1`, so a request without `apikey` gets Kong's 401 first.
`/.well-known/oauth-authorization-server` is its own Kong service without
`key-auth`. Megabase applies both in `megabase-server` (`key_auth.rs`).

## Inputs

`GET` with no parameters. Configuration:

- `GOTRUE_JWT_ISSUER` (reference: `API_EXTERNAL_URL`,
  `http://localhost:8000/auth/v1`).
- `GOTRUE_OAUTH_SERVER_ENABLED` (default false).
- `GOTRUE_OAUTH_SERVER_ALLOW_DYNAMIC_REGISTRATION` (default false).
- `GOTRUE_JWT_KEYS`: unset, blank, or `[]` means `JWT_SECRET` is the only key.

## Outputs

All 200 responses: `Content-Type: application/json`,
`Cache-Control: public, max-age=600`, body from Go `json.Marshal` (fields in
struct order; `<`, `>`, `&`, U+2028 and U+2029 escaped as `\u....`).

### `GET /auth/v1/.well-known/jwks.json`

GoTrue skips keys with no public key and HMAC (`oct`) keys. The secret is the
only key on the reference stack, so the body is `{"keys":[]}`.

### Discovery document

Both discovery paths return the same object. `base` is the issuer with every
trailing `/` removed; `issuer` itself is the configured value unchanged.

| Field | Value |
|---|---|
| `issuer` | `GOTRUE_JWT_ISSUER` |
| `authorization_endpoint` | `base + "/oauth/authorize"` |
| `token_endpoint` | `base + "/oauth/token"` |
| `jwks_uri` | `base + "/.well-known/jwks.json"` |
| `userinfo_endpoint` | `base + "/oauth/userinfo"` |
| `registration_endpoint` | `base + "/oauth/clients/register"`, only when the OAuth server and dynamic registration are both on; otherwise omitted |
| `scopes_supported` | `openid`, `profile`, `email`, `phone`, `offline_access` |
| `response_types_supported` | `code` |
| `response_modes_supported` | `query` |
| `grant_types_supported` | `authorization_code`, `refresh_token` |
| `subject_types_supported` | `public` |
| `id_token_signing_alg_values_supported` | `RS256`, `HS256`, `ES256` |
| `token_endpoint_auth_methods_supported` | `client_secret_basic`, `client_secret_post`, `none` |
| `claims_supported` | `sub`, `aud`, `iss`, `exp`, `iat`, `auth_time`, `nonce`, `email`, `email_verified`, `phone_number`, `phone_number_verified`, `name`, `picture`, `preferred_username`, `updated_at` |
| `code_challenge_methods_supported` | `S256`, `plain` |

## Errors

- `GET /.well-known/oauth-authorization-server` with the OAuth server off
  (the reference stack): 404
  `{"code":404,"error_code":"feature_disabled","msg":"OAuth server is disabled"}`.
- Any method other than `GET` (including `HEAD`): chi 405, `Allow: GET`,
  empty body. The 405 comes before the OAuth middleware.

## Not ported

- `GOTRUE_JWT_KEYS` with at least one key (or a value that is not a JSON
  array): Megabase signs with `JWT_SECRET` only and cannot publish asymmetric
  keys, so the JWKS answers 501 `MEGABASE_NOT_IMPLEMENTED`.

## Edge cases

- An issuer ending in `/` keeps it in `issuer` but not in the endpoints.
- A sub-path such as `/.well-known/oauth-authorization-server/x` is not a
  GoTrue route and stays Megabase's catch-all 501.
