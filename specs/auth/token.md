# Auth: token

Status: stub
Unit ids: `auth:route:POST /auth/v1/token`, `auth:grant-type:password`,
`auth:grant-type:refresh_token`
Level: 1

Token endpoint for the password and refresh-token grants. JWT verification
lives in [`specs/core/jwt.md`](../core/jwt.md). Template:
[`specs/_template.md`](../_template.md).

## Upstream

- `vendor/auth/internal/api/api.go` (route), `vendor/auth/internal/api/token.go`
  (grants).
- Pin: `auth v2.197.0` (`vendor.toml`).

## To write

Inputs, outputs, errors, edge cases, out of scope, judge cases.
