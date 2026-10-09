# REST: prefer

Status: stub
Unit ids: `rest:prefer:` + `count=exact|planned|estimated`,
`handling=strict|lenient`, `max-affected=*`, `missing=default|null`,
`resolution=merge-duplicates|ignore-duplicates`,
`return=minimal|representation|headers-only`, `timezone=*`, `tx=commit|rollback`
Level: 1

The `Prefer` request header and its `Preference-Applied` response. Template:
[`specs/_template.md`](../_template.md).

## Upstream

- `vendor/postgrest/src/library/PostgREST/ApiRequest/Preferences.hs`
- Pin: `postgrest v16.4` (`vendor.toml`).

## To write

Inputs, outputs, errors, edge cases, out of scope, judge cases.
