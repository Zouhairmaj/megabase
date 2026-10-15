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
- `vendor/postgrest/src/library/PostgREST/SchemaCache.hs:142-160,530-583,877-1079`
  (`ViewKeyDependency`, `addViewM2OAndO2ORels`, `allViewsKeyDependencies`).
- `vendor/postgrest/src/library/PostgREST/Error.hs:245,257-258,280,409-445`
  (`NoRelBetween`, `AmbiguousRelBetween`, `compressedRel`, `relHint`).
- Pin: `postgrest v16.4` (`vendor.toml`).

## Inputs

`GET`/`HEAD /rest/v1/{relation}?select=<fields>,<name>(<fields>)`, with an
optional `alias:` prefix and up to two `!` params on `<name>`: a join type
(`!left`, `!inner`) and a hint (`!<constraint>` or `!<column>`), in either
order. With two of a kind the first wins (`pEmbedParams`). `<name>` is one
of:

- the embedded table or view, in the same schema (`notes(body)`,
  `authors?select=books_view(title)`);
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
- Views and materialized views inherit the keys of the tables they select.
  A view that selects every column of one end of a key (followed through
  nested views in the request schema and `public`, the extra search path)
  gets that key with its own column names: view to table, table to view,
  and view to view, with the base key's cardinality. A key column the view
  selects twice gives one relationship per view column. The view
  definition is read from `pg_rewrite.ev_action` as upstream does.
- `<name>` matches a relationship when it equals the other table or view,
  the constraint name, or the relationship's single column on the
  requested relation. A constraint name or column never matches a
  relationship whose other end is a view. Multi-column keys never match a
  column hint.
- With a hint, `<name>` must equal the other table, and the hint must equal
  the constraint name, the single column on the requested table, or the
  single column on the other table
  (`posts?select=users!posts_author_id_fkey(name)`,
  `users!editor_id(name)`, `users!id(name)`). Two hinted embeds of one
  table resolve separately
  (`author:users!author_id(name),editor:users!editor_id(name)`).
- Several relationships matched by a table name are `PGRST201`, except when
  a view is one end (501).
- One-to-many: the embed is a JSON array, `[]` when empty. Many-to-one and
  one-to-one: a JSON object, or `null` when no row matches.
- `!left` (the default) keeps every parent row.
- `!inner` keeps only parent rows with at least one embedded row left after
  the embed's own filters, order and page (`INNER JOIN LATERAL … ON agg IS
  NOT NULL`, `QueryBuilder.hs:107-128`; served as `EXISTS` over the same
  subquery).
- Embedded parameters (`Plan.hs:907-1110`, `addFilters`, `addOrders`,
  `addRanges`, `addLogicTrees`, `updateNode`): a key prefixed with the embed
  path targets that embed instead of the root. `books.published=eq.true`
  filters the embedded rows, `books.or=(…)` / `books.not.and=(…)` add a logic
  tree, `books.order=title.desc` orders them, and `books.limit=1` /
  `books.offset=1` page them. The first segment matches the embed's name or
  alias (`url-use-legacy-target-names`, default `true`); naming the relation
  of an aliased embed adds a deprecation `Warning` header upstream
  (`App.hs:230-290`) and stays 501 here. Filter values are
  typed with the embedded table's columns. A left embed still keeps every
  parent row; its array or object is filtered.
- Null embed filter (`addNullEmbedFilters`, `Plan.hs:1027-1043`): a root
  filter or logic-tree leaf named after an embed (its alias, else its name)
  with `is.null` keeps the parents whose filtered embed is empty (an
  anti-join, `NOT EXISTS`); `not.is.null` keeps the others. Any other
  operator on that name stays an ordinary column filter.
- `Prefer: count` (`readPlanToCountQuery`, `QueryBuilder.hs:260-296`) counts
  parents behind the root filters, each `!inner` embed and each null embed
  filter, using the embed's filters but not its order or page.
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

- An embedded parameter whose first segment is not an embed in `select`,
  or that names a nested embed (`books.tags.limit=1`): `PGRST108` (400),
  `'<name>' is not an embedded resource in this request`, hint
  `Verify that '<name>' is included in the 'select' query parameter.`
  Filters are checked first, then orders, ranges and logic trees; within
  one kind the rightmost parameter fails first (`foldr`).

## Out of scope (still 501)

Spread (`...name(...)`), nested embeds, empty embeds (`notes()`,
filter-only upstream), related orders (`order=clients(id)` on an embed, or
inside one), embedding to or
from a foreign table, self-references, several view relationships matched by
a table name, and embeds in write responses. Many-to-many through a junction
table is 501 too: whenever a
junction exists (a table with two distinct many-to-one keys, one to each
side, both inside its primary key, as `addM2MRels` builds), an unhinted
table target or a hint naming that junction is 501, since upstream would
list the many-to-many candidate.

## Judge cases

`rest.adversarial.rls.embed`, `rest.adversarial.rls.embed-inner`,
`rest.adversarial.rls.embed-comments`, `rest.adversarial.rls.anon-embed`, `rest.embed.left-one-to-many`,
`rest.embed.left-many-to-one`, `rest.embed.alias`,
`rest.embed.inner-filters-parents`, `rest.embed.no-relationship`,
`rest.embed.column-hint-alias`, `rest.embed.column-hint-key`,
`rest.embed.column-hint-inner`, `rest.embed.constraint-target`,
`rest.embed.column-hint-ambiguous`, `rest.embed.column-hint-not-a-key`.
Embedded filters (left and `!inner`), embedded order and limit, and the
`is.null` anti-join need cases in a `review/issue-377-cases` PR.
