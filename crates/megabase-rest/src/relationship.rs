// Ported from PostgREST src/library/PostgREST/Plan.hs `findRel`,
// src/library/PostgREST/SchemaCache.hs `addViewM2OAndO2ORels` and
// src/library/PostgREST/Error.hs `compressedRel` / `relHint` (MIT), pin v16.4.
//! Pick the relationship an embed target names (`findRel`).
//!
//! The target is a table or view name, a foreign key constraint name, or the
//! single column of a foreign key on the requested table (a column hint, for
//! example `messages?select=author:user_id(*)`). A `!hint` after a table
//! target picks among its keys by constraint name, key column, or referenced
//! column (`posts?select=users!editor_id(*)`). Candidates are the foreign
//! keys between relations of one schema, in both directions, as
//! `allM2OandO2ORels` and `addInverseRels` in `SchemaCache.hs` list them,
//! plus the keys a view inherits from the tables it selects
//! (`addViewM2OAndO2ORels`).

use std::cmp::Ordering;

use crate::params::Relationship;

/// One foreign key constraint, or a view relationship derived from one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForeignKey {
    /// Constraint name.
    pub name: String,
    /// Schema of `table`.
    pub schema: String,
    /// Table (or view) that holds the key.
    pub table: String,
    /// Schema of `foreign_table`.
    pub foreign_schema: String,
    /// Referenced table (or view).
    pub foreign_table: String,
    /// `(column, referenced column)` pairs in key order.
    pub columns: Vec<(String, String)>,
    /// The key columns are a primary key or unique constraint of the base table.
    pub one_to_one: bool,
    /// `table` is a view (`relTableIsView`).
    pub table_is_view: bool,
    /// `foreign_table` is a view (`relFTableIsView`).
    pub foreign_is_view: bool,
}

/// Which end of a foreign key a view selects (`KeyDep`, without `PKDep`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyDepKind {
    /// `FKDep`: the referencing columns.
    Referencing,
    /// `FKDepRef`: the referenced columns.
    Referenced,
}

/// A view that selects every column of one end of a foreign key
/// (`ViewKeyDependency`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ViewKeyDep {
    /// Schema of the base table.
    pub table_schema: String,
    /// Base table.
    pub table: String,
    /// Schema of the view.
    pub view_schema: String,
    /// The view.
    pub view: String,
    /// Constraint name.
    pub constraint: String,
    /// Which end of the key the view selects.
    pub kind: KeyDepKind,
    /// Each key column of the table, in key order, with every view column
    /// that selects it.
    pub columns: Vec<(String, Vec<String>)>,
}

/// `expandKeyDepCols`: one `(table column, view column)` list per choice of
/// view column for each key column.
fn expand_key_dep_cols(cols: &[(String, Vec<String>)]) -> Vec<Vec<(&str, &str)>> {
    let mut out: Vec<Vec<(&str, &str)>> = vec![Vec::new()];
    for (table_col, view_cols) in cols {
        out = out
            .iter()
            .flat_map(|prefix| {
                view_cols.iter().map(move |view_col| {
                    let mut next = prefix.clone();
                    next.push((table_col.as_str(), view_col.as_str()));
                    next
                })
            })
            .collect();
    }
    out
}

/// Every expansion of the dependencies of one key end.
fn expanded<'a>(
    deps: &'a [ViewKeyDep],
    schema: &str,
    table: &str,
    constraint: &str,
    kind: KeyDepKind,
) -> Vec<(&'a ViewKeyDep, Vec<(&'a str, &'a str)>)> {
    deps.iter()
        .filter(|dep| {
            dep.table_schema == schema
                && dep.table == table
                && dep.constraint == constraint
                && dep.kind == kind
        })
        .flat_map(|dep| {
            expand_key_dep_cols(&dep.columns)
                .into_iter()
                .map(move |cols| (dep, cols))
        })
        .collect()
}

