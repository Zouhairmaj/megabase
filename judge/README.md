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
