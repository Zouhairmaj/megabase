//! Query-string split for PostgREST filters.
//!
//! `parse` in `QueryParams.hs` percent-decodes, treats `+` as a space, and
//! drops parameters with no `=`. `select`, `columns`, and `on_conflict` are
//! reserved names. `order`, `limit`, `offset`, `and`, and `or` match the last
//! dot-separated word so embedded resources can carry them.

use crate::filter::{parse_filter_value, ParsedFilter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawFilter {
    pub column: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClassifiedQuery {
    /// First reserved parameter, in query order.
    pub reserved: Option<&'static str>,
    /// A filter key is an embedded resource or a JSON path.
    pub embed: bool,
    pub filters: Vec<RawFilter>,
}

/// Split `query` (no leading `?`) into reserved parameters and column filters.
#[must_use]
pub(crate) fn classify_query(query: &str) -> ClassifiedQuery {
    let mut reserved = None;
    let mut embed = false;
    let mut filters = Vec::new();
    if query.is_empty() {
        return ClassifiedQuery {
            reserved,
            embed,
            filters,
        };
    }
    for part in query.split('&') {
        if part.is_empty() {
            continue;
        }
        let Some((raw_key, raw_value)) = part.split_once('=') else {
            continue;
        };
        let key = percent_decode(raw_key);
        let value = percent_decode(raw_value);
        if reserved.is_none() {
            if let Some(unit) = reserved_unit(&key) {
                reserved = Some(unit);
                continue;
            }
        } else if reserved_unit(&key).is_some() {
            continue;
        }
        if key.is_empty() || key.contains('.') || key.contains("->") {
            embed = true;
            continue;
        }
        filters.push(RawFilter { column: key, value });
    }
    ClassifiedQuery {
        reserved,
        embed,
        filters,
    }
}

fn reserved_unit(key: &str) -> Option<&'static str> {
    match key {
        "select" => Some("rest:query-param:select"),
        "columns" => Some("rest:query-param:columns"),
        "on_conflict" => Some("rest:query-param:on_conflict"),
        _ => {
            let last = key.rsplit('.').next().unwrap_or(key);
            match last {
                "order" => Some("rest:query-param:order"),
                "limit" => Some("rest:query-param:limit"),
                "offset" => Some("rest:query-param:offset"),
                "and" => Some("rest:query-param:and"),
                "or" => Some("rest:query-param:or"),
                _ => None,
            }
        }
    }
}

/// Decode a query component the way `parseQueryReplacePlus` does.
pub(crate) fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &bytes[index + 1..index + 3];
                match std::str::from_utf8(hex)
                    .ok()
                    .and_then(|text| u8::from_str_radix(text, 16).ok())
                {
                    Some(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    None => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Operator outcomes for one query string.
///
/// Panic-free on arbitrary input. The `rest_query` fuzzer calls this.
#[must_use]
pub fn interpret_query(query: &str) -> Vec<&'static str> {
    classify_query(query)
        .filters
        .iter()
        .map(|filter| match parse_filter_value(&filter.value) {
            Ok(ParsedFilter::Served { op, quant, .. }) => match (op, quant) {
                (_, Some(crate::filter::Quant::Any)) => "any",
                (_, Some(crate::filter::Quant::All)) => "all",
                (crate::filter::ServedOp::Eq, None) => "eq",
                (crate::filter::ServedOp::Gt, None) => "gt",
                (crate::filter::ServedOp::Gte, None) => "gte",
                (crate::filter::ServedOp::Ilike, None) => "ilike",
                (crate::filter::ServedOp::Fts, None) => "fts",
                (crate::filter::ServedOp::Cs, None) => "cs",
                (crate::filter::ServedOp::Cd, None) => "cd",
                (crate::filter::ServedOp::Adj, None) => "adj",
            },
            Ok(ParsedFilter::Unsupported { unit }) => unit,
            Err(_) => "invalid",
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_filters_and_reserved_params() {
        let query = classify_query("done=eq.true&select=id&priority=gt.1&order=id");
        assert_eq!(query.reserved, Some("rest:query-param:select"));
        assert!(!query.embed);
        assert_eq!(
            query.filters,
            vec![
                RawFilter {
                    column: "done".into(),
                    value: "eq.true".into(),
                },
                RawFilter {
                    column: "priority".into(),
                    value: "gt.1".into(),
                },
            ]
        );
    }

    #[test]
    fn decodes_plus_and_percent() {
        let query = classify_query("title=ilike.a%2Bb+c");
        assert_eq!(query.filters[0].value, "ilike.a+b c");
    }

    #[test]
    fn drops_bare_keys_and_marks_embeds() {
        let query = classify_query("or&project.name=eq.1&tags->0=cs.{a}");
        assert!(query.embed);
        assert!(query.filters.is_empty());
        assert_eq!(query.reserved, None);
    }

    #[test]
    fn interpret_query_names_served_operators() {
        assert_eq!(
            interpret_query("id=eq(any).{1,2}&title=ilike.*a*&id=nope.1"),
            vec!["any", "ilike", "invalid"]
        );
    }
}
