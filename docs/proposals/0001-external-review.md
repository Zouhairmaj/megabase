# Proposal 0001: External Review Recommendations

**Status**: Awaiting human approval  
**Source**: GPT-6 Astra external review  
**Date**: 2026-10-09

## Summary

External review of Phase 0 produced three recommendations to strengthen the Megabase experiment's rigor and measurability. These proposals do not modify GOAL.md (human-owned) but extend its implementation.

---

## Proposal 1: Bounded Compatibility Contract

### Problem

The current approach extracts units from vendor sources but doesn't formally document what "compatibility" means. This leaves ambiguity about:
- Which behaviors are in scope vs. intentionally excluded
- Which edge cases are considered implementation details vs. required behavior
- How to handle upstream bugs or undocumented behavior

### Proposed Solution

Create `docs/COMPATIBILITY.md` documenting:

1. **Pinned versions** - Explicit version matrix with commit SHAs
2. **In-scope endpoints** - Every endpoint Megabase will implement
3. **Excluded behaviors** - Explicitly out-of-scope features (e.g., cloud-only APIs, deprecated endpoints)
4. **Behavioral contracts** - Key invariants that must hold (e.g., "RLS policies are evaluated identically")
5. **Known divergences** - Intentional differences (if any) with rationale

### Coverage State Refinement

Extend the treemap/badge system to distinguish **three states per unit**:

| State | Color | Meaning |
|-------|-------|---------|
| Not implemented | Grey | Returns 501 |
| Implemented | Yellow | Returns something other than 501 |
| Tested | Blue | Has dedicated test coverage |
| Conformant | Green | Passes judge comparison against reference |

This prevents gaming where "implemented" could mean "returns 200 OK with wrong data."

### Files to Create/Modify

- Create: `docs/COMPATIBILITY.md`
- Modify: `scripts/generate-treemap.py` (add tested/conformant distinction)
- Modify: `coverage/units.json` schema (add `tested` field)

---

## Proposal 2: Stronger Judge Design

### Problem

The current judge design compares HTTP responses but has gaps:
- Agents can see all tests and potentially overfit
- Database side effects aren't verified
- Concurrent request behavior isn't tested
- Security-critical paths (Auth, RLS, pooler) lack adversarial testing

### Proposed Enhancements

#### 2.1 Hidden/Held-Out Tests

- Maintain a **private test suite** that agents cannot access
- Run periodically (weekly?) by humans or separate CI
- Results inform conformance but failures don't block merges
- Prevents overfitting to visible tests

#### 2.2 Database Side-Effect Verification

- After each test, compare database state between reference and Megabase
- Verify: rows inserted/updated/deleted, constraints satisfied, sequences advanced
- Essential for Auth (users table), Storage (objects table), RLS policies

#### 2.3 Concurrency Tests

- Send parallel requests to both stacks
- Verify: no race conditions, proper transaction isolation, connection pooling behavior
- Critical for: Pooler, Realtime subscriptions, concurrent Auth flows

#### 2.4 Fault Injection

- Test behavior under: database disconnection, slow queries, OOM conditions
- Verify graceful degradation matches reference behavior
- Important for production-readiness claims

#### 2.5 Adversarial Security Tests

- **Auth**: Invalid JWTs, expired tokens, privilege escalation attempts
- **RLS**: Policy bypass attempts, injection in filter values
- **Pooler**: Connection exhaustion, credential stuffing patterns
- Run against both stacks; any Megabase-only vulnerability is a blocker

### Implementation Notes

These require reviewer agent approval per GOAL.md rule 2. The judge README should document the design; implementation can be incremental.

---

## Proposal 3: Level Gates with Concrete Thresholds

### Problem

GOAL.md defines levels but doesn't specify:
- Exact conformance percentage required to advance
- How to handle cost tracking
- What happens on regressions
- Whether Level 5 is realistic given scope

### Proposed Level Gate Criteria

| Level | Components | Conformance Threshold | Additional Gates |
|-------|------------|----------------------|------------------|
| 1 | REST, Auth (email/password) | ≥95% | No P0 security issues |
| 2 | OAuth, Magic Links, Storage | ≥90% | Level 1 maintained ≥95% |
| 3 | Realtime | ≥85% | Levels 1-2 maintained |
| 4 | Functions, Pooler, Meta, Studio test | ≥80% | All levels maintained |
| 5 | Studio rewrite | **DEFERRED** | Requires separate proposal |

### Tracking Requirements

Add to PROGRESS.md:
- **Cost tracking**: Cumulative token spend and dollar cost
- **Regression log**: Any conformance decrease with date, commit, recovery
- **Human intervention count**: Running total from HUMAN_LOG.md

### Level 5 Deferral Rationale

Level 5 (Studio rewrite) is a stretch goal that:
- Depends on whether Studio can be statically exported
- May require a full Rust UI framework
- Is not necessary for the core experiment thesis

Recommend marking as "Deferred pending Level 4 completion and feasibility study."

---

## Implementation Plan

If approved, implementation order:

1. Create `docs/COMPATIBILITY.md` (immediate)
2. Update coverage schema for tested/conformant states (immediate)
3. Add level gates and tracking to PROGRESS.md (immediate)
4. Update judge README with stronger judge design (immediate, as design doc)
5. Implement hidden tests (post-Phase 0, requires infra)
6. Implement DB side-effect checks (Level 1)
7. Implement concurrency tests (Level 3)
8. Implement security tests (Level 1 Auth, Level 4 Pooler)

---

## Approval Required

These proposals modify the experiment's methodology. Per MANIFESTO.md, humans write the mission and guardrails. Please review and approve/reject each proposal:

- [ ] Proposal 1: Bounded Compatibility Contract
- [ ] Proposal 2: Stronger Judge Design
- [ ] Proposal 3: Level Gates with Concrete Thresholds

Approved proposals will be implemented in subsequent commits. Rejected proposals will be archived with rationale.