/// `addViewM2OAndO2ORels`: `keys` followed by the view-to-table,
/// table-to-view and view-to-view relationships `deps` derive from them.
pub(crate) fn add_view_keys(keys: Vec<ForeignKey>, deps: &[ViewKeyDep]) -> Vec<ForeignKey> {
    let mut derived = Vec::new();
    for key in keys
        .iter()
        .filter(|key| !key.table_is_view && !key.foreign_is_view)
    {
        let from_views = expanded(
            deps,
            &key.schema,
            &key.table,
            &key.name,
            KeyDepKind::Referencing,
        );
        let to_views = expanded(
            deps,
            &key.foreign_schema,
            &key.foreign_table,
            &key.name,
            KeyDepKind::Referenced,
        );
        let derive = |from: (&str, &str, bool), to: (&str, &str, bool), columns| ForeignKey {
            name: key.name.clone(),
            schema: from.0.to_string(),
            table: from.1.to_string(),
            foreign_schema: to.0.to_string(),
            foreign_table: to.1.to_string(),
            columns,
            one_to_one: key.one_to_one,
            table_is_view: from.2,
            foreign_is_view: to.2,
        };
        let base_from = (key.schema.as_str(), key.table.as_str(), false);
        let base_to = (
            key.foreign_schema.as_str(),
            key.foreign_table.as_str(),
            false,
        );
        for (dep, cols) in &from_views {
            let columns = cols
                .iter()
                .zip(&key.columns)
                .map(|((_, view_col), (_, foreign_col))| {
                    ((*view_col).to_string(), foreign_col.clone())
                })
                .collect();
            derived.push(derive(
                (dep.view_schema.as_str(), dep.view.as_str(), true),
                base_to,
                columns,
            ));
        }
        for (dep, cols) in &to_views {
            let columns = key
                .columns
                .iter()
                .zip(cols)
                .map(|((table_col, _), (_, view_col))| (table_col.clone(), (*view_col).to_string()))
                .collect();
            derived.push(derive(
                base_from,
                (dep.view_schema.as_str(), dep.view.as_str(), true),
                columns,
            ));
        }
        for (from, from_cols) in &from_views {
            for (to, to_cols) in &to_views {
                let columns = from_cols
                    .iter()
                    .zip(to_cols)
                    .map(|((_, view_col), (_, other_col))| {
                        ((*view_col).to_string(), (*other_col).to_string())
                    })
                    .collect();
                derived.push(derive(
                    (from.view_schema.as_str(), from.view.as_str(), true),
                    (to.view_schema.as_str(), to.view.as_str(), true),
                    columns,
                ));
            }
        }
    }
    let mut out = keys;
    out.append(&mut derived);
    out
}

/// `Cardinality` in `SchemaCache/Relationship.hs`, without many-to-many.
/// Variant order is the derived `Ord` upstream sorts candidates with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Cardinality {
    OneToMany,
    ManyToOne,
    /// `isParent`: the requested table is the referenced side.
    OneToOne {
        parent: bool,
    },
}

/// A relationship from the requested table to `foreign_table`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate<'a> {
    table: &'a str,
    foreign_table: &'a str,
    is_self: bool,
    cardinality: Cardinality,
    constraint: &'a str,
    /// `(column on table, column on foreign_table)` pairs.
    columns: Vec<(&'a str, &'a str)>,
    table_is_view: bool,
    foreign_is_view: bool,
}

impl Candidate<'_> {
    /// The `Ord` of PostgREST's `Relationship` with a fixed `relTable`.
    fn sort_cmp(&self, other: &Self) -> Ordering {
        let rank = |card: Cardinality| match card {
            Cardinality::OneToMany => 0,
            Cardinality::ManyToOne => 1,
            Cardinality::OneToOne { .. } => 2,
        };
        self.foreign_table
            .cmp(other.foreign_table)
            .then(self.is_self.cmp(&other.is_self))
            .then(rank(self.cardinality).cmp(&rank(other.cardinality)))
            .then(self.constraint.cmp(other.constraint))
            .then(self.columns.cmp(&other.columns))
            .then(self.cardinality.cmp(&other.cardinality))
            .then(self.table_is_view.cmp(&other.table_is_view))
            .then(self.foreign_is_view.cmp(&other.foreign_is_view))
    }

    /// `compressedRel`: one entry of the `PGRST201` details array.
    pub(crate) fn compressed(&self) -> serde_json::Value {
        let (own, other): (Vec<&str>, Vec<&str>) = self.columns.iter().copied().unzip();
        let cardinality = match self.cardinality {
            Cardinality::OneToMany => "one-to-many",
            Cardinality::ManyToOne => "many-to-one",
            Cardinality::OneToOne { .. } => "one-to-one",
        };
        serde_json::json!({
            "cardinality": cardinality,
            "embedding": format!("{} with {}", self.table, self.foreign_table),
            "relationship": format!(
                "{} using {}({}) and {}({})",
                self.constraint,
                self.table,
                own.join(", "),
                self.foreign_table,
                other.join(", "),
            ),
        })
    }

    fn single_column(&self) -> Option<&str> {
        match self.columns.as_slice() {
            [(column, _)] => Some(column),
            _ => None,
        }
    }

    fn single_referenced_column(&self) -> Option<&str> {
        match self.columns.as_slice() {
            [(_, referenced)] => Some(referenced),
            _ => None,
        }
    }
}

