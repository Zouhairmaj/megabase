# GOAL.md — Instructions for the Megabase agents

You are an agent working on **Megabase**. Read `MANIFESTO.md` once if you have not. This file is your operating manual. You start every iteration with no memory: everything you need to know is in this file and in the repository.

---

## 1. Mission

Build a **1:1, drop-in compatible clone of every Supabase-authored component, entirely in Rust**, by reading the upstream source code in `vendor/` and reproducing its externally observable behavior exactly.

"Exactly" means: same endpoints, same request syntax, same response bodies, same status codes, same headers, same error codes and messages, same database objects (schemas, tables, functions, roles) that applications and RLS policies depend on.

## 2. Sources of truth

- `vendor/` contains the upstream repositories, **pinned to fixed versions** as git submodules. This is the frozen specification. Never update the pins yourself.
  - Expected repos: Supabase Auth, PostgREST, Supabase Realtime, Supabase Storage, Supavisor, Postgres Meta, the Supabase monorepo (Studio, docker setup), Edge Runtime, and the official client libraries (`supabase-js` and its sub-packages).
- `judge/` runs the **reference stack** (official Supabase, self-hosted, from the same pinned versions) next to Megabase and compares their responses.
- When the source and the documentation disagree, **the source wins**. When the source and the reference stack disagree, **the reference stack wins**.

## 3. Hard rules

1. **Rust only.** All code you write is Rust, except configuration, SQL that must exist in the database for compatibility (for example `auth.uid()`), and the Studio front end's assets if the chosen Rust UI framework requires them.
2. **Never modify `vendor/` or `judge/`.** Changes to `judge/` are made only on a separate branch, reviewed by the reviewer agent, and merged by the orchestrator. If you believe the judge is wrong, write it up in `PROGRESS.md` under "Judge disputes" and move on.
3. **Never disable, skip or weaken a test** to make a score go up.
4. **No regressions.** A commit is kept only if total conformance does not decrease. If it does, revert and record what you tried.
5. **Fail loudly.** Anything not implemented returns HTTP 501 with a structured body: `{"code": "MEGABASE_NOT_IMPLEMENTED", "component": "...", "unit": "...", "message": "..."}`. Never return a plausible but unverified answer.
6. **Credit the source.** When you port logic from an upstream file, add a header comment naming the upstream repository, path and license. Keep `NOTICE` and `LICENSES/` up to date.
7. **No secrets, no production data.** You only ever use the local reference stack and generated or anonymized fixtures.
8. **One iteration, one commit.** Commit message format: `[component] unit: what changed (coverage X→Y, conformance A→B)`.

## 4. Architecture

- One Cargo workspace, one final binary: `megabase`.
- One crate per component, publishable on its own:
  `megabase-rest`, `megabase-auth`, `megabase-realtime`, `megabase-storage`, `megabase-functions`, `megabase-pooler`, `megabase-meta`, `megabase-studio`, plus `megabase-core` (shared types, config, JWT, errors) and `megabase-server` (assembly and gateway).
- HTTP: `axum` on `tokio`, with `tower-http` and `tracing`.
- Same URL layout as the Supabase gateway: `/rest/v1`, `/auth/v1`, `/storage/v1`, `/realtime/v1`, `/functions/v1`, `/pg` (meta). Clients must not notice any difference.
- PostgreSQL is external and standard. Megabase installs the schemas, roles and functions Supabase applications depend on.
- You may revise these choices, but only by writing an Architecture Decision Record in `docs/adr/` explaining why.

## 5. The loop (every iteration)

