//! `ident` and `literal` from pg-format 1.0.4.
//!
//! Postgres Meta builds dynamic SQL with these two functions. The word list
//! is `lib/reserved.js` from that package.

// Ported from pg-format 1.0.4 lib/index.js (`quoteIdent`, `quoteLiteral`) and
// lib/reserved.js (MIT), as used by supabase/postgres-meta (Apache-2.0), pin v0.100.0.

/// Reserved words pg-format refuses to leave unquoted. Uppercase, sorted.
const RESERVED: &[&str] = &[
    "AES128",
    "AES256",
    "ALL",
    "ALLOWOVERWRITE",
    "ANALYSE",
    "ANALYZE",
    "AND",
    "ANY",
    "ARRAY",
    "AS",
    "ASC",
    "AUTHORIZATION",
    "BACKUP",
    "BETWEEN",
    "BINARY",
    "BLANKSASNULL",
    "BOTH",
    "BYTEDICT",
    "CASE",
    "CAST",
    "CHECK",
    "COLLATE",
    "COLUMN",
    "CONSTRAINT",
    "CREATE",
    "CREDENTIALS",
    "CROSS",
    "CURRENT_DATE",
    "CURRENT_TIME",
    "CURRENT_TIMESTAMP",
    "CURRENT_USER",
    "CURRENT_USER_ID",
    "DEFAULT",
    "DEFERRABLE",
    "DEFLATE",
    "DEFRAG",
    "DELTA",
    "DELTA32K",
    "DESC",
    "DISABLE",
    "DISTINCT",
    "DO",
    "ELSE",
    "EMPTYASNULL",
    "ENABLE",
    "ENCODE",
    "ENCRYPT",
    "ENCRYPTION",
    "END",
    "EXCEPT",
    "EXPLICIT",
    "FALSE",
    "FOR",
    "FOREIGN",
    "FREEZE",
    "FROM",
    "FULL",
    "GLOBALDICT256",
    "GLOBALDICT64K",
    "GRANT",
    "GROUP",
    "GZIP",
    "HAVING",
    "IDENTITY",
    "IGNORE",
    "ILIKE",
    "IN",
    "INITIALLY",
    "INNER",
    "INTERSECT",
    "INTO",
    "IS",
    "ISNULL",
    "JOIN",
    "LEADING",
    "LEFT",
    "LIKE",
    "LIMIT",
    "LOCALTIME",
    "LOCALTIMESTAMP",
    "LUN",
    "LUNS",
    "LZO",
    "LZOP",
    "MINUS",
    "MOSTLY13",
    "MOSTLY32",
    "MOSTLY8",
    "NATURAL",
    "NEW",
    "NOT",
    "NOTNULL",
    "NULL",
    "NULLS",
    "OFF",
    "OFFLINE",
    "OFFSET",
    "OLD",
    "ON",
    "ONLY",
    "OPEN",
    "OR",
    "ORDER",
    "OUTER",
    "OVERLAPS",
    "PARALLEL",
    "PARTITION",
    "PERCENT",
    "PLACING",
    "PRIMARY",
    "RAW",
    "READRATIO",
    "RECOVER",
    "REFERENCES",
    "REJECTLOG",
    "RESORT",
    "RESTORE",
    "RIGHT",
    "SELECT",
    "SESSION_USER",
    "SIMILAR",
    "SOME",
    "SYSDATE",
    "SYSTEM",
    "TABLE",
    "TAG",
    "TDES",
    "TEXT255",
    "TEXT32K",
    "THEN",
    "TO",
    "TOP",
    "TRAILING",
    "TRUE",
    "TRUNCATECOLUMNS",
    "UNION",
    "UNIQUE",
    "USER",
    "USING",
    "VERBOSE",
    "WALLET",
    "WHEN",
    "WHERE",
    "WITH",
    "WITHOUT",
];