/// Why no single relationship was chosen.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FindError<'a> {
    /// `NoRelBetween` (`PGRST200`).
    NotFound,
    /// `AmbiguousRelBetween` (`PGRST201`), candidates in upstream order.
    Ambiguous(Vec<Candidate<'a>>),
    /// A shape Megabase does not serve yet: a self-reference, or several
    /// candidates where a many-to-many link could also match.
    Unsupported,
}

/// Every relationship from `schema.relation` to a relation of the same
/// schema, both directions of each key.
fn candidates<'a>(schema: &str, relation: &str, keys: &'a [ForeignKey]) -> Vec<Candidate<'a>> {
    let mut out = Vec::new();
    for key in keys
        .iter()
        .filter(|key| key.schema == schema && key.foreign_schema == schema)
    {
        let is_self = key.table == key.foreign_table;
        if key.table == relation {
            out.push(Candidate {
                table: &key.table,
                foreign_table: &key.foreign_table,
                is_self,
                cardinality: if key.one_to_one {
                    Cardinality::OneToOne { parent: false }
                } else {
                    Cardinality::ManyToOne
                },
                constraint: &key.name,
                columns: key
                    .columns
                    .iter()
                    .map(|(c, f)| (c.as_str(), f.as_str()))
                    .collect(),
                table_is_view: key.table_is_view,
                foreign_is_view: key.foreign_is_view,
            });
        }
        if key.foreign_table == relation {
            out.push(Candidate {
                table: &key.foreign_table,
                foreign_table: &key.table,
                is_self,
                cardinality: if key.one_to_one {
                    Cardinality::OneToOne { parent: true }
                } else {
                    Cardinality::OneToMany
                },
                constraint: &key.name,
                columns: key
                    .columns
                    .iter()
                    .map(|(c, f)| (f.as_str(), c.as_str()))
                    .collect(),
                table_is_view: key.foreign_is_view,
                foreign_is_view: key.table_is_view,
            });
        }
    }
    out
}

