// Ported from PostgREST src/library/PostgREST/Plan.hs `findRel` and
// src/library/PostgREST/Error.hs `compressedRel` / `relHint` (MIT), pin v16.4.
//! Pick the relationship an embed target names (`findRel`).
//!
//! The target is a table name, a foreign key constraint name, or the single
//! column of a foreign key on the requested table (a column hint, for example
//! `messages?select=author:user_id(*)`). A `!hint` after a table target
//! picks among its keys by constraint name, key column, or referenced column
//! (`posts?select=users!editor_id(*)`). Candidates are the table-to-table
//! foreign keys of one schema, in both directions, as `allM2OandO2ORels` and
//! `addInverseRels` in `SchemaCache.hs` list them.

use std::cmp::Ordering;

use crate::params::Relationship;

/// One foreign key constraint with an end on the requested table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForeignKey {
    /// Constraint name.
    pub name: String,
    /// Table that holds the key.
    pub table: String,
    /// Referenced table.
    pub foreign_table: String,
    /// `(column, referenced column)` pairs in key order.
    pub columns: Vec<(String, String)>,
    /// The key columns are a primary key or unique constraint of `table`.
    pub one_to_one: bool,
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

    /// `matchFKRefSingleCol`: the one column on the foreign table.
    fn single_referenced_column(&self) -> Option<&str> {
        match self.columns.as_slice() {
            [(_, column)] => Some(column),
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
    /// A shape Megabase does not serve yet: a self-reference, or a request
    /// a many-to-many link could match.
    Unsupported,
}

/// Every relationship from `relation`, both directions of each key.
fn candidates<'a>(relation: &str, keys: &'a [ForeignKey]) -> Vec<Candidate<'a>> {
    let mut out = Vec::new();
    for key in keys {
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
            });
        }
    }
    out
}