/// Quote a SQL identifier the way pg-format `ident` does.
pub(crate) fn ident(value: &str) -> String {
    if is_plain_ident(value) && !is_reserved(value) {
        return value.to_string();
    }
    let mut quoted = String::from("\"");
    for ch in value.chars() {
        if ch == '"' {
            quoted.push('"');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    quoted
}

/// Quote a SQL literal the way pg-format `literal` does for a string.
pub(crate) fn literal(value: &str) -> String {
    let mut quoted = String::from("'");
    let mut has_backslash = false;
    for ch in value.chars() {
        if ch == '\'' {
            quoted.push('\'');
        } else if ch == '\\' {
            quoted.push('\\');
            has_backslash = true;
        }
        quoted.push(ch);
    }
    quoted.push('\'');
    if has_backslash {
        quoted.insert(0, 'E');
    }
    quoted
}

/// Quote an integer the way pg-format quotes a JavaScript number: decimal
/// digits inside a string literal.
pub(crate) fn literal_i64(value: i64) -> String {
    literal(&value.to_string())
}

fn is_plain_ident(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch == '$' || ch.is_ascii_lowercase() || ch.is_ascii_digit())
}

fn is_reserved(value: &str) -> bool {
    let upper = value.to_ascii_uppercase();
    RESERVED.binary_search(&upper.as_str()).is_ok()
}

/// `IN ('a','b')`, or empty when `items` is empty.
pub(crate) fn filter_in<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let mut list = String::new();
    for (index, item) in items.into_iter().enumerate() {
        if index > 0 {
            list.push(',');
        }
        list.push_str(&literal(item));
    }
    if list.is_empty() {
        String::new()
    } else {
        format!("IN ({list})")
    }
}

/// `filterByList` from `src/lib/helpers.ts`.
///
/// A non-empty include list wins. Otherwise excluded names, plus
/// `default_exclude` when that slice is used by the caller, become `NOT IN`.
pub(crate) fn filter_by_list(
    include: &[&str],
    exclude: &[&str],
    default_exclude: &[&str],
) -> String {
    if !include.is_empty() {
        return filter_in(include.iter().copied());
    }
    if exclude.is_empty() && default_exclude.is_empty() {
        return String::new();
    }
    let mut list = String::new();
    for (index, item) in default_exclude.iter().chain(exclude.iter()).enumerate() {
        if index > 0 {
            list.push(',');
        }
        list.push_str(&literal(item));
    }
    format!("NOT IN ({list})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_words_are_sorted_and_unique() {
        let mut sorted = RESERVED.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, RESERVED);
    }

    #[test]
    fn ident_leaves_plain_names_and_quotes_the_rest() {
        assert_eq!(ident("todos"), "todos");
        assert_eq!(ident("_ok$1"), "_ok$1");
        assert_eq!(ident("Foo"), "\"Foo\"");
        assert_eq!(ident("user"), "\"user\"");
        assert_eq!(ident("select"), "\"select\"");
        assert_eq!(ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(ident("foo;drop"), "\"foo;drop\"");
        assert_eq!(ident(""), "\"\"");
    }

    #[test]
    fn literal_quotes_strings_numbers_and_backslashes() {
        assert_eq!(literal("public"), "'public'");
        assert_eq!(literal("'"), "''''");
        assert_eq!(literal("\\"), "E'\\\\'");
        assert_eq!(literal("a'\\b"), "E'a''\\\\b'");
        assert_eq!(
            literal("public'); drop table x; --"),
            "'public''); drop table x; --'"
        );
        assert_eq!(literal_i64(10), "'10'");
        assert_eq!(literal_i64(0), "'0'");
        assert_eq!(literal_i64(-1), "'-1'");
    }

    #[test]
    fn filters_match_include_over_exclude() {
        assert_eq!(
            filter_by_list(&["public", "core"], &["pg_catalog"], &[]),
            "IN ('public','core')"
        );
        assert_eq!(
            filter_by_list(&[], &["secret"], &["pg_catalog"]),
            "NOT IN ('pg_catalog','secret')"
        );
        assert_eq!(filter_by_list(&[], &[], &[]), "");
        assert_eq!(filter_in(["public.todos"]), "IN ('public.todos')");
    }
}
