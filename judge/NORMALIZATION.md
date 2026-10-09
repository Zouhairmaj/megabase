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

## Captures

A step may set `capture.name = "/json/pointer"`. The value is read from
**that stack's own** response (before ignore-pointers, after the request)
and substituted as `{{name}}` in later steps on the same stack. `{{run}}`
is a unique id for the judge process, so sign-ups do not collide.
