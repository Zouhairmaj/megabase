# Auth: OAuth 2.1 server (issue 48)

Status: disabled gate complete on all six routes; with the server on,
dynamic client registration and `userinfo` are served and the
authorization-code flow (`authorize`, `authorizations/{id}`, `consent`,
`token`) answers 501.
Unit ids (`coverage/units.json`):
`auth:route:POST /auth/v1/oauth/clients/register`,
`auth:route:GET /auth/v1/oauth/userinfo`,
`auth:route:GET /auth/v1/oauth/authorize`,
`auth:route:GET /auth/v1/oauth/authorizations/{authorization_id}`,
`auth:route:POST /auth/v1/oauth/authorizations/{authorization_id}/consent`,
`auth:route:POST /auth/v1/oauth/token`
Level: 2

Supabase Auth acting as an OAuth 2.1 / OIDC authorization server for
third-party clients. Distinct from `GET /auth/v1/authorize`, where Auth is the
OAuth *client* of an external provider.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Routes and middleware order | [`internal/api/api.go:445`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L445)–`:462` |
| `requireOAuthServerEnabled` | [`internal/api/middleware.go:404`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/middleware.go#L404) |
| `performRateLimiting` | [`internal/api/middleware.go:123`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/middleware.go#L123) |
| `requireOAuthClientAuth` | [`internal/api/middleware.go:143`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/middleware.go#L143) |
| `OAuthClientRegister` limiter (`newLimiterPer5mOver1h`) | [`internal/api/apilimiter/apilimiter.go:239`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/apilimiter/apilimiter.go#L239), [`:358`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/apilimiter/apilimiter.go#L358) |
| `OAuthServerClientDynamicRegister` | [`internal/api/oauthserver/handlers.go:133`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/oauthserver/handlers.go#L133) |
| `registerOAuthServerClient`, `validate` | [`internal/api/oauthserver/service.go:251`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/oauthserver/service.go#L251), [`:121`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/oauthserver/service.go#L121) |
| `OAuthToken` | [`internal/api/oauthserver/handlers.go:267`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/oauthserver/handlers.go#L267) |
| `OAuthUserInfo` | [`internal/api/oauthserver/handlers.go:658`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/oauthserver/handlers.go#L658) |
| `OAuthServerAuthorize`, `OAuthServerGetAuthorization`, `OAuthServerConsent` | [`internal/api/oauthserver/authorize.go:84`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/oauthserver/authorize.go#L84), [`:177`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/oauthserver/authorize.go#L177), [`:301`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/oauthserver/authorize.go#L301) |
| `ParseScopeString`, `HasScope` | [`internal/models/oauth_scope.go:29`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/oauth_scope.go#L29), [`:48`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/oauth_scope.go#L48) |
| `sessions.scopes` column | `migrations/20251111201300_add_scopes_to_sessions.up.sql` |

Pins: Auth `v2.197.0` (`4eee58f`, MIT).

## Gateway

Kong forwards `/auth/v1/oauth/*` through the `auth-v1` service, so a request
without `apikey` gets Kong's 401 first (`megabase-server` `key_auth.rs`).

## Configuration

- `GOTRUE_OAUTH_SERVER_ENABLED` (default false; unset on the reference stack).
- `GOTRUE_OAUTH_SERVER_ALLOW_DYNAMIC_REGISTRATION` (default false).
- `GOTRUE_RATE_LIMIT_OAUTH_DYNAMIC_CLIENT_REGISTER` (default 10 per five
  minutes, burst 30), keyed like every GoTrue limiter on the first
  comma-separated value of `GOTRUE_RATE_LIMIT_HEADER`.

## Server disabled (reference stack)

The six routes live in one chi sub-router whose first middleware is
`requireOAuthServerEnabled`. Sub-router middleware runs before chi matches the
method, so **every method** on each path answers:

```
404 {"code":404,"error_code":"feature_disabled","msg":"OAuth server is disabled"}
```

with `X-Sb-Error-Code: feature_disabled`, before any body, token or rate
limit is read.

## Server enabled

A method other than the route's one is chi's 405 with `Allow`.

### `POST /oauth/clients/register`

1. Rate limit (`OAuthClientRegister`): 429 `over_request_rate_limit`,
   `Request rate limit reached`.
2. Dynamic registration off: 403
   `oauth_dynamic_client_registration_disabled`,
   `Dynamic client registration is not enabled`.
3. Body is decoded with `json.NewDecoder(...).Decode`, which reads the first
   JSON value only. Failure: 400 `bad_json`, `Invalid JSON body`.
4. `registerOAuthServerClient` with `registration_type = "dynamic"`; the
   validation and insert are shared with `POST /admin/oauth/clients`
   (`specs/auth/admin.md`). A validation error is 400 `validation_failed`
   whose message is the Go `HTTPError.Error()` text (`400: <message>`).
5. 201 with the client object; `client_secret` only for confidential
   clients.

### `GET /oauth/userinfo`

`requireAuthentication` (same errors as `GET /user`), then:

- `sub` is always present.
- No `session_id` claim: only `sub`.
- Otherwise the session's `scopes` column (NULL is empty) selects claims:
  - `email`: `email` when non-empty; `email_verified: true` when
    `email_confirmed_at` is set.
  - `profile`: `name` = `user_metadata.name` (non-empty string) else the
    email; `picture` = `user_metadata.picture` else `avatar_url`;
    `preferred_username` = `user_metadata.preferred_username` else
    `username`; `updated_at` as Unix seconds when positive; `user_metadata`
    when not null.
  - `phone`: `phone` when non-empty; `phone_verified: true` when
    `phone_confirmed_at` is set.
- 200, Go `json.Marshal` of a map: sorted keys, HTML-escaped.

Only `POST /oauth/token` writes `sessions.scopes`, so sessions from the other
grants always yield `{"sub": ...}`.

### Not ported (501)

`GET /oauth/authorize`, `GET /oauth/authorizations/{id}`,
`POST /oauth/authorizations/{id}/consent` and `POST /oauth/token` need the
authorization-code flow (`oauth_authorizations`, consents, PKCE, client
authentication, OAuth session and refresh-token issuance). With the server
enabled they return `MegabaseNotImplemented` for their unit after the gate
and method check. They carry no `megabase:unit` marker until that flow is
ported (same rule as `GET /authorize` for enabled external providers).

## Edge cases

- Unknown paths under `/auth/v1/oauth/` stay on the gateway's catch-all.
- A wrong method while disabled is 404, not 405.
- Registration on the in-memory backend fails the insert and answers 400
  `failed to create OAuth client: ...`, as the admin route does.
