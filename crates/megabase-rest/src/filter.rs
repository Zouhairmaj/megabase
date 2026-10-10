// Ported from postgrest src/library/PostgREST/ApiRequest/QueryParams.hs (MIT), pin v16.4,
// and postgrest src/library/PostgREST/Query/SqlFragment.hs (MIT), pin v16.4.

//! Horizontal filter operators from a PostgREST query value (`eq.1`).
//!
//! Parsing follows `pOpExpr` in `QueryParams.hs`. SQL text follows
//! `pgFmtFilter` in `SqlFragment.hs`: values stay bound parameters, and
//! `*` in `ilike` becomes `%`.

use std::fmt::Write as _;

/// Quantifier inside `eq(any)` / `eq(all)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Quant {
    Any,
    All,
}

/// Operators this issue executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServedOp {
    Eq,
    Gt,
    Gte,
    Ilike,
    Fts,
    Cs,
    Cd,
    Adj,
}

impl ServedOp {
    fn sql(self) -> &'static str {
        match self {
            // megabase:unit rest:filter-operator:eq
            Self::Eq => "=",
            // megabase:unit rest:filter-operator:gt
            Self::Gt => ">",
            // megabase:unit rest:filter-operator:gte
            Self::Gte => ">=",
            // megabase:unit rest:filter-operator:ilike
            Self::Ilike => "ilike",
            // megabase:unit rest:filter-operator:cs
            Self::Cs => "@>",
            // megabase:unit rest:filter-operator:cd
            Self::Cd => "<@",
            // megabase:unit rest:filter-operator:adj
            Self::Adj => "-|-",
            // megabase:unit rest:filter-operator:fts
            Self::Fts => "@@ to_tsquery",
        }
    }
}

/// One filter after a successful parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParsedFilter {
    /// Operator this crate executes.
    Served {
        op: ServedOp,
        quant: Option<Quant>,
        /// `fts(english)` config name. Absent for `fts.terms`.
        language: Option<String>,
        value: String,
    },
    /// Recognized PostgREST operator that a later filtering issue owns.
    Unsupported { unit: &'static str },
}

/// Why a filter value is not a PostgREST operator expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilterParseError {
    pub message: String,
    pub details: String,
}

/// Parse `eq.1`, `not.gt.2`, `fts(english).cats`, `eq(any).{1,2}`.
///
/// # Errors
///
/// Returns a `PGRST100` message when the value is not an operator expression.
/// The column name is not part of this grammar.
pub(crate) fn parse_filter_value(value: &str) -> Result<ParsedFilter, FilterParseError> {
    let (negated, expr) = if let Some(rest) = value.strip_prefix("not.") {
        (true, rest)
    } else {
        (false, value)
    };
    match parse_operation(expr) {
        Some(_) if negated => Ok(ParsedFilter::Unsupported {
            unit: "rest:filter-operator:not",
        }),
        Some(parsed) => Ok(parsed),
        None => Err(parse_error(value)),
    }
}

