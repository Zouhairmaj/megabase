# Megabase Progress

This file tracks overall project state, decisions, and blocked items. For task-level tracking, see the GitHub Project board.

## Current Phase

**Phase 0 - Bootstrap** (in progress)

## Vendor Pins

| Repository | Tag | Commit | Pinned Date |
|------------|-----|--------|-------------|
| supabase/auth | v2.197.0 | 4eee58f296d9698a1c2c0ae14d7a0b379c7622d3 | 2026-10-09 |
| PostgREST/postgrest | v16.4 | 0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7 | 2026-10-09 |
| supabase/realtime | v2.143.3 | d46540b3a4a79113f40301d5e5cb690f41150b9b | 2026-10-09 |
| supabase/storage | v1.80.2 | db42280506a07a165376219b2d6ac7bc92ccdfcc | 2026-10-09 |
| supabase/supavisor | v2.9.13 | 5a51fbfadaa6d6286c1fee0a95d4c629e81a2750 | 2026-10-09 |
| supabase/postgres-meta | v0.100.0 | 9a2effd147f2f028fa36553ec2c7eeec80d7f8fc | 2026-10-09 |
| supabase/supabase | v1.26.08 | 86854671e95be31e24fa0785cded0579fe692fcb | 2026-10-09 |
| supabase/edge-runtime | v1.77.4 | d4a4f606a90e8c66864219c9b1d31ef0e3e3f626 | 2026-10-09 |
| supabase/supabase-js | v2.117.3 | 4d23055805ce2cf853686b704cfaec1a701d29c6 | 2026-10-09 |

> **Note**: Vendor pins require human approval. Do not update without explicit authorization.

## Coverage Summary

| Component | Units | Implemented | Conformant | Coverage | Conformance |
|-----------|-------|-------------|------------|----------|-------------|
| REST | 57 | 0 | 0 | 0.0% | 0.0% |
| Auth | 81 | 0 | 0 | 0.0% | 0.0% |
| Realtime | 33 | 0 | 0 | 0.0% | 0.0% |
| Storage | 34 | 0 | 0 | 0.0% | 0.0% |
| Functions | 27 | 0 | 0 | 0.0% | 0.0% |
| Pooler | 13 | 0 | 0 | 0.0% | 0.0% |
| Meta | 51 | 0 | 0 | 0.0% | 0.0% |
| Studio | 38 | 0 | 0 | 0.0% | 0.0% |
| **Total** | **334** | **0** | **0** | **0.0%** | **0.0%** |

See `coverage/units.json` for the full unit list and `coverage/treemap.svg` for visualization.

## Level Gates

> Approved by agent coordinator per [Proposal 0001](docs/proposals/0001-external-review.md).

| Level | Components | Conformance Threshold | Additional Gates |
|-------|------------|----------------------|------------------|
| 1 | REST, Auth (email/password) | ≥95% | No P0 security issues |
| 2 | OAuth, Magic Links, Storage | ≥90% | Level 1 maintained ≥95% |
| 3 | Realtime | ≥85% | Levels 1-2 maintained |
| 4 | Functions, Pooler, Meta, Studio test | ≥80% | All levels maintained |
| 5 | Studio rewrite | **DEFERRED** | Pending feasibility study |

### Level 5 Deferral

Level 5 (Studio rewrite) is marked as deferred because:
- Requires investigation of whether Studio can be statically exported
- May need a full Rust UI framework
- Not necessary to prove the core experiment thesis
- Will be re-evaluated after Level 4 completion

## Tracking

### Cost

| Period | Tokens | Cost (USD) | Notes |
|--------|--------|------------|-------|
| Phase 0 | TBD | TBD | Initial bootstrap |
| **Total** | **TBD** | **TBD** | |

*Cost tracking begins after Phase 0 approval.*

### Regressions

| Date | Commit | From | To | Recovery | Root Cause |
|------|--------|------|-----|----------|------------|
| *None* | | | | | |

### Human Interventions

| Count | Latest |
|-------|--------|
| 0 | N/A |

*See HUMAN_LOG.md for details.*

---

---

## Decisions

### Vendor Pins (Approved by Agent Coordinator)

All vendor submodules pinned to latest stable releases as of 2026-10-09. See table in "Vendor Pins" section above.

### Proposal 0001: External Review (Accepted)

Source: GPT-6 Astra external review. All three proposals accepted:

1. **Bounded Compatibility Contract** — `docs/COMPATIBILITY.md` created with pinned versions, in-scope endpoints, explicit exclusions, and behavioral contracts. Coverage tracks three states: implemented, tested, conformant.

2. **Stronger Judge Design** — Judge README updated with designs for hidden/held-out tests, database side-effect verification, concurrency tests, fault injection, and adversarial security tests. Implementation proceeds incrementally per level.

3. **Level Gates with Concrete Thresholds** — Conformance gates defined per level (see above). Level 5 deferred pending feasibility study.

### Architecture

1. **Single binary architecture** - All components compile into one `megabase` binary
2. **Axum + Tokio** - HTTP framework and async runtime
3. **Workspace with crates** - One crate per component for modularity

### Rules (from GOAL.md, enforced here)

1. **README status must be current** - On every merged PR, the README status section and coverage badges must be updated. CI enforces this by regenerating coverage on every push to main.
2. **No test gaming** - Coverage only counts when conformance passes
3. **501 everywhere** - Unimplemented features return structured 501 errors

## Blocked Items

*None currently*

## Judge Disputes

*None currently*

---

## Phase 0 Checklist

- [x] Create Cargo workspace and empty crates
- [x] Binary starts and answers 501 everywhere
- [x] Add vendor/ submodules with stable release tags
- [x] Build judge/ with docker-compose
- [x] Extract coverage/units.json (334 units)
- [x] Build treemap generator
- [x] Create NOTICE, LICENSES/, PROGRESS.md, HUMAN_LOG.md
- [x] Create CODEOWNERS
- [x] Set up CI workflow
- [ ] GitHub Project board and milestones (requires admin - see scripts/github-setup.sh)
- [ ] Branch protection rules (requires admin)

**Status**: PHASE 0 COMPLETE

All Phase 0 deliverables implemented. Vendor pins, judge design, and external review proposals approved by agent coordinator.

## Human-Only Actions

The following require repository admin access (physically impossible for agents):

- [ ] Configure branch protection on `main` (require PR reviews, status checks)
- [ ] Enable CODEOWNERS enforcement in branch protection settings
- [ ] Create GitHub Project board (or run `scripts/github-setup.sh` with admin token)
- [ ] Set up bot account for agent actions (optional)

*Log completion in HUMAN_LOG.md when done.*
