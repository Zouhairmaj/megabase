# Auth: endpoints

Status: stub
Unit ids: `auth:route:` + `GET /auth/v1/health`, `GET /auth/v1/settings`,
`GET /auth/v1/reauthenticate`, `POST /auth/v1/signup`, `POST /auth/v1/invite`,
`POST /auth/v1/logout`, `POST /auth/v1/recover`, `POST /auth/v1/resend`
Level: 1

GoTrue email/password endpoints in this Level 1 group. They are not all
public. Template: [`specs/_template.md`](../_template.md).

- Public (no session or admin credentials): `GET /health`, `GET /settings`,
  `POST /signup`, `POST /recover`, `POST /resend`.
- Protected: `POST /invite` uses `requireAdminCredentials`; `POST /logout`
  and `GET /reauthenticate` use `requireAuthentication`. Do not label these
  public.

## Upstream

- `vendor/auth/internal/api/api.go` (route table): `/health` at :202,
  `/settings` at :222, `/invite` at :228, `/signup` at :229, `/recover` at
  :255–256, `/resend` at :258–259, `/logout` at :275, `/reauthenticate` at
  :277–279. Handler files to read when filling this in: `signup.go`,
  `invite.go`, `logout.go`, `recover.go`, `resend.go`, `settings.go`,
  `reauthenticate.go`.
- Pin: `auth v2.197.0` (`vendor.toml`).

## To write

Inputs, outputs, errors, edge cases, out of scope, judge cases.