fn parse_operation(input: &str) -> Option<ParsedFilter> {
    if let Some(parsed) = attempt_prefix(input, "in.", "rest:filter-operator:in") {
        return Some(parsed);
    }
    if let Some(parsed) = attempt_prefix(input, "isdistinct.", "rest:filter-operator:isdistinct") {
        return Some(parsed);
    }
    if let Some(parsed) = attempt_prefix(input, "is.", "rest:filter-operator:is") {
        return Some(parsed);
    }
    if let Some(parsed) = attempt_fts(input, "plfts", "rest:filter-operator:plfts") {
        return Some(parsed);
    }
    if let Some(parsed) = attempt_fts(input, "phfts", "rest:filter-operator:phfts") {
        return Some(parsed);
    }
    if let Some(parsed) = attempt_fts(input, "wfts", "rest:filter-operator:wfts") {
        return Some(parsed);
    }
    if let Some(parsed) = attempt_fts_served(input) {
        return Some(parsed);
    }
    for (name, unit) in [
        ("neq", "rest:filter-operator:neq"),
        ("ov", "rest:filter-operator:ov"),
        ("sl", "rest:filter-operator:sl"),
        ("sr", "rest:filter-operator:sr"),
        ("nxr", "rest:filter-operator:nxr"),
        ("nxl", "rest:filter-operator:nxl"),
    ] {
        if let Some(parsed) = attempt_simple_unsupported(input, name, unit) {
            return Some(parsed);
        }
    }
    if let Some(parsed) = attempt_simple_served(input, "cs", ServedOp::Cs) {
        return Some(parsed);
    }
    if let Some(parsed) = attempt_simple_served(input, "cd", ServedOp::Cd) {
        return Some(parsed);
    }
    if let Some(parsed) = attempt_simple_served(input, "adj", ServedOp::Adj) {
        return Some(parsed);
    }
    for (name, op) in [
        ("ilike", Some(ServedOp::Ilike)),
        ("imatch", None),
        ("like", None),
        ("match", None),
        ("gte", Some(ServedOp::Gte)),
        ("gt", Some(ServedOp::Gt)),
        ("lte", None),
        ("lt", None),
        ("eq", Some(ServedOp::Eq)),
    ] {
        if let Some(parsed) = attempt_quant(input, name, op) {
            return Some(parsed);
        }
    }
    None
}

fn attempt_prefix(input: &str, prefix: &str, unit: &'static str) -> Option<ParsedFilter> {
    input
        .strip_prefix(prefix)
        .map(|_| ParsedFilter::Unsupported { unit })
}

fn attempt_simple_unsupported(input: &str, name: &str, unit: &'static str) -> Option<ParsedFilter> {
    let rest = input.strip_prefix(name)?;
    rest.strip_prefix('.')
        .map(|_| ParsedFilter::Unsupported { unit })
}

fn attempt_simple_served(input: &str, name: &str, op: ServedOp) -> Option<ParsedFilter> {
    let rest = input.strip_prefix(name)?;
    let value = rest.strip_prefix('.')?;
    Some(ParsedFilter::Served {
        op,
        quant: None,
        language: None,
        value: value.to_string(),
    })
}

fn attempt_fts(input: &str, name: &str, unit: &'static str) -> Option<ParsedFilter> {
    let rest = input.strip_prefix(name)?;
    let rest = strip_optional_language(rest)?;
    rest.strip_prefix('.')
        .map(|_| ParsedFilter::Unsupported { unit })
}

fn attempt_fts_served(input: &str) -> Option<ParsedFilter> {
    let rest = input.strip_prefix("fts")?;
    let (language, rest) = split_optional_language(rest)?;
    let value = rest.strip_prefix('.')?;
    Some(ParsedFilter::Served {
        op: ServedOp::Fts,
        quant: None,
        language,
        value: value.to_string(),
    })
}

fn strip_optional_language(input: &str) -> Option<&str> {
    split_optional_language(input).map(|(_, rest)| rest)
}

/// `None` when `(` is present but the config name or the closing `)` is not.
fn split_optional_language(input: &str) -> Option<(Option<String>, &str)> {
    if let Some(rest) = input.strip_prefix('(') {
        let end = rest.find(')')?;
        let lang = &rest[..end];
        if !is_identifier(lang) {
            return None;
        }
        Some((Some(lang.to_string()), &rest[end + 1..]))
    } else {
        Some((None, input))
    }
}

fn attempt_quant(input: &str, name: &str, served: Option<ServedOp>) -> Option<ParsedFilter> {
    let rest = input.strip_prefix(name)?;
    let (quant, rest) = if let Some(rest) = rest.strip_prefix('(') {
        if let Some(rest) = rest.strip_prefix("any)") {
            (Some(Quant::Any), rest)
        } else {
            let rest = rest.strip_prefix("all)")?;
            (Some(Quant::All), rest)
        }
    } else {
        (None, rest)
    };
    let value = rest.strip_prefix('.')?;
    if let Some(op) = served {
        Some(ParsedFilter::Served {
            op,
            quant,
            language: None,
            value: value.to_string(),
        })
    } else {
        let unit = match name {
            "imatch" => "rest:filter-operator:imatch",
            "like" => "rest:filter-operator:like",
            "match" => "rest:filter-operator:match",
            "lte" => "rest:filter-operator:lte",
            "lt" => "rest:filter-operator:lt",
            _ => "rest:filter-operator:eq",
        };
        Some(ParsedFilter::Unsupported { unit })
    }
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if is_ident_char(first) => chars.all(is_ident_char),
        _ => false,
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | ' ' | '$')
}