1. **Read the state:** `GOAL.md`, `PROGRESS.md`, `coverage/summary.json`, and the GitHub Project board (section 10).
2. **Pick one target:** take the top issue in the **Ready** column that is assigned to your role and component, move it to **In progress**, and assign yourself. If you are the orchestrator, keep **Ready** filled using the ratio of "how often real apps use it" to "how far it is from conformant". Prefer finishing started work over starting new work. Respect the current level (section 7).
3. **Write the spec first:** if `specs/<component>/<unit>.md` does not exist, read the upstream source in `vendor/` and write it: inputs, outputs, edge cases, errors, the upstream files it came from.
4. **Implement** the unit in Rust.
5. **Judge:** run `just judge` (or the equivalent documented command).
6. **Keep or revert** according to rule 4.
7. **Record:** open or update the pull request linked to your issue, comment on the issue with what you did and learned, regenerate `coverage/`, and update `PROGRESS.md` only for decisions and state that outlive a single issue.

If you are stuck on a unit after a few attempts, mark it "Blocked" in `PROGRESS.md` with the reason and pick another target. Never loop on the same failure.

## 6. Phase 0 — Bootstrap (before any feature work)

Do these in order. When all are done, stop and write "PHASE 0 COMPLETE — awaiting human review" in `PROGRESS.md`. A human reviews Phase 0 once, and logs it in `HUMAN_LOG.md`.

1. Create the Cargo workspace and empty crates from section 4. The binary starts and answers `501` everywhere.
2. Build `judge/`: start the reference stack and Megabase side by side, send identical requests to both, compare status, headers that matter, and bodies (with a documented normalization for timestamps, IDs and other non-deterministic values).
3. **Extract the full denominator:** generate `coverage/units.json` from the pinned upstream source: every route, query operator, auth flow, message type, storage endpoint, meta endpoint and Studio page, each with its component and upstream location.
4. Build the treemap generator: `coverage/` produces one SVG per metric (coverage, conformance), one square per unit.
5. Set up CI: build, judge, coverage, and a check that rejects any commit touching `vendor/` or `judge/` outside the review branch.
6. Create `NOTICE`, `LICENSES/`, `PROGRESS.md` and an empty `HUMAN_LOG.md`.
7. Set up GitHub project management (section 10): milestones for the five levels, labels, the Project board and its fields, and the first epics for Level 1 generated from `coverage/units.json`.

## 7. Levels (work order)

1. **Level 1:** `/rest/v1` (PostgREST behavior) + `/auth/v1` email/password, JWT, `auth.users`, `auth.uid()`, `auth.jwt()`.
2. **Level 2:** OAuth providers, magic links, OTP, `/storage/v1` with `storage.objects` and its RLS.
3. **Level 3:** `/realtime/v1`: database changes via logical replication, broadcast, presence.
4. **Level 4:** `/functions/v1`, pooler, Postgres Meta, then **the Studio test** (see section 9).
5. **Level 5 (stretch):** first investigate whether Studio's front end can be statically exported; if so, serve it from the `megabase` binary and reimplement its server-side API routes in Rust. Only then consider a full Rust rewrite of Studio. Never embed a Node runtime.

Do not start a level until the previous one reaches the conformance threshold set in `PROGRESS.md`.

## 8. Roles

- **Orchestrator:** reads coverage, assigns targets, merges branches, enforces levels.
- **Builders:** one per component, each on its own branch, following the loop above.
- **Reviewer:** has not seen the code being reviewed. Checks rules 2–6, reads diffs for test gaming, and approves or rejects merges. Any change to `judge/` requires reviewer approval.

## Design (design-first, Kite)

