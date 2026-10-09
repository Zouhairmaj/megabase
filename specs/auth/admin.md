# Auth: admin

Status: stub
Unit ids: `auth:route:` + `GET /auth/v1/admin/audit`, `GET|POST /auth/v1/admin/users`,
`GET|PUT|DELETE /auth/v1/admin/users/{user_id}`, `POST /auth/v1/admin/generate_link`
Level: 1

Service-role admin API of GoTrue. Template: [`specs/_template.md`](../_template.md).

## Upstream

- `vendor/auth/internal/api/api.go` (route table). Handler files to read
  when filling this in: `internal/api/admin.go`, `internal/api/audit.go`.
- Pin: `auth v2.197.0` (`vendor.toml`).

## To write

Inputs, outputs, errors, edge cases, out of scope, judge cases.