/// `PGRST100` text for a value that matches no operator.
///
/// Columns follow Parsec's `string` and `char` inside `pOpExpr`
/// (`QueryParams.hs`). A failed operator name is reported at the first
/// character of that name, and a failed `.` is reported on that character.
fn parse_error(input: &str) -> FilterParseError {
    let err = op_expr_error(input);
    let unexpected = if err.unexpected.is_empty() {
        "end of input".to_string()
    } else {
        err.unexpected
    };
    let expecting = join_expecting(&err.expects);
    FilterParseError {
        message: format!(
            "\"failed to parse filter ({input})\" (line {}, column {})",
            err.line, err.col
        ),
        details: format!("unexpected {unexpected} expecting {expecting}"),
    }
}

/// One Parsec error: the furthest `SysUnExpect` and the `Expect` labels.
#[derive(Clone)]
struct PErr {
    line: usize,
    col: usize,
    /// Haskell `show` of the unexpected character, including quotes.
    /// Empty means end of input.
    unexpected: String,
    expects: Vec<String>,
    known: bool,
}

#[derive(Clone, Copy)]
struct At<'a> {
    input: &'a str,
    byte: usize,
    line: usize,
    col: usize,
}

impl<'a> At<'a> {
    fn start(input: &'a str) -> Self {
        Self {
            input,
            byte: 0,
            line: 1,
            col: 1,
        }
    }

    fn rest(self) -> &'a str {
        &self.input[self.byte..]
    }

    fn peek(self) -> Option<char> {
        self.rest().chars().next()
    }

    fn bump(self, ch: char) -> Self {
        let col = match ch {
            '\n' => 1,
            '\t' => self.col + 8 - ((self.col - 1) % 8),
            _ => self.col + 1,
        };
        let line = if ch == '\n' { self.line + 1 } else { self.line };
        Self {
            input: self.input,
            byte: self.byte + ch.len_utf8(),
            line,
            col,
        }
    }
}

fn unknown(at: At<'_>) -> PErr {
    PErr {
        line: at.line,
        col: at.col,
        unexpected: String::new(),
        expects: Vec::new(),
        known: false,
    }
}

fn merge(e1: PErr, e2: PErr) -> PErr {
    if e1.known && !e2.known {
        return e1;
    }
    if !e1.known && e2.known {
        return e2;
    }
    match (e1.line, e1.col).cmp(&(e2.line, e2.col)) {
        std::cmp::Ordering::Greater => e1,
        std::cmp::Ordering::Less => e2,
        std::cmp::Ordering::Equal => {
            let mut expects = e1.expects;
            expects.extend(e2.expects);
            PErr {
                line: e1.line,
                col: e1.col,
                unexpected: e1.unexpected,
                expects,
                known: e1.known || e2.known,
            }
        }
    }
}

fn merge_acc(acc: &mut Option<PErr>, err: PErr) {
    *acc = Some(match acc.take() {
        Some(prev) => merge(prev, err),
        None => err,
    });
}

fn label(mut err: PErr, msg: &str) -> PErr {
    err.expects = vec![msg.to_string()];
    err.known = true;
    err
}

fn show_char(ch: char) -> String {
    let body = match ch {
        '\\' => "\\\\".to_string(),
        '"' => "\\\"".to_string(),
        '\n' => "\\n".to_string(),
        '\r' => "\\r".to_string(),
        '\t' => "\\t".to_string(),
        c => c.to_string(),
    };
    format!("\"{body}\"")
}

fn join_expecting(expects: &[String]) -> String {
    let mut unique = Vec::new();
    for item in expects {
        if item.is_empty() || unique.iter().any(|seen: &String| seen == item) {
            continue;
        }
        unique.push(item.clone());
    }
    if unique.is_empty() {
        return String::new();
    }
    let last = unique.pop().unwrap_or_default();
    if unique.is_empty() {
        return last;
    }
    let mut out = unique.join(", ");
    out.push_str(" or ");
    out.push_str(&last);
    out
}