/// `findRel`: the relationship `target` (and its optional `!hint`) names from
/// `schema.relation`. A constraint or column target never names a view.
///
/// `junctions` lists the tables that would give a many-to-many link between
/// `relation` and `target` (`addM2MRels`: a key to each, both inside the
/// junction's primary key). This lookup does not build those links, so a
/// request one of them could match is [`FindError::Unsupported`].
///
/// # Errors
///
/// [`FindError::NotFound`] when nothing matches, [`FindError::Ambiguous`]
/// when several column or constraint matches remain, and
/// [`FindError::Unsupported`] for shapes still served as 501.
pub(crate) fn find_relationship<'a>(
    schema: &str,
    relation: &str,
    target: &str,
    hint: Option<&str>,
    keys: &'a [ForeignKey],
    junctions: &[String],
) -> Result<Relationship, FindError<'a>> {
    // A many-to-many link matches by table name with no hint, or by
    // junction name with one.
    let many_to_many = match hint {
        None => !junctions.is_empty(),
        Some(hint) => junctions.iter().any(|junction| junction == hint),
    };
    if many_to_many {
        return Err(FindError::Unsupported);
    }
    let mut by_table = false;
    let mut matched: Vec<Candidate<'a>> = candidates(schema, relation, keys)
        .into_iter()
        .filter(|rel| match hint {
            None => {
                let column = rel.single_column() == Some(target);
                if rel.is_self {
                    // `family_tree?select=children:family_tree(*)` (one-to-many) or
                    // `family_tree?select=parent(*)` (many-to-one by column).
                    (rel.foreign_table == target && rel.cardinality == Cardinality::OneToMany)
                        || (column && rel.cardinality == Cardinality::ManyToOne)
                } else {
                    let table = rel.foreign_table == target;
                    by_table |= table;
                    table || ((rel.constraint == target || column) && !rel.foreign_is_view)
                }
            }
            Some(hint) => {
                let referenced = rel.single_referenced_column() == Some(hint);
                if rel.is_self {
                    // `organizations?select=auditees:organizations!auditor(*)`.
                    rel.foreign_table == target
                        && rel.cardinality == Cardinality::OneToMany
                        && referenced
                } else {
                    // `projects?select=clients!projects_client_id_fkey(*)`,
                    // `clients!client_id(*)` or `clients!id(*)`.
                    let table = rel.foreign_table == target;
                    by_table |= table;
                    table
                        && (rel.constraint == hint
                            || (rel.single_column() == Some(hint) && !rel.foreign_is_view)
                            || referenced)
                }
            }
        })
        .collect();
    matched.sort_by(Candidate::sort_cmp);
    match matched.as_slice() {
        [] => Err(FindError::NotFound),
        [rel] if rel.is_self => Err(FindError::Unsupported),
        [rel] => Ok(Relationship {
            table: rel.foreign_table.to_string(),
            to_many: rel.cardinality == Cardinality::OneToMany,
            root_columns: rel.columns.iter().map(|(r, _)| (*r).to_string()).collect(),
            target_columns: rel.columns.iter().map(|(_, t)| (*t).to_string()).collect(),
            column_types: Vec::new(),
        }),
        // Several view relationships are not listed in upstream's order or
        // wording here, so they stay a 501 rather than a guessed `PGRST201`.
        _ if by_table && matched.iter().any(|r| r.table_is_view || r.foreign_is_view) => {
            Err(FindError::Unsupported)
        }
        _ => Err(FindError::Ambiguous(matched)),
    }
}

