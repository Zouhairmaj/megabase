# Judge normalization

Applied identically to the reference stack and to Megabase before a step
is compared. Implemented in `judge/harness/src/normalize.rs`. Change this
file and that module together, on a `review/*` branch.

## What is compared

On every step:

- HTTP status
- Headers, when present: `content-type`, `content-range`, `location`,
  `preference-applied`, plus any names listed in the case's
  `compare_headers`
- Body (JSON as `serde_json::Value`, otherwise text)

Headers the gateway invents (`date`, `server`, `via`, `x-kong-*`,
`content-length`, …) are ignored.

## Body

| Kind | Rule |
|---|---|
| Compact JWT (`eyJ….….…`) | replaced by its decoded payload as sorted JSON, `<jwt:{…}>`, without `iat`, `exp`, `jti` and the `timestamp` of each `amr` entry (the method stays compared); the rules below then apply inside it (UUID claims such as `session_id` become `<uuid>`). `role`, `aud`, `sub`, `aal`, `amr` and every other claim must match. The header and signature are not compared and no signature check is added. A payload that is not a JSON object becomes `<jwt>` |
| bcrypt hash (`$2a$10$…`) | `<bcrypt>` |
| ISO-8601 / RFC 3339 timestamp | `<timestamp>` |
| UUID | `<uuid>` |
| JSON keys `expires_at`, `iat`, `exp` | replaced by `"<key>"` when the value is not null |
| JSON key `refresh_token` | same; the *next* step can still capture the raw value before this runs |
| JSON pointer in the case's `ignore` | removed from both bodies |

`content-type` is compared case-insensitively with spaces stripped.

## What is not normalized

Error codes, error messages, JSON field names, row order when the request
asked for `order=`, and header names other than `content-type`. If those
differ, the case fails.

## Database

Applied identically to the reference database (`postgres`) and the
Megabase database (`megabase`) before a catalog or row snapshot is
compared. Implemented in `judge/harness/src/db.rs`.

| Kind | Compared | Normalized |
|---|---|---|
| Table catalog | column name and attnum order, `pg_type.typname`, `NOT NULL`, generated expression, `pg_get_expr` column default, table and column ACLs (`aclexplode`: grantee, privilege, grant option; grantor excluded), RLS policies (`pg_policy`: name, command, permissive flag, sorted roles, `USING`, `WITH CHECK`), `relrowsecurity`, `pg_get_indexdef`, `pg_get_constraintdef` (PK, UNIQUE, CHECK, FK) | SQL text (defaults and policy expressions included): strip `--` comments, collapse whitespace, lowercase, all outside `'quoted'` literals and `"quoted"` identifiers (their contents, including spaces and `--`, are kept exactly). A required object missing on both sides fails; `absent = true` passes only when both databases lack the relation |
| Function catalog | identity arguments, result type, language, `provolatile`, `prosrc` | same SQL normalization on result type and body |
| Row snapshot | `jsonb_agg(row_to_json(t))` of `SELECT *` | HTTP JSON rules (including bcrypt), plus non-empty `auth.users` secret columns (`encrypted_password`, `*_token`), then sort the row array by serialized text. Mutating HTTP cases compare the before/after row delta as a multiset (duplicate normalized rows are counted), not the full table. Empty token strings stay empty so an autoconfirmed clear still differs from a leftover token. |

Owners, grantors and comments are not compared: they depend on which
cluster role created the object and on cosmetic `COMMENT ON`. ACL entries
whose grantee is the object's owner (`relowner`) are dropped for the same
reason. ACL and
policy roles are compared by name, so both databases must live in the
same cluster (they do).
`storage.objects` is out of scope until Level 2.

## Captures

A step may set `capture.name = "/json/pointer"`. The value is read from
**that stack's own** response (before ignore-pointers, after the request)
and substituted as `{{name}}` in later steps on the same stack. `{{run}}`
is a unique id for the judge process, so sign-ups do not collide.
