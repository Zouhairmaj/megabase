---
title: Configuration
description: DATABASE_URL, JWT_SECRET, MEGABASE_PORT — what each variable does and its default.
section: get-started
order: 3
card: DATABASE_URL, JWT_SECRET, MEGABASE_PORT: what each variable does and its default.
---

# Configuration

The megabase binary is not in this repository yet. These are the environment variables the experiment has already named. Do not invent others.

| Variable | Description |
| --- | --- |
| DATABASE_URL | PostgreSQL connection string. PostgreSQL stays external; Megabase does not bundle it. |
| JWT_SECRET | Secret used to sign and verify JWTs. Auth is not implemented; the value is still required so a missing secret fails loudly instead of minting a default. |
| MEGABASE_PORT | HTTP port. Default 8000. |

There is no `.env.example` in the tree. When the binary exists it will read the process environment at startup.

## Listen address

Same URL layout as the Supabase gateway, from GOAL.md:

- `/rest/v1`
- `/auth/v1`
- `/storage/v1`
- `/realtime/v1`
- `/functions/v1`
- `/pg` (Postgres Meta)

Until a unit passes the judge, each of those answers HTTP 501 with `code: MEGABASE_NOT_IMPLEMENTED`.

> [!PLANNED]
> Image tags, compose overlays and a documented ANON_KEY generator are not in this repository.
