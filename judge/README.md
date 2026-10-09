# Megabase Judge

The judge system runs the official Supabase self-hosted stack alongside Megabase and compares their responses to verify compatibility.

## Overview

The judge is the external arbiter of correctness. Agents do not grade themselves - every comparison runs against the real Supabase stack using pinned versions that match `vendor/`.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        Judge Harness                        │
│                        (compare.py)                         │
└─────────────────────────────────────────────────────────────┘
                    │                           │
                    ▼                           ▼
┌─────────────────────────────┐   ┌─────────────────────────────┐
│    Reference Supabase       │   │        Megabase             │
│    (pinned versions)        │   │     (under test)            │
│                             │   │                             │
│  ┌─────────────────────┐    │   │   ┌───────────────────┐     │
│  │ Auth (v2.197.0)     │    │   │   │ megabase binary   │     │
│  │ REST (v16.4)        │    │   │   │                   │     │
│  │ Realtime (v2.143.3) │    │   │   │ /rest/v1/*        │     │
│  │ Storage (v1.80.2)   │    │   │   │ /auth/v1/*        │     │
│  │ Meta (v0.100.0)     │    │   │   │ /storage/v1/*     │     │
│  └─────────────────────┘    │   │   │ /realtime/v1/*    │     │
│                             │   │   │ /functions/v1/*   │     │
└─────────────────────────────┘   │   │ /pg/*             │     │
                                  │   └───────────────────┘     │
                                  └─────────────────────────────┘
                    │                           │
                    └───────────┬───────────────┘
                                ▼
                    ┌───────────────────────┐
                    │     PostgreSQL        │
                    │    (shared, v15)      │
                    └───────────────────────┘
```

## Usage

### Prerequisites

- Docker and Docker Compose
- Python 3.10+ with `requests` library

### Starting the Stack

```bash
# From the judge/ directory
docker compose up -d

# Wait for services to be healthy
docker compose ps
```

### Running Comparisons

```bash
# Run all comparison tests
python compare.py

# Test a specific component
python compare.py --component auth

# Verbose output with diffs
python compare.py --verbose

# JSON output for CI
python compare.py --json > results.json
```

### Stopping

```bash
docker compose down
docker compose down -v  # Also remove volumes
```

## Normalization Rules

To compare responses fairly, the judge normalizes certain non-deterministic values:

| Value Type | Normalization |
|------------|---------------|
| Timestamps | Replaced with `TIMESTAMP_NORMALIZED` |
| UUIDs | Replaced with `UUID_NORMALIZED` |
| Order | Arrays sorted when order is not semantically significant |

These rules are documented in `compare.py` for transparency.

## Adding Tests

Add new test cases to the `get_test_cases()` function in `compare.py`:

```python
{
    "name": "Human-readable test name",
    "component": "rest|auth|storage|etc",
    "unit_id": "matches coverage/units.json",
    "method": "GET|POST|etc",
    "ref_path": "/path/on/reference",
    "meg_path": "/path/on/megabase",
    "headers": {"Authorization": "Bearer ..."},  # optional
    "json": {"body": "data"},  # optional
}
```

## Pinned Versions

The reference stack uses these pinned versions (must match `vendor/`):

| Component | Version |
|-----------|---------|
| Auth (GoTrue) | v2.197.0 |
| PostgREST | v16.4 |
| Realtime | v2.143.3 |
| Storage | v1.80.2 |
| Postgres Meta | v0.100.0 |
| PostgreSQL | 15.6.1.143 |

**DO NOT** update these versions without human approval and updating `vendor/` submodules.

## Changes to Judge

Per GOAL.md rule 2: Changes to `judge/` are made only on a separate branch, reviewed by the reviewer agent, and merged by the orchestrator. If you believe the judge is wrong, write it up in `PROGRESS.md` under "Judge disputes" and move on.

---

## Planned Enhancements

> **Status**: ✅ Accepted — approved by agent coordinator per [Proposal 0001](../docs/proposals/0001-external-review.md). Implementation proceeds incrementally.

### Hidden/Held-Out Tests

**Problem**: Agents can see all tests in `compare.py` and potentially overfit.

**Proposed Design**:
```
judge/
├── compare.py              # Visible tests (current)
├── held-out/               # Hidden from agents
│   ├── README.md           # Documents existence, not contents
│   ├── tests/              # NOT in git, synced separately
│   └── results/            # Historical results
```

- Held-out tests live in a **private repository** or encrypted store
- Run weekly by human maintainers or separate CI with secrets
- Results published to `held-out/results/` (scores only, not test details)
- Agents cannot access test logic but can see pass/fail rates
- Prevents overfitting while maintaining transparency on conformance

### Database Side-Effect Verification

**Problem**: HTTP response matching doesn't catch database state differences.

**Proposed Design**:
```python
# In compare.py, after each test:
def verify_database_state(test_name):
    ref_state = snapshot_database(REFERENCE_DB)
    meg_state = snapshot_database(MEGABASE_DB)
    
    # Compare relevant tables
    for table in ['auth.users', 'storage.objects', 'public.*']:
        diff = compare_tables(ref_state[table], meg_state[table])
        if diff:
            return TestResult(passed=False, diff=f"DB state mismatch: {diff}")
    
    return TestResult(passed=True)
```

**Tables to verify by component**:
- **Auth**: `auth.users`, `auth.sessions`, `auth.refresh_tokens`, `auth.identities`, `auth.mfa_factors`
- **Storage**: `storage.buckets`, `storage.objects`
- **Realtime**: Subscription state (in-memory, verify via API)
- **Meta**: Introspection accuracy (compare `pg_catalog` queries)

### Concurrency Tests

**Problem**: Single-request tests miss race conditions and isolation bugs.

**Proposed Design**:
```python
# concurrent_tests.py
async def test_concurrent_signups():
    """Verify no duplicate users from concurrent signups."""
    email = f"test-{uuid4()}@example.com"
    tasks = [signup(email, password) for _ in range(10)]
    results = await asyncio.gather(*tasks, return_exceptions=True)
    
    # Exactly one should succeed, rest should fail with conflict
    successes = [r for r in results if not isinstance(r, Exception)]
    assert len(successes) == 1
    
    # Verify same behavior on reference
    # ...
```

**Scenarios**:
- Concurrent signups with same email
- Concurrent token refresh
- Parallel file uploads to same path
- Connection pool exhaustion
- Realtime subscription during high load

### Fault Injection Tests

**Problem**: Happy-path tests miss error handling differences.

**Proposed Design**:
```yaml
# fault_scenarios.yml
scenarios:
  - name: database_disconnect
    action: kill_pg_connection
    verify: graceful_error_response
    
  - name: slow_query
    action: inject_pg_delay(5000ms)
    verify: timeout_handling
    
  - name: disk_full
    action: fill_storage_volume
    verify: upload_rejected_gracefully
```

**Implementation**: Use Docker network manipulation, `pg_terminate_backend()`, volume constraints.

### Adversarial Security Tests

**Problem**: Security-critical paths need more than functional testing.

**Proposed Test Categories**:

#### Auth Security
```python
def test_invalid_jwt_rejected():
    """Verify invalid JWTs don't grant access."""
    invalid_tokens = [
        "not.a.jwt",
        forge_jwt(wrong_secret),
        expired_jwt,
        jwt_with_modified_claims,
        jwt_with_none_algorithm,
    ]
    for token in invalid_tokens:
        assert request_with_token(token).status == 401

def test_no_privilege_escalation():
    """Verify users can't access other users' data."""
    user_a = create_user()
    user_b = create_user()
    
    # User A shouldn't see User B's data
    response = get_user_data(user_b.id, token=user_a.token)
    assert response.status == 403 or response.data == []
```

#### RLS Security
```python
def test_rls_sql_injection():
    """Verify filter values can't bypass RLS."""
    malicious_values = [
        "'; DROP TABLE users; --",
        "1 OR 1=1",
        "admin'--",
    ]
    for value in malicious_values:
        response = query_with_filter(f"role=eq.{value}")
        # Should either reject or return empty, never bypass
```

#### Pooler Security
```python
def test_connection_exhaustion():
    """Verify pooler handles connection exhaustion gracefully."""
    connections = [open_connection() for _ in range(MAX_CONNECTIONS + 10)]
    # Should queue or reject, not crash
    
def test_credential_isolation():
    """Verify tenant A can't use tenant B's credentials."""
    # Multi-tenant pooler scenarios
```

### Implementation Priority

| Enhancement | Priority | Dependency | Target Level |
|-------------|----------|------------|--------------|
| DB side-effect checks | High | None | Level 1 |
| Auth security tests | High | None | Level 1 |
| Hidden tests infra | Medium | CI secrets | Level 2 |
| Concurrency tests | Medium | Async harness | Level 3 |
| RLS security tests | High | Level 1 REST | Level 1 |
| Pooler security | Medium | Level 4 | Level 4 |
| Fault injection | Low | Docker control | Level 4 |

---

## Implementation Process

Per GOAL.md rule 2, changes to judge/:
1. Implementation on a separate branch
2. Reviewer agent approval before merge
3. Orchestrator merges approved changes
