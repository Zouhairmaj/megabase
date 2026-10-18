// Ported from datalanche/node-pg-format lib/index.js and lib/reserved.js (MIT),
// as used by supabase/postgres-meta (Apache-2.0), pin v0.100.0.
//
// `ident` and `literal` quote the way pg-format 1.0.4 does. Postgres Meta
// builds policy SQL with those two functions and does not bind parameters.

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

/// `pg-format` `ident` for a string.
pub(crate) fn quote_ident(value: &str) -> String {
    if is_plain_ident(value) && !is_reserved(value) {
        return value.to_string();
    }
    let mut quoted = String::from("\"");
    for c in value.chars() {
        if c == '"' {
            quoted.push('"');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

/// `pg-format` `literal` for a string.
pub(crate) fn quote_literal(value: &str) -> String {
    let mut has_backslash = false;
    let mut quoted = String::from("'");
    for c in value.chars() {
        if c == '\'' || c == '\\' {
            if c == '\\' {
                has_backslash = true;
            }
            quoted.push(c);
        }
        quoted.push(c);
    }
    quoted.push('\'');
    if has_backslash {
        quoted.insert(0, 'E');
    }
    quoted
}

fn is_plain_ident(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c == '$' || c.is_ascii_lowercase() || c.is_ascii_digit())
}

fn is_reserved(value: &str) -> bool {
    let upper = value.to_uppercase();
    RESERVED.binary_search(&upper.as_str()).is_ok()
}
