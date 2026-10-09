# Megabase Compatibility Contract

This document defines the bounded scope of Supabase compatibility that Megabase aims to achieve. It serves as the authoritative reference for what "drop-in compatible" means.

> **Status**: Draft — awaiting human approval per [Proposal 0001](proposals/0001-external-review.md)

---

## Pinned Upstream Versions

Megabase targets compatibility with these specific versions. The denominator for coverage is computed from these sources and remains fixed.

| Component | Repository | Version | Commit | Release Date |
|-----------|------------|---------|--------|--------------|
| Auth (GoTrue) | supabase/auth | v2.197.0 | 4eee58f296d9698a1c2c0ae14d7a0b379c7622d3 | 2026-09-09 |
| REST API (PostgREST) | PostgREST/postgrest | v16.4 | 0d97c05d8d23bd8a83c867abc45ef481a6d5d2c7 | 2026-09-24 |
| Realtime | supabase/realtime | v2.143.3 | d46540b3a4a79113f40301d5e5cb690f41150b9b | 2026-10-09 |
| Storage | supabase/storage | v1.80.2 | db42280506a07a165376219b2d6ac7bc92ccdfcc | 2026-10-08 |
| Pooler (Supavisor) | supabase/supavisor | v2.9.13 | 5a51fbfadaa6d6286c1fee0a95d4c629e81a2750 | 2026-09-10 |
| Postgres Meta | supabase/postgres-meta | v0.100.0 | 9a2effd147f2f028fa36553ec2c7eeec80d7f8fc | 2026-10-06 |
| Edge Runtime | supabase/edge-runtime | v1.77.4 | d4a4f606a90e8c66864219c9b1d31ef0e3e3f626 | 2026-10-01 |
| Monorepo (Studio) | supabase/supabase | v1.26.08 | 86854671e95be31e24fa0785cded0579fe692fcb | 2026-08-07 |
| Client Library | supabase/supabase-js | v2.117.3 | 4d23055805ce2cf853686b704cfaec1a701d29c6 | 2026-10-07 |

---

## In-Scope Endpoints

### REST API (`/rest/v1`)

All PostgREST functionality as documented in the pinned version:

- **Table operations**: GET, POST, PATCH, DELETE on any table
- **RPC**: GET/POST to `/rpc/:function`
- **Query operators**: eq, neq, gt, gte, lt, lte, like, ilike, in, is, cs, cd, ov, sl, sr, nxl, nxr, adj, not, or, and, all, any, fts, plfts, phfts, wfts
- **Query features**: select, order, limit, offset, range headers, resource embedding, computed columns, aggregates
- **Preferences**: return=representation, count=exact/planned/estimated, resolution=merge-duplicates, missing=default

### Auth (`/auth/v1`)

All GoTrue endpoints:

- **Core flows**: signup, token (password/refresh/pkce), logout, user, recover, verify, otp, magiclink
- **MFA**: factors (list, enroll, challenge, verify, unenroll)
- **OAuth**: authorize, callback for all supported providers
- **Admin**: users CRUD, generate_link, audit, SSO provider management
- **Settings/health**: settings, health

### Realtime (`/realtime/v1`)

- **WebSocket protocol**: Phoenix channels with all message types
- **Database changes**: INSERT, UPDATE, DELETE, TRUNCATE via logical replication
- **Broadcast**: Client-to-client messaging
- **Presence**: Track/untrack with state synchronization
- **Filters**: eq, neq, gt, gte, lt, lte, in on change events

### Storage (`/storage/v1`)

- **Buckets**: list, create, get, update, delete, empty
- **Objects**: upload, download, list, move, copy, delete, signed URLs, public URLs
- **Resumable uploads**: TUS protocol (create, upload, info, cancel)
- **Image transforms**: resize, format conversion, quality

### Edge Functions (`/functions/v1`)

- **Invocation**: All HTTP methods to `/:function_name`
- **Web APIs**: fetch, crypto, streams, URL, Headers, Request, Response, etc.
- **Supabase context**: Environment variables, secrets, JWT verification

### Pooler (PostgreSQL wire protocol)

- **Pooling modes**: transaction, session, statement
- **Auth**: MD5, SCRAM-SHA-256
- **Features**: Prepared statements, connection limits, tenant isolation

### Postgres Meta (`/pg`)

- **Introspection**: schemas, tables, columns, functions, triggers, types, extensions, publications, roles, policies
- **CRUD**: Create, update, delete for all introspectable objects
- **Query**: SQL execution, TypeScript type generation

### Studio (`/studio`)

- **API routes**: All server-side API endpoints Studio calls
- **Compatibility test**: Official Studio running unmodified against Megabase

---

## Explicitly Excluded

The following are **intentionally out of scope**:

### Cloud-Only Features
- Billing and subscription management
- Organization/team management APIs
- Cloud project provisioning
- Usage analytics and quotas
- Supabase CLI cloud commands

### Deprecated/Internal
- Endpoints marked deprecated in pinned versions
- Internal service-to-service APIs not exposed to clients
- Debug/profiling endpoints

### Platform Infrastructure
- Edge network / CDN behavior
- Geographic routing
- Log aggregation (beyond local logging)
- Metrics collection (beyond local metrics)

### PostgreSQL Itself
- PostgreSQL internals, extensions, or SQL behavior
- Megabase runs on standard PostgreSQL; it doesn't reimplement it

---

## Behavioral Contracts

These invariants must hold for Megabase to be considered compatible:

### Security

1. **JWT validation identical**: Same secrets, algorithms, expiry handling
2. **RLS enforcement identical**: Policies evaluate to same allow/deny decisions
3. **Auth flows secure**: No authentication bypasses not present in reference
4. **Storage ACLs enforced**: Same bucket/object access control

### Data Integrity

1. **Same database schema**: auth.users, storage.objects, etc. have identical structure
2. **Same constraints**: Foreign keys, unique constraints, check constraints
3. **Same triggers**: Database triggers fire in same conditions

### API Semantics

1. **Same status codes**: For identical requests
2. **Same response structure**: JSON shape, field names, types
3. **Same error codes**: Error responses match format and codes
4. **Same headers**: Content-Type, Cache-Control, CORS headers

### Performance (Non-Blocking)

Performance differences are tracked but don't block conformance:
- Latency within 2x of reference is acceptable
- Memory usage differences are documented but allowed

---

## Known Divergences

Any intentional differences from reference behavior are documented here:

*None currently — Megabase aims for exact compatibility.*

If divergences become necessary, they will be:
1. Documented here with rationale
2. Logged in HUMAN_LOG.md as a human decision
3. Clearly marked in the codebase

---

## Unit States

Each unit in `coverage/units.json` has one of four states:

| State | Badge Color | Definition |
|-------|-------------|------------|
| `not_implemented` | Grey | Returns 501 MEGABASE_NOT_IMPLEMENTED |
| `implemented` | Yellow | Returns a response other than 501 |
| `tested` | Blue | Has dedicated test coverage in judge |
| `conformant` | Green | Passes judge comparison with reference |

**Progression rules:**
- A unit can only be `tested` if it is `implemented`
- A unit can only be `conformant` if it is `tested` AND passes
- Coverage % = implemented / total
- Conformance % = conformant / total
- Test coverage % = tested / total

---

## Version Updates

This contract is pinned to specific versions. Updates require:

1. Human approval (logged in HUMAN_LOG.md)
2. Re-extraction of units from new vendor versions
3. Re-run of all judge comparisons
4. Update to this document with new version matrix

The denominator may change when versions update. Historical coverage is tracked against the denominator at that time.
