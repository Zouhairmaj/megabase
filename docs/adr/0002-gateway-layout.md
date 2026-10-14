# ADR 0002: Gateway layout matches Kong

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

## Context

GOAL.md says the URL layout is `/rest/v1`, `/auth/v1`, `/storage/v1`,
`/realtime/v1`, `/functions/v1`, `/pg`. The self-hosted gateway is Kong,
configured in `vendor/supabase/docker/volumes/api/kong.yml`.

## Decision

`megabase-server` matches Kong by **plain string prefix** of the request
path:

| Prefix | Component |
|---|---|
| `/auth/v1/` | Auth |
| `/.well-known/oauth-authorization-server` | Auth (RFC 8414) |
| `/rest/v1/` | REST |
| `/graphql/v1` | REST |
| `/realtime/v1/` | Realtime |
| `/storage/v1/` | Storage |
| `/functions/v1/` | Functions |
| `/pg/` | Postgres Meta |

Everything else, including `/` and `/auth/v1` without the trailing slash,
goes to Studio, like Kong's catch-all `dashboard` route.

Supavisor is not on the HTTP gateway upstream (clients speak PostgreSQL to
the pooler port), so it has no prefix here.

Megabase's own liveness probe is `GET /_megabase/health`, a prefix no
Supabase service uses.

## Consequences

A request to `/auth/v1` (no slash) is Studio's, not Auth's. Tests encode
this. Pooler work is a separate listener, not a gateway route.