/// `string` reports the error at the start of the token.
fn parse_string<'a>(at: At<'a>, token: &str) -> Result<At<'a>, PErr> {
    let mut cur = at;
    for expected in token.chars() {
        match cur.peek() {
            Some(got) if got == expected => cur = cur.bump(got),
            Some(got) => {
                return Err(PErr {
                    line: at.line,
                    col: at.col,
                    unexpected: show_char(got),
                    expects: vec![format!("\"{token}\"")],
                    known: true,
                });
            }
            None => {
                return Err(PErr {
                    line: at.line,
                    col: at.col,
                    unexpected: String::new(),
                    expects: vec![format!("\"{token}\"")],
                    known: true,
                });
            }
        }
    }
    Ok(cur)
}

/// `char '.' <?> label`. The error sits on the character that failed.
fn parse_char<'a>(at: At<'a>, expected: char, expect_label: &str) -> Result<At<'a>, PErr> {
    match at.peek() {
        Some(got) if got == expected => Ok(at.bump(got)),
        Some(got) => Err(label(
            PErr {
                line: at.line,
                col: at.col,
                unexpected: show_char(got),
                expects: Vec::new(),
                known: true,
            },
            expect_label,
        )),
        None => Err(label(
            PErr {
                line: at.line,
                col: at.col,
                unexpected: String::new(),
                expects: Vec::new(),
                known: true,
            },
            expect_label,
        )),
    }
}

fn first_string<'a>(at: At<'a>, tokens: &[&str]) -> Result<At<'a>, PErr> {
    let mut acc = None;
    for token in tokens {
        match parse_string(at, token) {
            Ok(next) => return Ok(next),
            Err(err) => merge_acc(&mut acc, err),
        }
    }
    Err(acc.unwrap_or_else(|| unknown(at)))
}

fn op_expr_error(input: &str) -> PErr {
    let start = At::start(input);
    let (at, not_err) = match not_prefix(start) {
        Ok(after) => (after, unknown(after)),
        Err(err) => (start, err),
    };
    match p_operation(at) {
        Ok(()) => not_err,
        Err(op_err) => merge(not_err, op_err),
    }
}

