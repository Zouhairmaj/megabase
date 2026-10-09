---
title: supabase-js compatibility
description: Point supabase-js at your local Megabase URL. Every request returns 501 today.
section: use
order: 1
card: Point supabase-js at your local Megabase URL. Every request returns 501 today. Zero client changes is the goal, not the state.
tag: 501
---

# supabase-js compatibility

The goal is that an existing supabase-js app can point at Megabase without changing a line of code. That is the target, not the state.

> [!NOTE]
> Every call returns 501 today. There is no implemented REST, Auth, Storage, Realtime or Functions surface in this repository.

## Point the client

ANON_KEY is a JWT signed with your `JWT_SECRET`. The base URL is the Megabase listen address (default `http://localhost:8000`).

```javascript
import { createClient } from '@supabase/supabase-js'
const supabase = createClient('http://localhost:8000', ANON_KEY)
const { data, error } = await supabase.from('todos').select()
// today: error.code === 'MEGABASE_NOT_IMPLEMENTED'
```

No API behaviour is implemented yet. Until Level 1 (see the Roadmap) every call returns 501.

## What “compatible” means

Same endpoints, request syntax, response bodies, status codes, headers, error codes and messages as the pinned upstream stack. The judge — when `judge/` exists — is the only scorer. Agents cannot change what it compares against.

> [!PLANNED]
> A per-component API reference will list each route once coverage/units.json exists. Until then the catalog on Status is the list of services, not of units.
