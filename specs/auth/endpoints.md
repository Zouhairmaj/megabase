# Auth: endpoints

Status: stub
Unit ids: `auth:route:` + `GET /auth/v1/health`, `GET /auth/v1/settings`,
`GET /auth/v1/reauthenticate`, `POST /auth/v1/signup`, `POST /auth/v1/invite`,
`POST /auth/v1/logout`, `POST /auth/v1/recover`, `POST /auth/v1/resend`
Level: 1

Public GoTrue endpoints for email/password sign-up and session end.
Template: [`specs/_template.md`](../_template.md).

## Upstream

- `vendor/auth/internal/api/api.go` (route table). Handler files to read
  when filling this in: `signup.go`, `invite.go`, `logout.go`, `recover.go`,
  `resend.go`, `settings.go`, `reauthenticate.go`.
- Pin: `auth v2.197.0` (`vendor.toml`).

## To write

Inputs, outputs, errors, edge cases, out of scope, judge cases.
