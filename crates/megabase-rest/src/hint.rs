// Ported from PostgREST src/library/PostgREST/Error.hs `noRelBetweenHint` (MIT), pin from vendor.toml.
// The fuzzy matcher follows the Haskell `fuzzyset` package PostgREST uses
// (gram sizes 2 to 3, cosine score, Levenshtein rescoring, minimum score 0.33).
//! `Perhaps you meant ...` hints for `PGRST200` (no relationship found).

use std::collections::HashMap;

const GRAM_LOWER: usize = 2;
const GRAM_UPPER: usize = 3;
const MIN_SCORE: f64 = 0.33;
const LEVENSHTEIN_CANDIDATES: usize = 50;

/// Lowercase and keep only letters, digits, spaces and commas.
fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == ',')
        .flat_map(char::to_lowercase)
        .collect()
}

/// Gram counts of `value` at one gram size.
fn grams(value: &str, size: usize) -> HashMap<String, f64> {
    let mut chars: Vec<char> = format!("-{}-", normalize(value)).chars().collect();
    while chars.len() < size {
        chars.push('-');
    }
    let mut counts = HashMap::new();
    for window in chars.windows(size) {
        *counts
            .entry(window.iter().collect::<String>())
            .or_insert(0.0) += 1.0;
    }
    counts
}

fn norm(counts: &HashMap<String, f64>) -> f64 {
    counts.values().map(|n| n * n).sum::<f64>().sqrt()
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut current = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current.push(
                (previous[j] + cost)
                    .min(previous[j + 1] + 1)
                    .min(current[j] + 1),
            );
        }
        previous = current;
    }
    previous[b.len()]
}

/// Candidates for `query` with a score of at least 0.33, best first.
fn fuzzy_get<'a>(set: &[&'a str], query: &str) -> Vec<(f64, &'a str)> {
    // `fuzzyset` answers an exact (normalized) match alone, with score 1.
    if let Some(item) = set.iter().find(|item| normalize(item) == normalize(query)) {
        return vec![(1.0, *item)];
    }
    for size in (GRAM_LOWER..=GRAM_UPPER).rev() {
        let query_grams = grams(query, size);
        let query_norm = norm(&query_grams);
        let mut scored: Vec<(f64, &str)> = set
            .iter()
            .filter_map(|item| {
                let item_grams = grams(item, size);
                let dot: f64 = query_grams
                    .iter()
                    .filter_map(|(gram, n)| item_grams.get(gram).map(|m| n * m))
                    .sum();
                (dot > 0.0).then(|| (dot / (query_norm * norm(&item_grams)), *item))
            })
            .collect();
        if scored.is_empty() {
            continue;
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored.truncate(LEVENSHTEIN_CANDIDATES);
        let query_chars: Vec<char> = normalize(query).chars().collect();
        for entry in &mut scored {
            let item_chars: Vec<char> = normalize(entry.1).chars().collect();
            let longest = query_chars.len().max(item_chars.len()).max(1);
            entry.0 = 1.0 - levenshtein(&query_chars, &item_chars) as f64 / longest as f64;
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored.retain(|(score, _)| *score >= MIN_SCORE);
        return scored;
    }
    Vec::new()
}

/// Suggest a table name when no foreign key links `parent` and `child`.
///
/// `edges` holds every `(table, table)` pair joined by a foreign key in the
/// schema, in either direction. An unknown parent gets a parent suggestion; a
/// known parent gets a suggestion among its related tables. Both known tables
/// (an exact match) yield no hint.
#[must_use]
pub(crate) fn no_rel_between_hint(
    parent: &str,
    child: &str,
    edges: &[(String, String)],
) -> Option<String> {
    let mut parents: Vec<&str> = Vec::new();
    for (a, b) in edges {
        for name in [a.as_str(), b.as_str()] {
            if !parents.contains(&name) {
                parents.push(name);
            }
        }
    }
    if parents.contains(&parent) {
        let mut children: Vec<&str> = Vec::new();
        for (a, b) in edges {
            let other = if a == parent {
                b
            } else if b == parent {
                a
            } else {
                continue;
            };
            if !children.contains(&other.as_str()) {
                children.push(other);
            }
        }
        let (_, suggestion) = fuzzy_get(&children, child)
            .into_iter()
            .find(|(score, _)| *score < 1.0)?;
        Some(format!(
            "Perhaps you meant '{suggestion}' instead of '{child}'."
        ))
    } else {
        let (_, suggestion) = fuzzy_get(&parents, parent).into_iter().next()?;
        Some(format!(
            "Perhaps you meant '{suggestion}' instead of '{parent}'."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edges() -> Vec<(String, String)> {
        ["directors", "roles", "actors"]
            .iter()
            .map(|t| ("films".to_string(), (*t).to_string()))
            .collect()
    }

    #[test]
    fn suggests_parent() {
        assert_eq!(
            no_rel_between_hint("film", "directors", &edges()).as_deref(),
            Some("Perhaps you meant 'films' instead of 'film'.")
        );
    }

    #[test]
    fn suggests_child() {
        assert_eq!(
            no_rel_between_hint("films", "role", &edges()).as_deref(),
            Some("Perhaps you meant 'roles' instead of 'role'.")
        );
    }

    #[test]
    fn no_hint_when_both_exist_or_nothing_close() {
        assert_eq!(no_rel_between_hint("films", "actors", &edges()), None);
        assert_eq!(
            no_rel_between_hint("noclosealternative", "roles", &edges()),
            None
        );
        assert_eq!(
            no_rel_between_hint("films", "noclosealternative", &edges()),
            None
        );
    }

    #[test]
    fn unknown_parent_picks_closest_related_table() {
        let edges = vec![("note_comments".to_string(), "notes".to_string())];
        assert_eq!(
            no_rel_between_hint("todos", "notes", &edges).as_deref(),
            Some("Perhaps you meant 'notes' instead of 'todos'.")
        );
    }
}
