# REST: embedding

Status: draft
Unit ids: `rest:embed-join:inner`, `rest:embed-join:left`
Level: 1

Resource embedding in `select=`. Template:
[`specs/_template.md`](../_template.md).

## Upstream

- `vendor/postgrest/src/library/PostgREST/ApiRequest/QueryParams.hs:528-535,647-658`
  (`pRelationSelect`, `pEmbedParams`: `!left`, `!inner`, `!hint`).
- `vendor/postgrest/src/library/PostgREST/Plan.hs:652-728` (`findRel`).
- `vendor/postgrest/src/library/PostgREST/SchemaCache.hs:212-229,585-609,783-829`
  (`getOverrideRelationshipsMap` sort, `addInverseRels`, `addM2MRels`,
  `allM2OandO2ORels`).
- `vendor/postgrest/src/library/PostgREST/Error.hs:245,257-258,280,409-445`
  (`NoRelBetween`, `AmbiguousRelBetween`, `compressedRel`, `relHint`).
- Pin: `postgrest v16.4` (`vendor.toml`).

## Inputs

`GET`/`HEAD /rest/v1/{relation}?select=<fields>,<name>(<fields>)`, with an
optional `alias:` prefix and up to two `!` params on `<name>`: a join type
(`!left`, `!inner`) and a hint (`!<constraint>` or `!<column>`), in either
order. With two of a kind the first wins (`pEmbedParams`). `<name>` is one
of:

- the embedded table, in the same schema (`notes(body)`);
- a foreign key constraint name (`note_comments_note_id_fkey(body)`);
- a column hint: the single column of a foreign key, as seen from the
  requested table (`messages?select=author:user_id(*)`). From the
  referencing table that is the key column; from the referenced table it
  is the referenced column (`profiles?select=id(...)`).

## Behaviour

- Candidates are the foreign keys of one schema with an end on the
  requested table (`conparentid = 0`), read from `pg_constraint`. Each key
  gives a relationship from the referencing side (many-to-one, or
  one-to-one when the key columns are a primary key or unique constraint)
  and one from the referenced side (one-to-many, or one-to-one).
- `<name>` matches a relationship when it equals the other table, the
  constraint name, or the relationship's single column on the requested
  table. Multi-column keys never match a column hint.
- With a hint, `<name>` must equal the other table, and the hint must equal
  the constraint name, the single column on the requested table, or the
  single column on the other table
  (`posts?select=users!posts_author_id_fkey(name)`,
  `users!editor_id(name)`, `users!id(name)`). Two hinted embeds of one
  table resolve separately
  (`author:users!author_id(name),editor:users!editor_id(name)`).
- One-to-many: the embed is a JSON array, `[]` when empty. Many-to-one and
  one-to-one: a JSON object, or `null` when no row matches.
- `!left` (the default) keeps every parent row.
- `!inner` keeps only parent rows with at least one matching embedded row
  (`EXISTS`). The embedded rows are not otherwise filtered.
- The embedded query runs as the request role, so grants and row-level
  security apply. A missing grant is the database error (`42501`).
- The output key is the alias, else `<name>` as written (`user_id` for
  `user_id(*)`). Keys follow select order.

## Errors

- No relationship matches: `PGRST200` (400). With a hint, `details` reads
  `Searched for a foreign key relationship between '<relation>' and
  '<name>' using the hint '<hint>' in the schema '<schema>', but no matches
  were found.`
- Several relationships match (a table name with several keys, a column
  hint, a constraint name, or a hint on a shared referenced column): `PGRST201`
  (300). `message` is `Could not embed because more than one relationship
  was found for '<relation>' and '<name>'`. `details` lists each candidate
  as `{cardinality, embedding, relationship}`
  (`"many-to-one"`, `"<relation> with <other>"`,
  `"<constraint> using <relation>(<cols>) and <other>(<cols>)"`), sorted
  like upstream's `Relationship` `Ord`: other table, self flag,
  cardinality (one-to-many, many-to-one, one-to-one), constraint, columns.
  `hint` is `Try changing '<name>' to one of the following:
  '<other>!<constraint>', .... Find the desired relationship in the
  'details' key.`

## Out of scope (still 501)

Spread (`...name(...)`), nested
embeds, empty embeds (`notes()`, filter-only upstream), filters/order/limit on embedded resources (`notes.order=body` is 501, not
`PGRST108`), embedding a view, materialized view or foreign table,
self-references, and embeds in write responses. Many-to-many through a
junction table is 501 too: whenever a junction exists (a table with two
distinct many-to-one keys, one to each side, both inside its primary key,
as `addM2MRels` builds), an unhinted table target or a hint naming that
junction is 501, since upstream would list the many-to-many candidate.

## Judge cases

`rest.adversarial.rls.embed`, `rest.adversarial.rls.embed-inner`,
`rest.adversarial.rls.embed-comments`, `rest.adversarial.rls.anon-embed`, `rest.embed.left-one-to-many`,
`rest.embed.left-many-to-one`, `rest.embed.alias`,
`rest.embed.inner-filters-parents`, `rest.embed.no-relationship`,
`rest.embed.column-hint-alias`, `rest.embed.column-hint-key`,
`rest.embed.column-hint-inner`, `rest.embed.constraint-target`,
`rest.embed.column-hint-ambiguous`, `rest.embed.column-hint-not-a-key`.
