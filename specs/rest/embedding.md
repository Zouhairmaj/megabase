# REST: embedding

Status: draft
Unit ids: `rest:embed-join:inner`, `rest:embed-join:left`
Level: 1

Resource embedding in `select=`. Template:
[`specs/_template.md`](../_template.md).

## Upstream

- `vendor/postgrest/src/library/PostgREST/ApiRequest/QueryParams.hs:656-657`
  (`pJoinHint` / `!left`, `!inner`).
- Pin: `postgrest v16.4` (`vendor.toml`).

## Inputs

`GET`/`HEAD /rest/v1/{relation}?select=<fields>,<name>(<fields>)`, with an
optional `alias:` prefix and a `!left` or `!inner` suffix on `<name>`.
`<name>` is the embedded table in the same schema.

## Behaviour

- The relationship is the single foreign key between the two tables, read
  from `pg_constraint` in either direction.
- Foreign key on the requested table: the embed is a JSON object, or `null`
  when no row matches.
- Foreign key on the embedded table: the embed is a JSON array, `[]` when
  empty.
- `!left` (the default) keeps every parent row.
- `!inner` keeps only parent rows with at least one matching embedded row
  (`EXISTS`). The embedded rows are not otherwise filtered.
- The embedded query runs as the request role, so grants and row-level
  security apply. A missing grant is the database error (`42501`).
- The output key is the alias, else the table name. Keys follow select order.

## Errors

- No foreign key between the tables: `PGRST200` (400).

## Out of scope (still 501)

Join hints (`!fk_name`, column hints), spread (`...name(...)`), nested
embeds, filters/order/limit on embedded resources, many-to-many through a
junction table, self-references, several candidate keys (`PGRST201`), and
embeds in write responses.

## Judge cases

`rest.adversarial.rls.embed`, `rest.adversarial.rls.embed-inner`,
`rest.adversarial.rls.embed-comments`, `rest.adversarial.rls.anon-embed`.
