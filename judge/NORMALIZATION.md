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
| Compact JWT (`eyJ….….…`) | `<jwt>` |
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
| Table catalog | column name and attnum order, `pg_type.typname`, `NOT NULL`, generated expression, `relrowsecurity`, `pg_get_indexdef` | SQL text: strip `--` comments, collapse whitespace, lowercase outside `'quoted'` literals. `absent = true` passes only when both databases lack the relation |
| Function catalog | identity arguments, result type, language, `provolatile`, `prosrc` | same SQL normalization on result type and body |
| Row snapshot | `jsonb_agg(row_to_json(t))` of `SELECT *` | same JSON rules as HTTP bodies (JWT, UUID, timestamp, volatile keys), then sort the array by serialized row text so `ORDER BY 1` is not the only order (the first `auth.users` column is `instance_id`, shared by every row) |

Owners, ACLs, `column_default` and comments are not compared: they
depend on which cluster roles exist and on cosmetic `COMMENT ON`.
`storage.objects` is out of scope until Level 2.

## Captures

A step may set `capture.name = "/json/pointer"`. The value is read from
**that stack's own** response (before ignore-pointers, after the request)
and substituted as `{{name}}` in later steps on the same stack. `{{run}}`
is a unique id for the judge process, so sign-ups do not collide.