fn not_prefix(at: At<'_>) -> Result<At<'_>, PErr> {
    let after = parse_string(at, "not")?;
    parse_char(after, '.', "delimiter (.)")
}

fn p_operation(at: At<'_>) -> Result<(), PErr> {
    let mut acc = None;
    for name in ["in", "is", "isdistinct"] {
        match word_then_dot(at, name) {
            Ok(()) => return Ok(()),
            Err(err) => merge_acc(&mut acc, err),
        }
    }
    for attempt in [p_fts(at), p_simple(at), p_quant(at)] {
        match attempt {
            Ok(()) => return Ok(()),
            Err(err) => merge_acc(&mut acc, err),
        }
    }
    Err(label(
        acc.unwrap_or_else(|| unknown(at)),
        "operator (eq, gt, ...)",
    ))
}

fn word_then_dot(at: At<'_>, name: &str) -> Result<(), PErr> {
    let after = parse_string(at, name)?;
    parse_char(after, '.', "delimiter (.)").map(|_| ())
}

fn p_simple(at: At<'_>) -> Result<(), PErr> {
    const OPS: &[&str] = &["neq", "cs", "cd", "ov", "sl", "sr", "nxr", "nxl", "adj"];
    let after = first_string(at, OPS)?;
    parse_char(after, '.', "delimiter (.)").map(|_| ())
}

fn p_quant(at: At<'_>) -> Result<(), PErr> {
    const OPS: &[&str] = &[
        "eq", "gte", "gt", "lte", "lt", "like", "ilike", "match", "imatch",
    ];
    let after = first_string(at, OPS)?;
    let (at_delim, quant_err) = optional_any_all(after);
    match parse_char(at_delim, '.', "delimiter (.)") {
        Ok(_) => Ok(()),
        Err(delim_err) => Err(merge(quant_err, delim_err)),
    }
}

fn p_fts(at: At<'_>) -> Result<(), PErr> {
    const OPS: &[&str] = &["fts", "plfts", "phfts", "wfts"];
    let after = first_string(at, OPS)?;
    let (at_delim, lang_err) = optional_language(after);
    match parse_char(at_delim, '.', "delimiter (.)") {
        Ok(_) => Ok(()),
        Err(delim_err) => Err(merge(lang_err, delim_err)),
    }
}

/// `optionMaybe (try (between '(' ')' (try (string "any") <|> string "all")))`.
fn optional_any_all(at: At<'_>) -> (At<'_>, PErr) {
    let after_open = match parse_char(at, '(', "\"(\"") {
        Ok(next) => next,
        Err(err) => return (at, err),
    };
    let after_name = match parse_string(after_open, "any") {
        Ok(next) => next,
        Err(any_err) => match parse_string(after_open, "all") {
            Ok(next) => next,
            Err(all_err) => return (at, merge(any_err, all_err)),
        },
    };
    match parse_char(after_name, ')', "\")\"") {
        Ok(next) => (next, unknown(next)),
        Err(err) => (at, err),
    }
}

/// `optionMaybe (try (between '(' ')' pIdentifier))`.
fn optional_language(at: At<'_>) -> (At<'_>, PErr) {
    let after_open = match parse_char(at, '(', "\"(\"") {
        Ok(next) => next,
        Err(err) => return (at, err),
    };
    let mut cur = after_open;
    let mut saw_ident = false;
    while let Some(ch) = cur.peek() {
        if !is_ident_char(ch) {
            break;
        }
        saw_ident = true;
        cur = cur.bump(ch);
    }
    if !saw_ident {
        let unexpected = cur.peek().map(show_char).unwrap_or_default();
        return (
            at,
            PErr {
                line: cur.line,
                col: cur.col,
                unexpected,
                expects: Vec::new(),
                known: true,
            },
        );
    }
    match parse_char(cur, ')', "\")\"") {
        Ok(next) => (next, unknown(next)),
        Err(err) => (at, err),
    }
}

/// WHERE predicates for served filters. Values are `$n` parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PredicateSql {
    pub sql: String,
    pub params: Vec<String>,
}

/// One served filter. `pg_type` is `format_type` when the column exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoundFilter {
    pub column: String,
    pub op: ServedOp,
    pub quant: Option<Quant>,
    pub language: Option<String>,
    pub value: String,
    pub pg_type: Option<String>,
}

/// Why a catalog type cannot be interpolated into a cast.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("column type is not a safe cast target")]
pub(crate) struct UnsafeType;

/// Build `relation.col op ($n::text)::type` predicates joined with `AND`.
///
/// # Errors
///
/// Returns [`UnsafeType`] when `pg_type` is not a `format_type` result this
/// crate is willing to splice into SQL. The value itself is never spliced.
///
/// The driver binds text. The cast restores the coercion an `unknown` literal
/// would get. A missing column has no cast so PostgreSQL can report `42703`.
pub(crate) fn predicate_sql(
    relation: &str,
    filters: &[BoundFilter],
) -> Result<PredicateSql, UnsafeType> {
    let mut sql = String::new();
    let mut params = Vec::new();
    for (index, filter) in filters.iter().enumerate() {
        if index > 0 {
            sql.push_str(" AND ");
        }
        push_predicate(&mut sql, &mut params, relation, filter)?;
    }
    Ok(PredicateSql { sql, params })
}