/// `relHint`: the `PGRST201` hint listing `table!constraint` choices.
pub(crate) fn ambiguous_hint(target: &str, rels: &[Candidate<'_>]) -> String {
    let choices = rels
        .iter()
        .map(|rel| format!("'{}!{}'", rel.foreign_table, rel.constraint))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Try changing '{target}' to one of the following: {choices}. Find the desired relationship in the 'details' key."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `findRel` with no hint and no many-to-many junction.
    fn find_relationship<'a>(
        schema: &str,
        relation: &str,
        target: &str,
        keys: &'a [ForeignKey],
    ) -> Result<Relationship, FindError<'a>> {
        super::find_relationship(schema, relation, target, None, keys, &[])
    }

    fn fk(name: &str, table: &str, foreign: &str, cols: &[(&str, &str)], one: bool) -> ForeignKey {
        ForeignKey {
            name: name.to_string(),
            schema: "public".to_string(),
            table: table.to_string(),
            foreign_schema: "public".to_string(),
            foreign_table: foreign.to_string(),
            columns: cols
                .iter()
                .map(|(c, f)| ((*c).to_string(), (*f).to_string()))
                .collect(),
            one_to_one: one,
            table_is_view: false,
            foreign_is_view: false,
        }
    }

    fn dep(
        table: &str,
        view: &str,
        constraint: &str,
        kind: KeyDepKind,
        cols: &[(&str, &[&str])],
    ) -> ViewKeyDep {
        ViewKeyDep {
            table_schema: "public".to_string(),
            table: table.to_string(),
            view_schema: "public".to_string(),
            view: view.to_string(),
            constraint: constraint.to_string(),
            kind,
            columns: cols
                .iter()
                .map(|(c, vs)| {
                    (
                        (*c).to_string(),
                        vs.iter().map(|v| (*v).to_string()).collect(),
                    )
                })
                .collect(),
        }
    }

    /// The issue #375 schema: `books_view` selects `books.author_id` and
    /// `authors_view` selects `authors.id` as `author`.
    fn library() -> Vec<ForeignKey> {
        let keys = vec![fk(
            "books_author_id_fkey",
            "books",
            "authors",
            &[("author_id", "id")],
            false,
        )];
        let deps = vec![
            dep(
                "books",
                "books_view",
                "books_author_id_fkey",
                KeyDepKind::Referencing,
                &[("author_id", &["author_id"])],
            ),
            dep(
                "authors",
                "authors_view",
                "books_author_id_fkey",
                KeyDepKind::Referenced,
                &[("id", &["author"])],
            ),
        ];
        add_view_keys(keys, &deps)
    }

    #[test]
    fn view_to_table_is_many_to_one() {
        let keys = library();
        let rel = find_relationship("public", "books_view", "authors", &keys).unwrap();
        assert_eq!(
            rel,
            Relationship {
                table: "authors".to_string(),
                to_many: false,
                root_columns: vec!["author_id".to_string()],
                target_columns: vec!["id".to_string()],
                column_types: Vec::new(),
            }
        );
    }

    #[test]
    fn table_to_view_is_one_to_many() {
        let keys = library();
        let rel = find_relationship("public", "authors", "books_view", &keys).unwrap();
        assert_eq!(
            rel,
            Relationship {
                table: "books_view".to_string(),
                to_many: true,
                root_columns: vec!["id".to_string()],
                target_columns: vec!["author_id".to_string()],
                column_types: Vec::new(),
            }
        );
        let rel = find_relationship("public", "books", "authors_view", &keys).unwrap();
        assert!(!rel.to_many);
        assert_eq!(rel.root_columns, vec!["author_id".to_string()]);
        assert_eq!(rel.target_columns, vec!["author".to_string()]);
    }

    #[test]
    fn view_to_view_maps_both_ends() {
        let keys = library();
        let rel = find_relationship("public", "books_view", "authors_view", &keys).unwrap();
        assert!(!rel.to_many);
        assert_eq!(rel.root_columns, vec!["author_id".to_string()]);
        assert_eq!(rel.target_columns, vec!["author".to_string()]);
        let rel = find_relationship("public", "authors_view", "books_view", &keys).unwrap();
        assert!(rel.to_many);
        assert_eq!(rel.root_columns, vec!["author".to_string()]);
    }

    #[test]
    fn constraint_and_column_targets_never_name_a_view() {
        let keys = library();
        // From the view, the column hint still finds the base table.
        let rel = find_relationship("public", "books_view", "author_id", &keys).unwrap();
        assert_eq!(rel.table, "authors");
        // From the table, the view relationships are skipped: no ambiguity.
        let rel = find_relationship("public", "books", "books_author_id_fkey", &keys).unwrap();
        assert_eq!(rel.table, "authors");
        // From a view, the constraint names the base table, not `books_view`.
        let rel =
            find_relationship("public", "authors_view", "books_author_id_fkey", &keys).unwrap();
        assert_eq!(rel.table, "books");
        assert!(rel.to_many);
    }

    #[test]
    fn view_relationships_stay_in_the_requested_schema() {
        let mut keys = library();
        for key in &mut keys {
            if key.table == "books_view" {
                key.schema = "api".to_string();
            }
        }
        assert_eq!(
            find_relationship("public", "books_view", "authors", &keys),
            Err(FindError::NotFound)
        );
        assert_eq!(
            find_relationship("api", "books_view", "authors", &keys),
            Err(FindError::NotFound)
        );
    }

    #[test]
    fn a_key_column_selected_twice_gives_one_relationship_each() {
        let keys = vec![fk(
            "books_author_id_fkey",
            "books",
            "authors",
            &[("author_id", "id")],
            false,
        )];
        let deps = vec![dep(
            "books",
            "books_view",
            "books_author_id_fkey",
            KeyDepKind::Referencing,
            &[("author_id", &["a1", "a2"])],
        )];
        let keys = add_view_keys(keys, &deps);
        assert_eq!(keys.len(), 3);
        let rel = find_relationship("public", "books_view", "a2", &keys).unwrap();
        assert_eq!(rel.root_columns, vec!["a2".to_string()]);
        // Two relationships named by the table stay unsupported (501).
        assert_eq!(
            find_relationship("public", "books_view", "authors", &keys),
            Err(FindError::Unsupported)
        );
    }

    fn slack() -> Vec<ForeignKey> {
        vec![
            fk(
                "messages_user_id_fkey",
                "messages",
                "users",
                &[("user_id", "id")],
                false,
            ),
            fk(
                "messages_channel_id_fkey",
                "messages",
                "channels",
                &[("channel_id", "id")],
                false,
            ),
            fk(
                "channels_created_by_fkey",
                "channels",
                "users",
                &[("created_by", "id")],
                false,
            ),
        ]
    }

    #[test]
    fn column_hint_resolves_to_the_referenced_table() {
        let keys = slack();
        let rel = find_relationship("public", "messages", "user_id", &keys).unwrap();
        assert_eq!(
            rel,
            Relationship {
                table: "users".to_string(),
                to_many: false,
                root_columns: vec!["user_id".to_string()],
                target_columns: vec!["id".to_string()],
                column_types: Vec::new(),
            }
        );
        let rel = find_relationship("public", "messages", "channel_id", &keys).unwrap();
        assert_eq!(rel.table, "channels");
    }

    #[test]
    fn table_and_constraint_targets_still_resolve() {
        let keys = slack();
        let rel = find_relationship("public", "users", "messages", &keys).unwrap();
        assert!(rel.to_many);
        assert_eq!(rel.root_columns, vec!["id".to_string()]);
        assert_eq!(rel.target_columns, vec!["user_id".to_string()]);
        let rel =
            find_relationship("public", "messages", "messages_channel_id_fkey", &keys).unwrap();
        assert_eq!(rel.table, "channels");
    }

    #[test]
    fn unknown_column_is_not_found() {
        let keys = slack();
        assert_eq!(
            find_relationship("public", "messages", "message", &keys),
            Err(FindError::NotFound)
        );
        // A non-key column of the requested table names no relationship.
        assert_eq!(
            find_relationship("public", "messages", "body", &keys),
            Err(FindError::NotFound)
        );
    }

    #[test]
    fn referenced_column_from_the_parent_is_ambiguous() {
        let keys = slack();
        let Err(FindError::Ambiguous(rels)) = find_relationship("public", "users", "id", &keys)
        else {
            panic!("expected ambiguity");
        };
        assert_eq!(
            rels.iter().map(Candidate::compressed).collect::<Vec<_>>(),
            vec![
                serde_json::json!({
                    "cardinality": "one-to-many",
                    "embedding": "users with channels",
                    "relationship": "channels_created_by_fkey using users(id) and channels(created_by)",
                }),
                serde_json::json!({
                    "cardinality": "one-to-many",
                    "embedding": "users with messages",
                    "relationship": "messages_user_id_fkey using users(id) and messages(user_id)",
                }),
            ]
        );
        assert_eq!(
            ambiguous_hint("id", &rels),
            "Try changing 'id' to one of the following: 'channels!channels_created_by_fkey', 'messages!messages_user_id_fkey'. Find the desired relationship in the 'details' key."
        );
    }

    #[test]
    fn one_to_one_is_a_single_object_both_ways() {
        let keys = vec![fk(
            "profiles_id_fkey",
            "profiles",
            "users",
            &[("id", "id")],
            true,
        )];
        let rel = find_relationship("public", "users", "profiles", &keys).unwrap();
        assert!(!rel.to_many);
        let rel = find_relationship("public", "profiles", "id", &keys).unwrap();
        assert!(!rel.to_many);
        assert_eq!(rel.table, "users");
    }

    #[test]
    fn multi_column_keys_do_not_match_a_column() {
        let keys = vec![fk("a_fkey", "a", "b", &[("x", "x"), ("y", "y")], false)];
        assert_eq!(
            find_relationship("public", "a", "x", &keys),
            Err(FindError::NotFound)
        );
    }

    #[test]
    fn self_reference_and_junctions_are_unsupported() {
        let keys = vec![fk(
            "tree_parent_fkey",
            "tree",
            "tree",
            &[("parent", "id")],
            false,
        )];
        assert_eq!(
            find_relationship("public", "tree", "parent", &keys),
            Err(FindError::Unsupported)
        );
        assert_eq!(
            find_relationship("public", "tree", "tree", &keys),
            Err(FindError::Unsupported)
        );
        assert_eq!(
            hinted("tree", "tree", Some("parent"), &keys, &[]),
            Err(FindError::Unsupported)
        );
        // A junction table turns a table-name target or a junction-name hint
        // into a many-to-many link.
        let keys = posts();
        let junctions = ["post_reviewers".to_string()];
        assert_eq!(
            hinted("posts", "users", None, &keys, &junctions),
            Err(FindError::Unsupported)
        );
        assert_eq!(
            hinted("posts", "users", Some("post_reviewers"), &keys, &junctions),
            Err(FindError::Unsupported)
        );
        let rel = hinted("posts", "users", Some("editor_id"), &keys, &junctions).unwrap();
        assert_eq!(rel.root_columns, vec!["editor_id".to_string()]);
    }

    /// `findRel` with a hint and the junctions of the pair.
    fn hinted<'a>(
        relation: &str,
        target: &str,
        hint: Option<&str>,
        keys: &'a [ForeignKey],
        junctions: &[String],
    ) -> Result<Relationship, FindError<'a>> {
        super::find_relationship("public", relation, target, hint, keys, junctions)
    }

    /// The issue #376 schema: two keys from `posts` to `users`.
    fn posts() -> Vec<ForeignKey> {
        vec![
            fk(
                "posts_author_id_fkey",
                "posts",
                "users",
                &[("author_id", "id")],
                false,
            ),
            fk(
                "posts_editor_id_fkey",
                "posts",
                "users",
                &[("editor_id", "id")],
                false,
            ),
        ]
    }

    #[test]
    fn several_keys_to_one_table_are_ambiguous() {
        let keys = posts();
        let Err(FindError::Ambiguous(rels)) = find_relationship("public", "posts", "users", &keys)
        else {
            panic!("expected ambiguity");
        };
        assert_eq!(
            rels.iter().map(Candidate::compressed).collect::<Vec<_>>(),
            vec![
                serde_json::json!({
                    "cardinality": "many-to-one",
                    "embedding": "posts with users",
                    "relationship": "posts_author_id_fkey using posts(author_id) and users(id)",
                }),
                serde_json::json!({
                    "cardinality": "many-to-one",
                    "embedding": "posts with users",
                    "relationship": "posts_editor_id_fkey using posts(editor_id) and users(id)",
                }),
            ]
        );
        assert_eq!(
            ambiguous_hint("users", &rels),
            "Try changing 'users' to one of the following: 'users!posts_author_id_fkey', 'users!posts_editor_id_fkey'. Find the desired relationship in the 'details' key."
        );
    }

    #[test]
    fn hint_picks_a_key_by_constraint_or_column() {
        let keys = posts();
        let by_constraint =
            hinted("posts", "users", Some("posts_author_id_fkey"), &keys, &[]).unwrap();
        assert_eq!(
            by_constraint,
            Relationship {
                table: "users".to_string(),
                to_many: false,
                root_columns: vec!["author_id".to_string()],
                target_columns: vec!["id".to_string()],
                column_types: Vec::new(),
            }
        );
        let by_column = hinted("posts", "users", Some("editor_id"), &keys, &[]).unwrap();
        assert_eq!(by_column.root_columns, vec!["editor_id".to_string()]);
        // From the referenced side, the hint names the column on `posts`.
        let reverse = hinted("users", "posts", Some("author_id"), &keys, &[]).unwrap();
        assert!(reverse.to_many);
        assert_eq!(reverse.root_columns, vec!["id".to_string()]);
        assert_eq!(reverse.target_columns, vec!["author_id".to_string()]);
    }

    #[test]
    fn hint_on_the_referenced_column_can_stay_ambiguous() {
        let keys = posts();
        let Err(FindError::Ambiguous(rels)) = hinted("posts", "users", Some("id"), &keys, &[])
        else {
            panic!("expected ambiguity");
        };
        assert_eq!(rels.len(), 2);
    }

    #[test]
    fn hint_must_match_the_target_table() {
        let keys = posts();
        // The constraint exists but the hinted target is not its table.
        assert_eq!(
            hinted(
                "posts",
                "comments",
                Some("posts_author_id_fkey"),
                &keys,
                &[]
            ),
            Err(FindError::NotFound)
        );
        assert_eq!(
            hinted("posts", "users", Some("title"), &keys, &[]),
            Err(FindError::NotFound)
        );
        // A hint never lets a constraint or column stand in for the target.
        assert_eq!(
            hinted(
                "posts",
                "author_id",
                Some("posts_author_id_fkey"),
                &keys,
                &[]
            ),
            Err(FindError::NotFound)
        );
    }
}
