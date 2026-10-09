# Auth: verify

Status: stub
Unit ids: `auth:route:GET /auth/v1/verify`, `auth:route:POST /auth/v1/verify`,
`auth:verify-type:` + `signup`, `recovery`, `invite`, `email_change`
Level: 1

Confirmation of one-time tokens sent by email. Template:
[`specs/_template.md`](../_template.md).

## Upstream

- `vendor/auth/internal/api/api.go` (routes), `vendor/auth/internal/api/verify.go`
  (verify types).
- Pin: `auth v2.197.0` (`vendor.toml`).

## To write

Inputs, outputs, errors, edge cases, out of scope, judge cases.