Every graphic and UI element — brand, banners, OG/social images, website
pages, badges, and treemap style — is designed first in the Kite project
**Megabase** (public mockups:
[kite.new/p/megabase-identity](https://kite.new/p/megabase-identity)).

- Every page and screen has a **desktop** version and a **mobile** version.
- Designs are reviewed by an LLM committee via OpenRouter, corrected, then
  implemented pixel-faithfully.
- Implementation must not diverge from Kite. To change a visual, change
  Kite first, then the code. Kite governs visual design only, never
  compatibility behavior.
- Agents design on their own: design decisions are made by agents with the
  LLM committee, with **no human design input**. That is an explicit part
  of the experiment.

## Documentation

Documentation (`docs/` markdown, rendered on
[megabase.sh/docs](https://megabase.sh/docs)) is a first-class
deliverable. It must stay up to date with the code and be well written.

- Any PR that changes behavior, commands, config, or status must update
  the relevant docs in the same PR.
- Docs must be truthful (never claim unimplemented features) and clear.
- Reviewers block PRs with stale or missing docs.

## 9. The Studio test

The official Supabase Studio, pinned and **unmodified**, is part of the judge.

- `judge/studio/` runs the official Studio container twice: once against the reference stack, once against Megabase.
- Scripted browser sessions (Playwright) perform the same scenarios on both: browse schemas, create and edit tables, run SQL, manage RLS policies, create and delete users, upload and delete files, inspect logs where available.
- Compared outputs: every network call Studio makes (method, path, status, body after normalization), visible errors in the UI, and the final database state.
- Every Studio call that Megabase answers with `MEGABASE_NOT_IMPLEMENTED` is added automatically to the priority list in `PROGRESS.md`.
- The Studio test passes when all scenarios produce identical results on both stacks.

Like the rest of `judge/`, the Studio scenarios are changed only through the reviewer.

## 10. Project management on GitHub

The orchestrator runs Megabase like a project manager, in public, with GitHub's native tools. All agents use the `gh` CLI. GitHub is the source of truth for **what is being worked on**; `PROGRESS.md` is the source of truth for **decisions and overall state**.

**Structure**
- **Milestones** = levels (`Level 1` … `Level 5`), each with its conformance threshold in the description.
- **Epics** = one issue per feature group (for example "PostgREST: filtering operators", "Auth: email/password flows"), with sub-issues for the units. Do not create one issue per unit in `coverage/units.json`: group them. Keep the open issue count manageable (target: under 150 open at any time).
- **Labels:** `component:rest|auth|realtime|storage|functions|pooler|meta|studio|core|judge`, `level:1…5`, `type:feature|bug|spec|infra|investigation`, `blocked`, `judge-dispute`, `needs-human`.
- **Project board** (GitHub Projects) with columns: Backlog → Ready → In progress → In review → Done, plus Blocked. Custom fields: component, level, coverage delta, conformance delta, estimated effort.

**Workflow**
1. The orchestrator turns `coverage/units.json` and failing judge results into epics and sub-issues, prioritizes them, and fills **Ready**.
2. A builder takes one issue, works on branch `issue-<number>-<slug>`, and opens a pull request containing `Closes #<number>` and the coverage and conformance deltas.
3. CI runs build, judge and coverage on the PR and posts the results as a comment.
4. The reviewer agent reviews the PR (rules in section 3, test gaming, upstream attribution) and approves or requests changes.
5. The orchestrator merges approved, green PRs and moves the issue to **Done**.
6. Blocked work is labeled `blocked` with the reason. Questions only a human can answer are labeled `needs-human`; they are the only legitimate way for agents to ask for human input, and every human answer is recorded in `HUMAN_LOG.md`.

**Rituals**
- **Weekly report:** every week the orchestrator publishes a GitHub Discussion (category "Weekly reports"): what was merged, coverage and conformance trends, blocked items, cost, priorities for next week.
- **Milestone retrospective:** when a level's threshold is reached, the orchestrator writes a retrospective (what worked, what failed, what to change in the process) and proposes changes to this file as a pull request for human approval.

**Enforcement (set up by humans in Phase 0)**
- Branch protection on `main`: pull requests only, CI green, one approving review required.
- `CODEOWNERS` assigns `vendor/`, `judge/`, `GOAL.md` and `MANIFESTO.md` to the human maintainers, so agents cannot change them without a logged human approval.
- Agents act through a dedicated bot account or GitHub App with the minimum permissions needed.

## 11. Public record

- `PROGRESS.md`: current state, decisions, blocked items, judge disputes.
- `devlog/YYYY-MM-DD.md`: one short entry per day, written for humans following the experiment.
- `coverage/`: metrics and treemaps, regenerated on every commit.
- `HUMAN_LOG.md`: maintained by humans only.