fn push_predicate(
    sql: &mut String,
    params: &mut Vec<String>,
    relation: &str,
    filter: &BoundFilter,
) -> Result<(), UnsafeType> {
    sql.push_str(&quote_ident(relation));
    sql.push('.');
    sql.push_str(&quote_ident(&filter.column));
    sql.push(' ');
    if filter.op == ServedOp::Fts {
        sql.push_str(ServedOp::Fts.sql());
        sql.push('(');
        if let Some(language) = &filter.language {
            push_text_cast(sql, params, language, "regconfig");
            sql.push_str(", ");
        }
        push_text_cast(sql, params, &filter.value, "text");
        sql.push(')');
        return Ok(());
    }
    sql.push_str(filter.op.sql());
    sql.push(' ');
    let ilike_value;
    let value = if filter.op == ServedOp::Ilike {
        ilike_value = filter.value.replace('*', "%");
        ilike_value.as_str()
    } else {
        filter.value.as_str()
    };
    let Some(pg_type) = filter.pg_type.as_deref() else {
        push_param(sql, params, value);
        return Ok(());
    };
    let cast = match filter.quant {
        Some(quant) => {
            match quant {
                // megabase:unit rest:filter-operator:any
                Quant::Any => sql.push_str("ANY ("),
                // megabase:unit rest:filter-operator:all
                Quant::All => sql.push_str("ALL ("),
            }
            array_cast(pg_type)?
        }
        None => cast_target(pg_type)?.to_string(),
    };
    push_text_cast(sql, params, value, &cast);
    if filter.quant.is_some() {
        sql.push(')');
    }
    Ok(())
}

fn push_param(sql: &mut String, params: &mut Vec<String>, value: &str) {
    params.push(value.to_string());
    let index = params.len();
    let _ = write!(sql, "${index}");
}

fn push_text_cast(sql: &mut String, params: &mut Vec<String>, value: &str, cast: &str) {
    params.push(value.to_string());
    let index = params.len();
    let _ = write!(sql, "(${index}::text)::{cast}");
}

fn array_cast(pg_type: &str) -> Result<String, UnsafeType> {
    let base = cast_target(pg_type)?;
    Ok(format!("{base}[]"))
}

fn cast_target(pg_type: &str) -> Result<&str, UnsafeType> {
    if pg_type.is_empty()
        || !pg_type.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, ' ' | '_' | '[' | ']' | ',' | '"' | '(' | ')' | '.')
        })
    {
        return Err(UnsafeType);
    }
    Ok(pg_type)
}

