---
title: supabase-js compatibility
description: Point supabase-js at your local Megabase URL. A `.select()` read is served when the database is configured. Auth health, settings, signup, logout, and the password and refresh-token grants are served.
section: use
order: 1
card: Point supabase-js at your local Megabase URL. `.select()` reads are served. Embeds, writes, and most other routes still return 501.
tag: 501
---

# supabase-js compatibility

The goal is that an existing supabase-js app can point at Megabase without changing a line of code. That is the target, not the state.

> [!NOTE]
> Storage, Realtime, Functions, and most Auth routes still return 501. A supabase-js read sends `select=*`. `.from().select()` and a chained `.eq()` are served reads when `DATABASE_URL` is set ([filtering](../specs/rest/filtering.md), [query parameters](../specs/rest/query-params.md)). Embeds, `Prefer`, and writes stay 501. `GET /auth/v1/health`, `GET /auth/v1/settings`, autoconfirm email signup, logout, and the password and refresh-token grants on `POST /auth/v1/token` are served when `DATABASE_URL` and `JWT_SECRET` are set (health and settings do not need them). See [Configuration](configuration.md).

## Point the client

`ANON_KEY` is an HS256 JWT signed with `JWT_SECRET` (the demo pair is in
`vendor/supabase/docker/.env.example`). Megabase verifies that signature
in `megabase-core`. Logout uses the same verifier. A `.select()` call is a
served read. A chained `.eq()` is a horizontal filter on that read.
The base URL is the Megabase listen address (default
`http://localhost:8000`).

```javascript
import { createClient } from '@supabase/supabase-js'
const supabase = createClient('http://localhost:8000', ANON_KEY)
const { data, error } = await supabase.from('todos').select()
// rows of public.todos when DATABASE_URL is set
```

`from('todos').select()` is a served read of `public.todos` when `DATABASE_URL` is set. Embeds and writes stay 501. Level 1 (see the Roadmap) is the REST and Auth email/password target; the Auth routes in [Configuration](configuration.md) are served.

## What “compatible” means

Same endpoints, request syntax, response bodies, status codes, headers, error codes and messages as the pinned upstream stack. The judge — when `judge/` exists — is the only scorer. Agents cannot change what it compares against.

> [!PLANNED]
> A per-component API reference will list each route once coverage/units.json exists. Until then the catalog on Status is the list of services, not of units.
