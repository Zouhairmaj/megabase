<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/wordmark-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="assets/wordmark.svg">
  <img alt="Megabase" src="assets/wordmark.svg" width="320">
</picture>

# Supabase, rewritten in Rust. By agents. In public.

[![CI](https://github.com/AgenP/megabase/actions/workflows/ci.yml/badge.svg)](https://github.com/AgenP/megabase/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Coverage](coverage/badge-coverage.svg)](coverage/treemap.svg)
[![Conformance](coverage/badge-conformance.svg)](coverage/treemap.svg)
[![Units](coverage/badge-units.svg)](coverage/units.json)

> **Early Experiment — Day 0**
>
> Megabase is an experiment in autonomous software development. AI agents are building a complete, drop-in compatible Supabase clone by reading the upstream source code and reproducing its behavior exactly.
>
> This is not production-ready software. Everything currently returns `501 NOT IMPLEMENTED`.

---

## What is Megabase?

Megabase aims to be a **single, lightweight binary** that serves the exact same APIs as a full Supabase self-hosted stack. Any app built with `supabase-js` or any other Supabase client should run against Megabase **without changing a line of code**.

| Supabase | Megabase |
|----------|----------|
| ~12 containers | 1 binary |
| 6 languages (Haskell, Go, Elixir, TypeScript, Rust, Lua) | Rust |
| ~2GB+ RAM | Target: <256MB |

## Status

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="coverage/treemap.svg">
  <img alt="Coverage Treemap" src="coverage/treemap.svg" width="100%">
</picture>

### Component Coverage

| Component | Coverage | Conformance | Units |
|-----------|----------|-------------|-------|
| ![REST](coverage/badge-coverage.svg) REST API | 0.0% | 0.0% | 57 |
| ![Auth](coverage/badge-coverage.svg) Auth | 0.0% | 0.0% | 81 |
| ![Realtime](coverage/badge-coverage.svg) Realtime | 0.0% | 0.0% | 33 |
| ![Storage](coverage/badge-coverage.svg) Storage | 0.0% | 0.0% | 34 |
| ![Functions](coverage/badge-coverage.svg) Functions | 0.0% | 0.0% | 27 |
| ![Pooler](coverage/badge-coverage.svg) Pooler | 0.0% | 0.0% | 13 |
| ![Meta](coverage/badge-coverage.svg) Meta | 0.0% | 0.0% | 51 |
| ![Studio](coverage/badge-coverage.svg) Studio | 0.0% | 0.0% | 38 |
| **Total** | **0.0%** | **0.0%** | **334** |

<details>
<summary>How coverage is computed</summary>

**The denominator (334 units)** is extracted once from the pinned `vendor/` sources. Each unit represents one route, query operator, auth flow, message type, or feature that Megabase must implement. The unit list is in [`coverage/units.json`](coverage/units.json).

- **Coverage**: percentage of units where Megabase returns something other than `501 NOT IMPLEMENTED`
- **Conformance**: percentage of units where Megabase's response is **identical** to the reference Supabase stack (after normalizing timestamps, UUIDs, and other non-deterministic values)

A unit only counts as "done" when it is both implemented AND conformant. The treemap shows one square per unit: green = conformant, yellow = implemented but not conformant, grey = not implemented.
</details>

### Levels

| Level | Description | Status |
|-------|-------------|--------|
| 1 | REST API + email/password auth | 🔲 Not started |
| 2 | OAuth, magic links, Storage | 🔲 Not started |
| 3 | Realtime | 🔲 Not started |
| 4 | Functions, Pooler, Meta, Studio test | 🔲 Not started |
| 5 | Studio rewrite (stretch) | 🔲 Not started |

### Verified Apps

*No apps verified yet — coming after Level 1*

[Submit your app →](https://github.com/AgenP/megabase/issues/new?labels=verified-app)

---

## Quick Start

### Prerequisites

- **Rust 1.75+** — Install via [rustup](https://rustup.rs/)
- **PostgreSQL 15+** — Megabase connects to your existing Postgres

### Build from Source

```bash
# Clone the repository
git clone https://github.com/AgenP/megabase.git
cd megabase

# Build release binary
cargo build --release

# Binary is at target/release/megabase
```

### Run

```bash
# Set required environment variables
export DATABASE_URL="postgres://postgres:postgres@localhost:5432/postgres"
export JWT_SECRET="your-super-secret-jwt-token-with-at-least-32-characters"

# Optional configuration
export MEGABASE_HOST="0.0.0.0"  # Default: 0.0.0.0
export MEGABASE_PORT="8000"      # Default: 8000

# Run Megabase
./target/release/megabase
```

### Verify it's running

```bash
curl http://localhost:8000/health
# {"status":"healthy","version":"0.1.0"}

# All endpoints currently return 501 NOT IMPLEMENTED
curl http://localhost:8000/rest/v1/users
# {"code":"MEGABASE_NOT_IMPLEMENTED","component":"rest","unit":"GET /rest/v1/users","message":"..."}
```

### Docker (Planned)

Docker images are not yet available. For now, build from source.

---

## Usage with supabase-js

Once endpoints are implemented, you'll be able to point any Supabase client at Megabase:

```javascript
import { createClient } from '@supabase/supabase-js'

// Point to your Megabase instance instead of Supabase
const supabase = createClient(
  'http://localhost:8000',  // Megabase URL
  'your-anon-key'           // Same JWT format as Supabase
)

// All Supabase operations work identically
const { data, error } = await supabase
  .from('users')
  .select('*')
  .eq('status', 'active')
```

> **Note**: This example will return `501 NOT IMPLEMENTED` until Level 1 is complete.

---

## Running the Judge

The judge compares Megabase responses against the real Supabase stack to verify compatibility.

### Start the Reference Stack

```bash
cd judge
docker compose up -d

# Wait for services to be healthy
docker compose ps
```

### Run Comparisons

```bash
# Install Python dependencies
pip install requests

# Run all comparison tests
python judge/compare.py

# Test specific component
python judge/compare.py --component auth

# Verbose output with diffs
python judge/compare.py --verbose

# JSON output for CI
python judge/compare.py --json > results.json
```

### Read Coverage Treemaps

Coverage visualizations are in the `coverage/` directory:

- `coverage/treemap.svg` — Combined view of all components
- `coverage/treemap-{component}.svg` — Per-component views
- `coverage/units.json` — Full unit list with status
- `coverage/summary.json` — Current metrics

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         megabase binary                         │
├─────────────────────────────────────────────────────────────────┤
│  megabase-server (gateway)                                      │
│  Routes: /rest/v1, /auth/v1, /storage/v1, /realtime/v1, etc.    │
├─────────┬─────────┬─────────┬─────────┬─────────┬───────────────┤
│  rest   │  auth   │realtime │ storage │functions│   ...more     │
│(PostgREST)│(GoTrue)│         │         │(Edge RT)│               │
├─────────┴─────────┴─────────┴─────────┴─────────┴───────────────┤
│                        megabase-core                            │
│                  (shared types, JWT, errors)                    │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
                    ┌───────────────────┐
                    │    PostgreSQL     │
                    │   (external)      │
                    └───────────────────┘
```

### Project Structure

```
megabase/
├── crates/
│   ├── megabase/           # Binary entry point
│   ├── megabase-core/      # Shared types, config, JWT, errors
│   ├── megabase-server/    # Gateway, routes all requests
│   ├── megabase-rest/      # PostgREST compatibility
│   ├── megabase-auth/      # GoTrue compatibility
│   ├── megabase-realtime/  # Realtime compatibility
│   ├── megabase-storage/   # Storage compatibility
│   ├── megabase-functions/ # Edge Functions compatibility
│   ├── megabase-pooler/    # Supavisor compatibility
│   ├── megabase-meta/      # Postgres Meta compatibility
│   └── megabase-studio/    # Studio API compatibility
├── vendor/                 # Pinned upstream sources (git submodules)
├── judge/                  # Comparison harness
├── coverage/               # Metrics and treemaps
├── scripts/                # Build and analysis scripts
├── assets/                 # Branding
├── MANIFESTO.md           # Project philosophy
├── GOAL.md                # Agent operating manual
└── PROGRESS.md            # Current state and decisions
```

---

## Contributing

Megabase is built by AI agents following strict rules. Human contributions are welcome for:

- **Bug reports** — Found something wrong? [Open an issue](https://github.com/AgenP/megabase/issues)
- **Verified apps** — Got a Supabase app working on Megabase? Let us know!
- **Discussion** — Ideas, questions, feedback

### For Agents

Read [`GOAL.md`](GOAL.md) — it's your operating manual. Key rules:

1. **Rust only** — All code is Rust
2. **Never modify `vendor/` or `judge/`** without human approval
3. **Never skip tests** to improve scores
4. **No regressions** — Conformance must not decrease
5. **Fail loudly** — Return `501 MEGABASE_NOT_IMPLEMENTED` for anything not done
6. **Credit sources** — Link to upstream files you're porting

### For Humans

Read [`MANIFESTO.md`](MANIFESTO.md) — it explains the experiment.

- Humans write only the mission and guardrails
- Every human intervention is logged in [`HUMAN_LOG.md`](HUMAN_LOG.md)
- The count of interventions is part of the result

---

## License

Megabase is licensed under the **Apache License 2.0**. See [LICENSE](LICENSE).

Each upstream component's license is preserved. See [NOTICE](NOTICE) for credits and [LICENSES/](LICENSES/) for full license texts.

---

## Disclaimer

Megabase is an **independent experiment**. It is:

- ❌ **NOT** affiliated with Supabase, Inc.
- ❌ **NOT** endorsed by Supabase, Inc.
- ❌ **NOT** sponsored by Supabase, Inc.

"Supabase" is a trademark of Supabase, Inc. and is used here only to describe API compatibility.

---

<p align="center">
  <a href="MANIFESTO.md">Manifesto</a> •
  <a href="GOAL.md">Agent Manual</a> •
  <a href="PROGRESS.md">Progress</a> •
  <a href="coverage/treemap.svg">Coverage</a>
</p>
