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

/// Decode a query component the way `parseQueryReplacePlus True` does.
pub(crate) fn percent_decode(raw: &str) -> String {
    decode_component(raw, true)
}

/// Decode one path segment the way `urlDecode False` does.
///
/// `+` stays `+`. Only two ASCII hex digits after `%` are an escape.
pub(crate) fn path_decode(raw: &str) -> String {
    decode_component(raw, false)
}

fn decode_component(raw: &str, plus_is_space: bool) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' if plus_is_space => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len()
                && bytes[index + 1].is_ascii_hexdigit()
                && bytes[index + 2].is_ascii_hexdigit() =>
            {
                let byte = (hex_value(bytes[index + 1]) << 4) | hex_value(bytes[index + 2]);
                out.push(byte);
                index += 3;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn filter_name(parsed: &ParsedFilter) -> &'static str {
    let (negated, name) = match parsed {
        ParsedFilter::Served {
            negated, op, quant, ..
        } => {
            let name = match (op, quant) {
                (_, Some(crate::filter::Quant::Any)) => "any",
                (_, Some(crate::filter::Quant::All)) => "all",
                (crate::filter::ServedOp::Eq, None) => "eq",
                (crate::filter::ServedOp::Neq, None) => "neq",
                (crate::filter::ServedOp::Gt, None) => "gt",
                (crate::filter::ServedOp::Gte, None) => "gte",
                (crate::filter::ServedOp::Lt, None) => "lt",
                (crate::filter::ServedOp::Lte, None) => "lte",
                (crate::filter::ServedOp::Like, None) => "like",
                (crate::filter::ServedOp::Ilike, None) => "ilike",
                (crate::filter::ServedOp::Match, None) => "match",
                (crate::filter::ServedOp::Imatch, None) => "imatch",
                (crate::filter::ServedOp::Fts, None) => "fts",
                (crate::filter::ServedOp::Plfts, None) => "plfts",
                (crate::filter::ServedOp::Phfts, None) => "phfts",
                (crate::filter::ServedOp::Wfts, None) => "wfts",
                (crate::filter::ServedOp::Cs, None) => "cs",
                (crate::filter::ServedOp::Cd, None) => "cd",
                (crate::filter::ServedOp::Ov, None) => "ov",
                (crate::filter::ServedOp::Sl, None) => "sl",
                (crate::filter::ServedOp::Sr, None) => "sr",
                (crate::filter::ServedOp::Nxr, None) => "nxr",
                (crate::filter::ServedOp::Nxl, None) => "nxl",
                (crate::filter::ServedOp::Adj, None) => "adj",
            };
            (*negated, name)
        }
        ParsedFilter::In { negated, .. } => (*negated, "in"),
        ParsedFilter::Is { negated, .. } => (*negated, "is"),
        ParsedFilter::IsDistinct { negated, .. } => (*negated, "isdistinct"),
    };
    if negated {
        "not"
    } else {
        name
    }
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => 0,
    }
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
            Ok(parsed) => filter_name(&parsed),
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
    fn percent_escape_requires_two_hex_digits() {
        // `from_str_radix` would accept the `+` as a sign. http-types does not.
        assert_eq!(percent_decode("%+1"), "% 1");
        assert_eq!(percent_decode("%+F"), "% F");
        assert_eq!(percent_decode("%2F"), "/");
        assert_eq!(percent_decode("%GG"), "%GG");
        assert_eq!(path_decode("a+b"), "a+b");
        assert_eq!(path_decode("%+1"), "%+1");
        assert_eq!(path_decode("a%20b"), "a b");
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
            interpret_query("id=eq(any).{1,2}&title=ilike.*a*&id=nope.1&id=not.lt.3&id=in.(1,2)&during=ov.[1,4)&title=plfts(english).spec&during=nxl.[4,7)"),
            vec!["any", "ilike", "invalid", "not", "in", "ov", "plfts", "nxl"]
        );
    }
}