/// Quote a PostgreSQL identifier. Null bytes are dropped, quotes are doubled.
pub(crate) fn quote_ident(name: &str) -> String {
    let mut out = String::from("\"");
    for ch in name.chars() {
        if ch == '\0' {
            continue;
        }
        if ch == '"' {
            out.push('"');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn served(value: &str) -> ParsedFilter {
        parse_filter_value(value).unwrap()
    }

    #[test]
    fn parses_served_operators() {
        assert_eq!(
            served("eq.true"),
            ParsedFilter::Served {
                op: ServedOp::Eq,
                quant: None,
                language: None,
                value: "true".into(),
            }
        );
        assert_eq!(
            served("gt.1"),
            ParsedFilter::Served {
                op: ServedOp::Gt,
                quant: None,
                language: None,
                value: "1".into(),
            }
        );
        assert_eq!(
            served("gte.1"),
            ParsedFilter::Served {
                op: ServedOp::Gte,
                quant: None,
                language: None,
                value: "1".into(),
            }
        );
        assert_eq!(
            served("ilike.*spec*"),
            ParsedFilter::Served {
                op: ServedOp::Ilike,
                quant: None,
                language: None,
                value: "*spec*".into(),
            }
        );
        assert_eq!(
            served("cs.{1,2}"),
            ParsedFilter::Served {
                op: ServedOp::Cs,
                quant: None,
                language: None,
                value: "{1,2}".into(),
            }
        );
        assert_eq!(
            served("cd.{a,b}"),
            ParsedFilter::Served {
                op: ServedOp::Cd,
                quant: None,
                language: None,
                value: "{a,b}".into(),
            }
        );
        assert_eq!(
            served("adj.[1,4)"),
            ParsedFilter::Served {
                op: ServedOp::Adj,
                quant: None,
                language: None,
                value: "[1,4)".into(),
            }
        );
        assert_eq!(
            served("fts.cats"),
            ParsedFilter::Served {
                op: ServedOp::Fts,
                quant: None,
                language: None,
                value: "cats".into(),
            }
        );
        assert_eq!(
            served("fts(english).cats"),
            ParsedFilter::Served {
                op: ServedOp::Fts,
                quant: None,
                language: Some("english".into()),
                value: "cats".into(),
            }
        );
        assert_eq!(
            served("eq(any).{1,2}"),
            ParsedFilter::Served {
                op: ServedOp::Eq,
                quant: Some(Quant::Any),
                language: None,
                value: "{1,2}".into(),
            }
        );
        assert_eq!(
            served("gt(all).{3,4}"),
            ParsedFilter::Served {
                op: ServedOp::Gt,
                quant: Some(Quant::All),
                language: None,
                value: "{3,4}".into(),
            }
        );
        assert_eq!(
            served("eq."),
            ParsedFilter::Served {
                op: ServedOp::Eq,
                quant: None,
                language: None,
                value: String::new(),
            }
        );
    }

    #[test]
    fn leaves_later_operators_unsupported() {
        assert_eq!(
            served("lt.3"),
            ParsedFilter::Unsupported {
                unit: "rest:filter-operator:lt"
            }
        );
        assert_eq!(
            served("not.eq.1"),
            ParsedFilter::Unsupported {
                unit: "rest:filter-operator:not"
            }
        );
        assert_eq!(
            served("in.(1,3)"),
            ParsedFilter::Unsupported {
                unit: "rest:filter-operator:in"
            }
        );
        assert_eq!(
            served("is.null"),
            ParsedFilter::Unsupported {
                unit: "rest:filter-operator:is"
            }
        );
        assert_eq!(
            served("like.*a*"),
            ParsedFilter::Unsupported {
                unit: "rest:filter-operator:like"
            }
        );
        assert!(parse_filter_value("nope.1").is_err());
        assert!(parse_filter_value("0").is_err());
        assert!(parse_filter_value("fts().x").is_err());
        assert!(parse_filter_value("eq(foo).1").is_err());
    }

    #[test]
    fn unknown_operator_message_matches_postgrest_shape() {
        let cases = [
            (
                "0",
                1,
                "unexpected \"0\" expecting \"not\" or operator (eq, gt, ...)",
            ),
            (
                "nope.1",
                1,
                "unexpected \"p\" expecting \"not\" or operator (eq, gt, ...)",
            ),
            (
                "noop.0",
                1,
                "unexpected \"o\" expecting \"not\" or operator (eq, gt, ...)",
            ),
            (
                "abcd.1",
                1,
                "unexpected \"a\" expecting \"not\" or operator (eq, gt, ...)",
            ),
            (
                "gX",
                1,
                "unexpected \"g\" expecting \"not\" or operator (eq, gt, ...)",
            ),
            (
                "nX",
                1,
                "unexpected \"X\" expecting \"not\" or operator (eq, gt, ...)",
            ),
            (
                "ilikX",
                1,
                "unexpected \"i\" expecting \"not\" or operator (eq, gt, ...)",
            ),
            (
                "gtX.1",
                3,
                "unexpected \"X\" expecting operator (eq, gt, ...)",
            ),
            (
                "ilikeX.1",
                6,
                "unexpected \"X\" expecting operator (eq, gt, ...)",
            ),
            (
                "gteX",
                4,
                "unexpected \"X\" expecting operator (eq, gt, ...)",
            ),
            (
                "likeX",
                5,
                "unexpected \"X\" expecting operator (eq, gt, ...)",
            ),
            (
                "neqX",
                4,
                "unexpected \"X\" expecting operator (eq, gt, ...)",
            ),
            (
                "inX",
                3,
                "unexpected \"X\" expecting operator (eq, gt, ...)",
            ),
            (
                "isX",
                3,
                "unexpected \"X\" expecting operator (eq, gt, ...)",
            ),
            (
                "csX",
                3,
                "unexpected \"X\" expecting operator (eq, gt, ...)",
            ),
            (
                "adjX",
                4,
                "unexpected \"X\" expecting operator (eq, gt, ...)",
            ),
            (
                "ftsX",
                4,
                "unexpected \"X\" expecting operator (eq, gt, ...)",
            ),
            (
                "wftsX",
                5,
                "unexpected \"X\" expecting operator (eq, gt, ...)",
            ),
            (
                "isdX",
                3,
                "unexpected \"d\" expecting operator (eq, gt, ...)",
            ),
            (
                "eq",
                3,
                "unexpected end of input expecting operator (eq, gt, ...)",
            ),
            (
                "fts().x",
                5,
                "unexpected \")\" expecting operator (eq, gt, ...)",
            ),
            (
                "eq().value",
                4,
                "unexpected \")\" expecting operator (eq, gt, ...)",
            ),
            (
                "eq(foo).1",
                4,
                "unexpected \"f\" expecting operator (eq, gt, ...)",
            ),
            (
                "is().value",
                3,
                "unexpected \"(\" expecting operator (eq, gt, ...)",
            ),
            (
                "not.nope",
                5,
                "unexpected \"n\" expecting operator (eq, gt, ...)",
            ),
            (
                "not.",
                5,
                "unexpected end of input expecting operator (eq, gt, ...)",
            ),
            (
                "not.eq",
                7,
                "unexpected end of input expecting operator (eq, gt, ...)",
            ),
            ("notX", 4, "unexpected \"X\" expecting delimiter (.)"),
        ];
        for (input, column, details) in cases {
            let err = parse_filter_value(input).unwrap_err();
            assert_eq!(
                err.message,
                format!("\"failed to parse filter ({input})\" (line 1, column {column})"),
                "{input}"
            );
            assert_eq!(err.details, details, "{input}");
        }
    }

    #[test]
    fn predicate_binds_values_and_casts_catalog_types() {
        let rendered = predicate_sql(
            "todos",
            &[
                BoundFilter {
                    column: "done".into(),
                    op: ServedOp::Eq,
                    quant: None,
                    language: None,
                    value: "true".into(),
                    pg_type: Some("boolean".into()),
                },
                BoundFilter {
                    column: "title".into(),
                    op: ServedOp::Ilike,
                    quant: None,
                    language: None,
                    value: "*spec*".into(),
                    pg_type: Some("text".into()),
                },
                BoundFilter {
                    column: "id".into(),
                    op: ServedOp::Eq,
                    quant: Some(Quant::Any),
                    language: None,
                    value: "{1,2}".into(),
                    pg_type: Some("bigint".into()),
                },
                BoundFilter {
                    column: "body".into(),
                    op: ServedOp::Fts,
                    quant: None,
                    language: Some("english".into()),
                    value: "cats".into(),
                    pg_type: Some("tsvector".into()),
                },
            ],
        )
        .unwrap();
        assert_eq!(
            rendered.sql,
            "\"todos\".\"done\" = ($1::text)::boolean AND \"todos\".\"title\" ilike ($2::text)::text AND \"todos\".\"id\" = ANY (($3::text)::bigint[]) AND \"todos\".\"body\" @@ to_tsquery(($4::text)::regconfig, ($5::text)::text)"
        );
        assert_eq!(
            rendered.params,
            vec!["true", "%spec%", "{1,2}", "english", "cats"]
        );
        assert!(!rendered.sql.contains("drop"));
    }

    #[test]
    fn predicate_does_not_splice_the_value() {
        let payload = "1'; DROP TABLE todos;--";
        let rendered = predicate_sql(
            "todos",
            &[BoundFilter {
                column: "id\".\"x".into(),
                op: ServedOp::Eq,
                quant: None,
                language: None,
                value: payload.into(),
                pg_type: Some("bigint".into()),
            }],
        )
        .unwrap();
        assert!(!rendered.sql.contains("DROP"));
        assert!(rendered
            .sql
            .starts_with("\"todos\".\"id\"\".\"\"x\" = ($1::text)::bigint"));
        assert_eq!(rendered.params, vec![payload]);
    }

    #[test]
    fn rejects_unsafe_cast_targets() {
        let err = predicate_sql(
            "todos",
            &[BoundFilter {
                column: "id".into(),
                op: ServedOp::Gt,
                quant: None,
                language: None,
                value: "1".into(),
                pg_type: Some("int;drop".into()),
            }],
        );
        assert!(err.is_err());
    }

    #[test]
    fn quote_ident_doubles_quotes_and_drops_nulls() {
        assert_eq!(quote_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_ident("a\0b"), "\"ab\"");
    }
}
