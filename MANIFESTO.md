# Megabase

**Supabase, rewritten in Rust. By agents. In public.**

---

## What this is

Megabase is an experiment.

One mission, given to a team of AI coding agents: produce a **1:1, drop-in compatible clone of every component Supabase has written**, entirely in **Rust**, by reading the Supabase source code and porting its behavior.

The finished product is a single lightweight binary that sits next to PostgreSQL and serves the exact same APIs as a full Supabase stack, so that any existing app built with `supabase-js` (or any other Supabase client) runs against it **without changing a line of code**.

The experiment is the real product. The binary is its proof.

## Why

1. **To measure what autonomous agents can actually build.** Not a toy, not a demo: a large, multi-language, production-grade system with a precise, verifiable target.
2. **Because the target is perfect for it.** Supabase is open source, documented, and runnable locally. Every behavior the agents need to reproduce can be read in the source and checked against the real thing. There is no guessing about what "correct" means.
3. **Because the result is useful.** Self-hosting Supabase today means running a dozen containers written in six languages. Megabase aims for one binary, a fraction of the memory, and the same API.

## Rules of the experiment

1. **Agents write everything.** Code, architecture, documentation, CI, comparison harness, benchmarks, README, devlog. Everything in this repository after this manifesto and `GOAL.md` is agent-written.
2. **Humans write only the mission and the guardrails.** That means this manifesto, `GOAL.md`, and the initial setup of the environment. Nothing else.
3. **Every human intervention is logged.** Any action a human takes on the repository or the running agents (a fix, a nudge, a reverted commit, a changed prompt) is recorded publicly in `HUMAN_LOG.md`, with the reason. The count is part of the result.
4. **The judge is external.** Agents do not grade themselves. Correctness is decided by comparing Megabase's responses with those of the **real Supabase stack**, run side by side from pinned upstream versions, plus upstream test suites where they can target an HTTP server. Agents may build the comparison harness, but they may never change what it compares against, and every change to the harness goes through an independent reviewer agent.
5. **Everything is in Rust.** Every component in scope is reimplemented in Rust. No wrappers around the original services, no embedded runtimes of other languages, except where the upstream component itself embeds one (the Deno-based Edge Runtime).
6. **Failures are loud.** Anything not yet implemented returns an explicit, structured "not implemented" error. Megabase never returns a silently wrong answer.
7. **Everything is public.** The code, the agent prompts, the loop, the logs, the token spend and the cost, in real time.

## Scope

**In scope: every service Supabase has written.**

| Component | Upstream language | Megabase |
|---|---|---|
| REST API (PostgREST) | Haskell | Rust |
| Auth (GoTrue) | Go | Rust |
| Realtime | Elixir | Rust |
| Storage API | TypeScript | Rust |
| Edge Functions runtime | Rust + Deno | Rust + `deno_core` |
| Connection pooler (Supavisor) | Elixir | Rust |
| Postgres Meta | TypeScript | Rust |
| API gateway (replaces Kong) | Lua / Nginx | Rust |
| Studio (dashboard) | TypeScript / Next.js | Unmodified official Studio first (as a judge), Rust rewrite as the final stretch goal |

**Out of scope:**
- **PostgreSQL and its extensions.** They are the foundation, not a Supabase component. Megabase runs on standard PostgreSQL, so Row Level Security, migrations and extensions work exactly as they do on Supabase.
- **Supabase's hosted-platform features** that do not exist in the self-hosted stack (billing, organization management, the cloud management API).

## Definition of success

Progress is reached in public levels:

1. **Level 1:** database REST API + email/password auth. Most apps already run.
2. **Level 2:** OAuth providers, magic links, Storage.
3. **Level 3:** Realtime (database changes, broadcast, presence).
4. **Level 4:** Edge Functions, pooler, Postgres Meta, and **the Studio test**: the official, unmodified Supabase Studio manages a Megabase instance without noticing any difference.
5. **Level 5 (stretch goal):** Studio itself, served from the Megabase binary with its server-side routes reimplemented in Rust, then rewritten entirely in Rust.

**The experiment succeeds** when real, unmodified open-source Supabase apps run on Megabase **and the official Supabase Studio cannot tell the difference**. Level 5 is the bonus round.

### The Studio test

Supabase Studio is the most demanding client of the whole stack: it calls dozens of endpoints across Postgres Meta, Auth admin, Storage, the SQL editor, the table editor and RLS policy management. Running it unmodified against Megabase is the closest thing to a Turing test for compatibility. Scripted browser sessions drive Studio against the reference stack and against Megabase, and the results are compared.

## How progress is measured

- **Coverage:** the share of all upstream units (routes, query operators, auth flows, message types, storage endpoints, Studio pages) that Megabase implements. The denominator is extracted from the pinned upstream source on day one, so 100% means 100%.
- **Conformance:** the share of differential tests where Megabase's response is identical to Supabase's.
- A unit counts as done only when it is both implemented **and** conformant.
- **Treemaps:** one square per unit, grey to green, regenerated on every commit.
- **Verified apps:** a public list of real Supabase apps confirmed to run unmodified.
- **Cost:** tokens and money spent, published continuously.

## Licensing and credits

Megabase is a port of open-source software. It is released under the **Apache License 2.0**. Each upstream component's license and copyright notices are preserved and credited in `NOTICE` and `LICENSES/`.

Megabase is an independent experiment. It is **not affiliated with, endorsed by, or sponsored by Supabase**. "Supabase" is a trademark of its owner and is used here only to describe compatibility.

## Status

Day 0.
