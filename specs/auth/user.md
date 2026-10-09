# Auth: user

Status: stub
Unit ids: `auth:route:GET /auth/v1/user`, `auth:route:PUT /auth/v1/user`
Level: 1

Read and update the authenticated user. Template:
[`specs/_template.md`](../_template.md).

## Upstream

- `vendor/auth/internal/api/api.go` (route table); handler in
  `internal/api/user.go`.
- Pin: `auth v2.197.0` (`vendor.toml`).

## To write

Inputs, outputs, errors, edge cases, out of scope, judge cases.