/// `findRel`: the relationship `target` (and its optional `!hint`) names
/// from `relation`.
///
/// `junctions` lists the tables that would give a many-to-many link between
/// `relation` and `target` (`addM2MRels`: a key to each, both inside the
/// junction's primary key). This lookup does not build those links, so a
/// request one of them could match is [`FindError::Unsupported`].
///
/// # Errors
///
/// [`FindError::NotFound`] when nothing matches, [`FindError::Ambiguous`]
/// when several relationships remain, and [`FindError::Unsupported`] for
/// shapes still served as 501.
pub(crate) fn find_relationship<'a>(
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
    let mut matched: Vec<Candidate<'a>> = candidates(relation, keys)
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
                    // `projects?select=clients(*)`, `projects_client_id_fkey(*)`
                    // or `client_id(*)`.
                    rel.foreign_table == target || rel.constraint == target || column
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
                    rel.foreign_table == target
                        && (rel.constraint == hint
                            || rel.single_column() == Some(hint)
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
        }),
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

    fn fk(name: &str, table: &str, foreign: &str, cols: &[(&str, &str)], one: bool) -> ForeignKey {
        ForeignKey {
            name: name.to_string(),
            table: table.to_string(),
            foreign_table: foreign.to_string(),
            columns: cols
                .iter()
                .map(|(c, f)| ((*c).to_string(), (*f).to_string()))
                .collect(),
            one_to_one: one,
        }
    }

    /// `findRel` with no hint and no many-to-many junction.
    fn find<'a>(
        relation: &str,
        target: &str,
        keys: &'a [ForeignKey],
    ) -> Result<Relationship, FindError<'a>> {
        find_relationship(relation, target, None, keys, &[])
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
        let rel = find("messages", "user_id", &keys).unwrap();
        assert_eq!(
            rel,
            Relationship {
                table: "users".to_string(),
                to_many: false,
                root_columns: vec!["user_id".to_string()],
                target_columns: vec!["id".to_string()],
            }
        );
        let rel = find("messages", "channel_id", &keys).unwrap();
        assert_eq!(rel.table, "channels");
    }

    #[test]
    fn table_and_constraint_targets_still_resolve() {
        let keys = slack();
        let rel = find("users", "messages", &keys).unwrap();
        assert!(rel.to_many);
        assert_eq!(rel.root_columns, vec!["id".to_string()]);
        assert_eq!(rel.target_columns, vec!["user_id".to_string()]);
        let rel = find("messages", "messages_channel_id_fkey", &keys).unwrap();
        assert_eq!(rel.table, "channels");
    }

    #[test]
    fn unknown_column_is_not_found() {
        let keys = slack();
        assert_eq!(find("messages", "message", &keys), Err(FindError::NotFound));
        // A non-key column of the requested table names no relationship.
        assert_eq!(find("messages", "body", &keys), Err(FindError::NotFound));
    }

    #[test]
    fn referenced_column_from_the_parent_is_ambiguous() {
        let keys = slack();
        let Err(FindError::Ambiguous(rels)) = find("users", "id", &keys) else {
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
        let rel = find("users", "profiles", &keys).unwrap();
        assert!(!rel.to_many);
        let rel = find("profiles", "id", &keys).unwrap();
        assert!(!rel.to_many);
        assert_eq!(rel.table, "users");
    }

    #[test]
    fn multi_column_keys_do_not_match_a_column() {
        let keys = vec![fk("a_fkey", "a", "b", &[("x", "x"), ("y", "y")], false)];
        assert_eq!(find("a", "x", &keys), Err(FindError::NotFound));
    }

    #[test]
    fn self_reference_and_many_to_many_are_unsupported() {
        let keys = vec![fk(
            "tree_parent_fkey",
            "tree",
            "tree",
            &[("parent", "id")],
            false,
        )];
        assert_eq!(find("tree", "parent", &keys), Err(FindError::Unsupported));
        assert_eq!(find("tree", "tree", &keys), Err(FindError::Unsupported));
        assert_eq!(
            find_relationship("tree", "tree", Some("parent"), &keys, &[]),
            Err(FindError::Unsupported)
        );
        // A junction table turns a table-name target or a junction-name hint
        // into a many-to-many link.
        let keys = posts();
        let junctions = ["post_reviewers".to_string()];
        assert_eq!(
            find_relationship("posts", "users", None, &keys, &junctions),
            Err(FindError::Unsupported)
        );
        assert_eq!(
            find_relationship("posts", "users", Some("post_reviewers"), &keys, &junctions),
            Err(FindError::Unsupported)
        );
        let rel =
            find_relationship("posts", "users", Some("editor_id"), &keys, &junctions).unwrap();
        assert_eq!(rel.root_columns, vec!["editor_id".to_string()]);
    }

    #[test]
    fn several_keys_to_one_table_are_ambiguous() {
        let keys = posts();
        let Err(FindError::Ambiguous(rels)) = find("posts", "users", &keys) else {
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
            find_relationship("posts", "users", Some("posts_author_id_fkey"), &keys, &[]).unwrap();
        assert_eq!(
            by_constraint,
            Relationship {
                table: "users".to_string(),
                to_many: false,
                root_columns: vec!["author_id".to_string()],
                target_columns: vec!["id".to_string()],
            }
        );
        let by_column = find_relationship("posts", "users", Some("editor_id"), &keys, &[]).unwrap();
        assert_eq!(by_column.root_columns, vec!["editor_id".to_string()]);
        // From the referenced side, the hint names the column on `posts`.
        let reverse = find_relationship("users", "posts", Some("author_id"), &keys, &[]).unwrap();
        assert!(reverse.to_many);
        assert_eq!(reverse.root_columns, vec!["id".to_string()]);
        assert_eq!(reverse.target_columns, vec!["author_id".to_string()]);
    }

    #[test]
    fn hint_on_the_referenced_column_can_stay_ambiguous() {
        let keys = posts();
        let Err(FindError::Ambiguous(rels)) =
            find_relationship("posts", "users", Some("id"), &keys, &[])
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
            find_relationship(
                "posts",
                "comments",
                Some("posts_author_id_fkey"),
                &keys,
                &[]
            ),
            Err(FindError::NotFound)
        );
        assert_eq!(
            find_relationship("posts", "users", Some("title"), &keys, &[]),
            Err(FindError::NotFound)
        );
        // A hint never lets a constraint or column stand in for the target.
        assert_eq!(
            find_relationship(
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
