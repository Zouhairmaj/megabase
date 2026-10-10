// Ported from postgrest src/library/PostgREST/ApiRequest/QueryParams.hs (MIT), pin v16.4,
// and postgrest src/library/PostgREST/Query/SqlFragment.hs (MIT), pin v16.4.

//! Horizontal filter operators from a PostgREST query value (`eq.1`).
//!
//! Parsing follows `pOpExpr` in `QueryParams.hs`. SQL text follows
//! `pgFmtFilter` in `SqlFragment.hs`: values stay bound parameters, `*` in
//! `like` and `ilike` becomes `%`, and `in` builds a bound array literal.

use std::fmt::Write as _;

/// Quantifier inside `eq(any)` / `eq(all)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Quant {
    Any,
    All,
}

/// Operators whose SQL is `column op value`, plus full-text `fts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServedOp {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
    Like,
    Ilike,
    Match,
    Imatch,
    Fts,
    Plfts,
    Phfts,
    Wfts,
    Cs,
    Cd,
    Ov,
    Sl,
    Sr,
    Nxr,
    Nxl,
    Adj,
}

impl ServedOp {
    fn sql(self) -> &'static str {
        match self {
            // megabase:unit rest:filter-operator:eq
            Self::Eq => "=",
            // megabase:unit rest:filter-operator:neq
            Self::Neq => "<>",
            // megabase:unit rest:filter-operator:gt
            Self::Gt => ">",
            // megabase:unit rest:filter-operator:gte
            Self::Gte => ">=",
            // megabase:unit rest:filter-operator:lt
            Self::Lt => "<",
            // megabase:unit rest:filter-operator:lte
            Self::Lte => "<=",
            // megabase:unit rest:filter-operator:like
            Self::Like => "like",
            // megabase:unit rest:filter-operator:ilike
            Self::Ilike => "ilike",
            // megabase:unit rest:filter-operator:match
            Self::Match => "~",
            // megabase:unit rest:filter-operator:imatch
            Self::Imatch => "~*",
            // megabase:unit rest:filter-operator:cs
            Self::Cs => "@>",
            // megabase:unit rest:filter-operator:cd
            Self::Cd => "<@",
            // megabase:unit rest:filter-operator:ov
            Self::Ov => "&&",
            // megabase:unit rest:filter-operator:sl
            Self::Sl => "<<",
            // megabase:unit rest:filter-operator:sr
            Self::Sr => ">>",
            // megabase:unit rest:filter-operator:nxr
            Self::Nxr => "&<",
            // megabase:unit rest:filter-operator:nxl
            Self::Nxl => "&>",
            // megabase:unit rest:filter-operator:adj
            Self::Adj => "-|-",
            // megabase:unit rest:filter-operator:fts
            Self::Fts => "@@ to_tsquery",
            // megabase:unit rest:filter-operator:plfts
            Self::Plfts => "@@ plainto_tsquery",
            // megabase:unit rest:filter-operator:phfts
            Self::Phfts => "@@ phraseto_tsquery",
            // megabase:unit rest:filter-operator:wfts
            Self::Wfts => "@@ websearch_to_tsquery",
        }
    }

    /// `like` and `ilike` map every `*` to `%`. Other operators keep `*`.
    fn stars_are_wildcards(self) -> bool {
        matches!(self, Self::Like | Self::Ilike)
    }

    /// Full-text operators bind a `tsquery` function, not a column cast.
    fn is_fts(self) -> bool {
        matches!(self, Self::Fts | Self::Plfts | Self::Phfts | Self::Wfts)
    }
}

/// `is.null` / `is.true` / `is.unknown` after the case-insensitive match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IsVal {
    Null,
    NotNull,
    True,
    False,
    Unknown,
}

impl IsVal {
    fn sql(self) -> &'static str {
        match self {
            // megabase:unit rest:filter-operator:is
            Self::Null => "NULL",
            Self::NotNull => "NOT NULL",
            Self::True => "TRUE",
            Self::False => "FALSE",
            Self::Unknown => "UNKNOWN",
        }
    }
}

/// `in.(...)`. A single empty element is `in.()` and becomes `= ANY('{}')`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InList {
    Empty,
    Values(Vec<String>),
}

/// One filter after a successful parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParsedFilter {
    /// Comparison, pattern, or full-text operator this crate executes.
    Served {
        /// `not.` prefix. The inner operator is still [`ServedOp`].
        negated: bool,
        op: ServedOp,
        quant: Option<Quant>,
        /// `fts(english)` config name. Absent for `fts.terms`.
        language: Option<String>,
        value: String,
    },
    /// `in.(1,2)` list. Owned by `rest:filter-operator:in`.
    In { negated: bool, values: InList },
    /// `is.null` and the other tri-state keywords.
    Is { negated: bool, value: IsVal },
    /// `isdistinct.value`.
    IsDistinct { negated: bool, value: String },
}

/// Why a filter value is not a PostgREST operator expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilterParseError {
    pub message: String,
    pub details: String,
}

/// Parse `eq.1`, `not.gt.2`, `fts(english).cats`, `eq(any).{1,2}`, `in.(1,2)`.
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
        Some(parsed) => Ok(apply_negation(parsed, negated)),
        None => Err(parse_error(value)),
    }
}

fn apply_negation(filter: ParsedFilter, negated: bool) -> ParsedFilter {
    if !negated {
        return filter;
    }
    match filter {
        ParsedFilter::Served {
            op,
            quant,
            language,
            value,
            ..
        } => ParsedFilter::Served {
            negated: true,
            op,
            quant,
            language,
            value,
        },
        ParsedFilter::In { values, .. } => ParsedFilter::In {
            negated: true,
            values,
        },
        ParsedFilter::Is { value, .. } => ParsedFilter::Is {
            negated: true,
            value,
        },
        ParsedFilter::IsDistinct { value, .. } => ParsedFilter::IsDistinct {
            negated: true,
            value,
        },
    }
}

/// `Hit` is a parsed operator. `Fail` consumed a committed prefix (`in.`,
/// `is.`) whose value is not valid, so later operators must not run.
enum Attempt {
    Miss,
    Hit(ParsedFilter),
    Fail,
}

fn parse_operation(input: &str) -> Option<ParsedFilter> {
    match attempt_in(input) {
        Attempt::Hit(parsed) => return Some(parsed),
        Attempt::Fail => return None,
        Attempt::Miss => {}
    }
    match attempt_is(input) {
        Attempt::Hit(parsed) => return Some(parsed),
        Attempt::Fail => return None,
        Attempt::Miss => {}
    }
    if let Some(parsed) = attempt_isdistinct(input) {
        return Some(parsed);
    }
    // `pFts` then `pSimpleOp` in `QueryParams.hs`. `fts` is not a prefix of
    // `plfts` / `phfts` / `wfts`, and none of the simple names is a prefix
    // of another, so the first match is the only match.
    for (name, op) in [
        ("fts", ServedOp::Fts),
        ("plfts", ServedOp::Plfts),
        ("phfts", ServedOp::Phfts),
        ("wfts", ServedOp::Wfts),
    ] {
        if let Some(parsed) = attempt_fts(input, name, op) {
            return Some(parsed);
        }
    }
    for (name, op) in [
        ("neq", ServedOp::Neq),
        ("cs", ServedOp::Cs),
        ("cd", ServedOp::Cd),
        ("ov", ServedOp::Ov),
        ("sl", ServedOp::Sl),
        ("sr", ServedOp::Sr),
        ("nxr", ServedOp::Nxr),
        ("nxl", ServedOp::Nxl),
        ("adj", ServedOp::Adj),
    ] {
        if let Some(parsed) = attempt_simple_served(input, name, op) {
            return Some(parsed);
        }
    }
    for (name, op) in [
        ("ilike", ServedOp::Ilike),
        ("imatch", ServedOp::Imatch),
        ("like", ServedOp::Like),
        ("match", ServedOp::Match),
        ("gte", ServedOp::Gte),
        ("gt", ServedOp::Gt),
        ("lte", ServedOp::Lte),
        ("lt", ServedOp::Lt),
        ("eq", ServedOp::Eq),
    ] {
        if let Some(parsed) = attempt_quant(input, name, op) {
            return Some(parsed);
        }
    }
    None
}

fn attempt_in(input: &str) -> Attempt {
    let Some(rest) = input.strip_prefix("in") else {
        return Attempt::Miss;
    };
    let Some(rest) = rest.strip_prefix('.') else {
        return Attempt::Miss;
    };
    match parse_in_list(rest) {
        Ok(values) => Attempt::Hit(ParsedFilter::In {
            negated: false,
            values,
        }),
        Err(_) => Attempt::Fail,
    }
}

fn attempt_is(input: &str) -> Attempt {
    let Some(rest) = input.strip_prefix("is") else {
        return Attempt::Miss;
    };
    let Some(rest) = rest.strip_prefix('.') else {
        return Attempt::Miss;
    };
    match parse_is_val(rest) {
        Some(value) => Attempt::Hit(ParsedFilter::Is {
            negated: false,
            value,
        }),
        None => Attempt::Fail,
    }
}

fn attempt_isdistinct(input: &str) -> Option<ParsedFilter> {
    let rest = input.strip_prefix("isdistinct")?;
    let value = rest.strip_prefix('.')?;
    Some(ParsedFilter::IsDistinct {
        negated: false,
        value: value.to_string(),
    })
}

fn attempt_simple_served(input: &str, name: &str, op: ServedOp) -> Option<ParsedFilter> {
    let rest = input.strip_prefix(name)?;
    let value = rest.strip_prefix('.')?;
    Some(ParsedFilter::Served {
        negated: false,
        op,
        quant: None,
        language: None,
        value: value.to_string(),
    })
}

fn attempt_fts(input: &str, name: &str, op: ServedOp) -> Option<ParsedFilter> {
    let rest = input.strip_prefix(name)?;
    let (language, rest) = split_optional_language(rest)?;
    let value = rest.strip_prefix('.')?;
    Some(ParsedFilter::Served {
        negated: false,
        op,
        quant: None,
        language,
        value: value.to_string(),
    })
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

fn attempt_quant(input: &str, name: &str, op: ServedOp) -> Option<ParsedFilter> {
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
    Some(ParsedFilter::Served {
        negated: false,
        op,
        quant,
        language: None,
        value: value.to_string(),
    })
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
    match classify_in(at) {
        Control::Success => return Ok(()),
        Control::Hard(err) => return Err(err),
        Control::Soft(err) => merge_acc(&mut acc, err),
    }
    match classify_is(at) {
        Control::Success => return Ok(()),
        Control::Hard(err) => return Err(err),
        Control::Soft(err) => merge_acc(&mut acc, err),
    }
    match word_then_dot(at, "isdistinct") {
        Ok(()) => return Ok(()),
        Err(err) => merge_acc(&mut acc, err),
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

/// A soft error is still inside `try` and the outer operator label applies.
/// A hard error consumed `in.` or `is.` and keeps its own expectation.
enum Control {
    Success,
    Soft(PErr),
    Hard(PErr),
}

fn classify_in(at: At<'_>) -> Control {
    let after = match parse_string(at, "in") {
        Ok(next) => next,
        Err(err) => return Control::Soft(err),
    };
    let after_dot = match parse_char(after, '.', "delimiter (.)") {
        Ok(next) => next,
        Err(err) => return Control::Soft(err),
    };
    match parse_in_list(after_dot.rest()) {
        Ok(_) => Control::Success,
        Err(fail) => {
            let at_fail = shift(after_dot, fail.at);
            let unexpected = fail.unexpected.map(show_char).unwrap_or_default();
            Control::Hard(PErr {
                line: at_fail.line,
                col: at_fail.col,
                unexpected,
                expects: vec![fail.expecting.to_string()],
                known: true,
            })
        }
    }
}

fn classify_is(at: At<'_>) -> Control {
    let after = match parse_string(at, "is") {
        Ok(next) => next,
        Err(err) => return Control::Soft(err),
    };
    let after_dot = match parse_char(after, '.', "delimiter (.)") {
        Ok(next) => next,
        Err(err) => return Control::Soft(err),
    };
    if parse_is_val(after_dot.rest()).is_some() {
        Control::Success
    } else {
        Control::Hard(is_val_error(after_dot))
    }
}

fn shift(at: At<'_>, bytes: usize) -> At<'_> {
    let target = at.byte + bytes;
    let mut cur = at;
    while cur.byte < target {
        let Some(ch) = cur.peek() else {
            break;
        };
        cur = cur.bump(ch);
    }
    cur
}

fn is_val_error(at: At<'_>) -> PErr {
    const WORDS: &[&str] = &["null", "not_null", "true", "false", "unknown"];
    let mut best = None;
    for word in WORDS {
        merge_acc(&mut best, ci_word_error(at, word));
    }
    label(
        best.unwrap_or_else(|| unknown(at)),
        "isVal: (null, not_null, true, false, unknown)",
    )
}

/// Furthest mismatch of one `ciString`. The error sits on the failing character.
fn ci_word_error(at: At<'_>, word: &str) -> PErr {
    let mut cur = at;
    for expected in word.chars() {
        match cur.peek() {
            Some(got) if got.eq_ignore_ascii_case(&expected) => cur = cur.bump(got),
            Some(got) => {
                return PErr {
                    line: cur.line,
                    col: cur.col,
                    unexpected: show_char(got),
                    expects: Vec::new(),
                    known: true,
                };
            }
            None => {
                return PErr {
                    line: cur.line,
                    col: cur.col,
                    unexpected: String::new(),
                    expects: Vec::new(),
                    known: true,
                };
            }
        }
    }
    unknown(cur)
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
    pub negated: bool,
    pub body: FilterBody,
    pub pg_type: Option<String>,
}

/// SQL shape of one served filter, without the column or the `NOT` prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FilterBody {
    /// Comparison, pattern, range, or `fts`.
    Op {
        op: ServedOp,
        quant: Option<Quant>,
        language: Option<String>,
        value: String,
    },
    /// `in.(...)`.
    In(InList),
    /// `is.null` and the other keywords.
    Is(IsVal),
    /// `isdistinct.value`.
    IsDistinct(String),
}

struct ListFail {
    at: usize,
    unexpected: Option<char>,
    expecting: &'static str,
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
    if filter.negated {
        // megabase:unit rest:filter-operator:not
        sql.push_str("NOT ");
    }
    push_field(sql, params, relation, filter);
    match &filter.body {
        FilterBody::Op {
            op,
            quant,
            language,
            value,
        } => push_op(
            sql,
            params,
            filter.pg_type.as_deref(),
            *op,
            *quant,
            language,
            value,
        ),
        FilterBody::In(values) => {
            // megabase:unit rest:filter-operator:in
            push_in(sql, params, filter.pg_type.as_deref(), values)
        }
        FilterBody::Is(value) => {
            sql.push_str(" IS ");
            sql.push_str(value.sql());
            Ok(())
        }
        FilterBody::IsDistinct(value) => {
            // megabase:unit rest:filter-operator:isdistinct
            sql.push_str(" IS DISTINCT FROM ");
            match filter.pg_type.as_deref() {
                Some(pg_type) => {
                    let cast = cast_target(pg_type)?;
                    push_text_cast(sql, params, value, cast);
                }
                None => push_param(sql, params, value),
            }
            Ok(())
        }
    }
}

/// Column reference, or `to_tsvector` for a full-text filter on a non-`tsvector` column.
///
/// PostgREST wraps every full-text field except a `tsvector` (`Plan.hs`
/// `cfBaseType = "tsvector"`). A missing catalog type is not `tsvector`, so it
/// is wrapped too. The optional language is the same `regconfig` as the query.
fn push_field(sql: &mut String, params: &mut Vec<String>, relation: &str, filter: &BoundFilter) {
    let language = match &filter.body {
        FilterBody::Op { op, language, .. }
            if op.is_fts() && filter.pg_type.as_deref() != Some("tsvector") =>
        {
            Some(language)
        }
        _ => None,
    };
    let Some(language) = language else {
        sql.push_str(&quote_ident(relation));
        sql.push('.');
        sql.push_str(&quote_ident(&filter.column));
        return;
    };
    sql.push_str("to_tsvector(");
    if let Some(language) = language {
        push_text_cast(sql, params, language, "regconfig");
        sql.push_str(", ");
    }
    sql.push_str(&quote_ident(relation));
    sql.push('.');
    sql.push_str(&quote_ident(&filter.column));
    sql.push(')');
}

fn push_op(
    sql: &mut String,
    params: &mut Vec<String>,
    pg_type: Option<&str>,
    op: ServedOp,
    quant: Option<Quant>,
    language: &Option<String>,
    value: &str,
) -> Result<(), UnsafeType> {
    sql.push(' ');
    if op.is_fts() {
        sql.push_str(op.sql());
        sql.push('(');
        if let Some(language) = language {
            push_text_cast(sql, params, language, "regconfig");
            sql.push_str(", ");
        }
        push_text_cast(sql, params, value, "text");
        sql.push(')');
        return Ok(());
    }
    sql.push_str(op.sql());
    sql.push(' ');
    let starred;
    let value = if op.stars_are_wildcards() {
        starred = value.replace('*', "%");
        starred.as_str()
    } else {
        value
    };
    let Some(pg_type) = pg_type else {
        push_param(sql, params, value);
        return Ok(());
    };
    let cast = match quant {
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
    if quant.is_some() {
        sql.push(')');
    }
    Ok(())
}

fn push_in(
    sql: &mut String,
    params: &mut Vec<String>,
    pg_type: Option<&str>,
    values: &InList,
) -> Result<(), UnsafeType> {
    match values {
        InList::Empty => {
            sql.push_str(" = ANY('{}')");
            Ok(())
        }
        InList::Values(items) => {
            let literal = pg_build_array_literal(items);
            sql.push_str(" = ANY (");
            if let Some(pg_type) = pg_type {
                let cast = array_cast(pg_type)?;
                push_text_cast(sql, params, &literal, &cast);
            } else {
                push_param(sql, params, &literal);
            }
            sql.push(')');
            Ok(())
        }
    }
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

/// `is.null` matches a keyword prefix, case-insensitively, with leftover text ignored.
fn parse_is_val(input: &str) -> Option<IsVal> {
    const WORDS: &[(&str, IsVal)] = &[
        ("null", IsVal::Null),
        ("not_null", IsVal::NotNull),
        ("true", IsVal::True),
        ("false", IsVal::False),
        ("unknown", IsVal::Unknown),
    ];
    for (word, value) in WORDS {
        if starts_with_ci(input, word) {
            return Some(*value);
        }
    }
    None
}

fn starts_with_ci(input: &str, word: &str) -> bool {
    let mut chars = input.chars();
    for expected in word.chars() {
        match chars.next() {
            Some(got) if got.eq_ignore_ascii_case(&expected) => {}
            _ => return false,
        }
    }
    true
}

/// `pListVal` from `QueryParams.hs`. Whitespace is space and tab only.
fn parse_in_list(input: &str) -> Result<InList, ListFail> {
    let mut scan = Scan { input, byte: 0 };
    scan.skip_ws();
    if !scan.eat('(') {
        return Err(scan.fail("\"(\""));
    }
    scan.skip_ws();
    let mut items = Vec::new();
    loop {
        items.push(scan.element());
        if scan.eat(',') {
            continue;
        }
        scan.skip_ws();
        if scan.eat(')') {
            return Ok(in_list(items));
        }
        return Err(scan.fail("\")\""));
    }
}

fn in_list(items: Vec<String>) -> InList {
    if items.len() == 1 && items[0].is_empty() {
        InList::Empty
    } else {
        InList::Values(items)
    }
}

struct Scan<'a> {
    input: &'a str,
    byte: usize,
}

impl<'a> Scan<'a> {
    fn rest(&self) -> &'a str {
        &self.input[self.byte..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn bump(&mut self) {
        if let Some(ch) = self.peek() {
            self.byte += ch.len_utf8();
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.bump();
        }
    }

    fn eat(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn fail(&self, expecting: &'static str) -> ListFail {
        ListFail {
            at: self.byte,
            unexpected: self.peek(),
            expecting,
        }
    }

    fn element(&mut self) -> String {
        if self.peek() == Some('"') {
            let saved = self.byte;
            if let Some(value) = self.quoted() {
                return value;
            }
            self.byte = saved;
        }
        let start = self.byte;
        while let Some(ch) = self.peek() {
            if ch == ',' || ch == ')' {
                break;
            }
            self.bump();
        }
        self.input[start..self.byte].to_string()
    }

    /// Quoted element, or `None` when the `try` would backtrack.
    fn quoted(&mut self) -> Option<String> {
        if !self.eat('"') {
            return None;
        }
        let mut out = String::new();
        loop {
            match self.peek() {
                Some('\\') => {
                    self.bump();
                    let ch = self.peek()?;
                    out.push(ch);
                    self.bump();
                }
                Some('"') => {
                    self.bump();
                    match self.peek() {
                        None | Some(',' | ')') => return Some(out),
                        _ => return None,
                    }
                }
                Some(ch) => {
                    out.push(ch);
                    self.bump();
                }
                None => return None,
            }
        }
    }
}

/// PostgREST `pgBuildArrayLiteral`. The result is a bound parameter, not SQL text.
fn pg_build_array_literal(values: &[String]) -> String {
    let mut out = String::from("{");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let trimmed = match value.find('\0') {
            Some(end) => &value[..end],
            None => value.as_str(),
        };
        let slashed = trimmed.replace('\\', "\\\\");
        let escaped = slashed.replace('"', "\\\"");
        out.push('"');
        out.push_str(&escaped);
        out.push('"');
    }
    out.push('}');
    out
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
                negated: false,
                op: ServedOp::Eq,
                quant: None,
                language: None,
                value: "true".into(),
            }
        );
        assert_eq!(
            served("gt.1"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Gt,
                quant: None,
                language: None,
                value: "1".into(),
            }
        );
        assert_eq!(
            served("gte.1"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Gte,
                quant: None,
                language: None,
                value: "1".into(),
            }
        );
        assert_eq!(
            served("ilike.*spec*"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Ilike,
                quant: None,
                language: None,
                value: "*spec*".into(),
            }
        );
        assert_eq!(
            served("cs.{1,2}"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Cs,
                quant: None,
                language: None,
                value: "{1,2}".into(),
            }
        );
        assert_eq!(
            served("cd.{a,b}"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Cd,
                quant: None,
                language: None,
                value: "{a,b}".into(),
            }
        );
        assert_eq!(
            served("adj.[1,4)"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Adj,
                quant: None,
                language: None,
                value: "[1,4)".into(),
            }
        );
        assert_eq!(
            served("fts.cats"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Fts,
                quant: None,
                language: None,
                value: "cats".into(),
            }
        );
        assert_eq!(
            served("fts(english).cats"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Fts,
                quant: None,
                language: Some("english".into()),
                value: "cats".into(),
            }
        );
        assert_eq!(
            served("eq(any).{1,2}"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Eq,
                quant: Some(Quant::Any),
                language: None,
                value: "{1,2}".into(),
            }
        );
        assert_eq!(
            served("gt(all).{3,4}"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Gt,
                quant: Some(Quant::All),
                language: None,
                value: "{3,4}".into(),
            }
        );
        assert_eq!(
            served("eq."),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Eq,
                quant: None,
                language: None,
                value: String::new(),
            }
        );
    }

    #[test]
    fn parses_issue_28_operators() {
        assert_eq!(
            served("neq.1"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Neq,
                quant: None,
                language: None,
                value: "1".into(),
            }
        );
        assert_eq!(
            served("lt.3"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Lt,
                quant: None,
                language: None,
                value: "3".into(),
            }
        );
        assert_eq!(
            served("lte.3"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Lte,
                quant: None,
                language: None,
                value: "3".into(),
            }
        );
        assert_eq!(
            served("like.*a*"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Like,
                quant: None,
                language: None,
                value: "*a*".into(),
            }
        );
        assert_eq!(
            served("match.yx$"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Match,
                quant: None,
                language: None,
                value: "yx$".into(),
            }
        );
        assert_eq!(
            served("imatch..*YY.*"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Imatch,
                quant: None,
                language: None,
                value: ".*YY.*".into(),
            }
        );
        assert_eq!(
            served("lt(any).{1,2}"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Lt,
                quant: Some(Quant::Any),
                language: None,
                value: "{1,2}".into(),
            }
        );
        assert_eq!(
            served("in.(1,3)"),
            ParsedFilter::In {
                negated: false,
                values: InList::Values(vec!["1".into(), "3".into()]),
            }
        );
        assert_eq!(
            served("in.()"),
            ParsedFilter::In {
                negated: false,
                values: InList::Empty,
            }
        );
        assert_eq!(
            served("in.(    )"),
            ParsedFilter::In {
                negated: false,
                values: InList::Empty,
            }
        );
        assert_eq!(
            served("in.(\"a,b\",c)"),
            ParsedFilter::In {
                negated: false,
                values: InList::Values(vec!["a,b".into(), "c".into()]),
            }
        );
        assert_eq!(
            served("is.null"),
            ParsedFilter::Is {
                negated: false,
                value: IsVal::Null,
            }
        );
        assert_eq!(
            served("is.NULL"),
            ParsedFilter::Is {
                negated: false,
                value: IsVal::Null,
            }
        );
        assert_eq!(
            served("is.not_null"),
            ParsedFilter::Is {
                negated: false,
                value: IsVal::NotNull,
            }
        );
        assert_eq!(
            served("isdistinct.2"),
            ParsedFilter::IsDistinct {
                negated: false,
                value: "2".into(),
            }
        );
        assert_eq!(
            served("not.eq.1"),
            ParsedFilter::Served {
                negated: true,
                op: ServedOp::Eq,
                quant: None,
                language: None,
                value: "1".into(),
            }
        );
        assert_eq!(
            served("not.in.()"),
            ParsedFilter::In {
                negated: true,
                values: InList::Empty,
            }
        );
        assert_eq!(
            served("not.is.not_null"),
            ParsedFilter::Is {
                negated: true,
                value: IsVal::NotNull,
            }
        );
    }

    #[test]
    fn parses_issue_29_operators() {
        assert_eq!(
            served("ov.[1,4)"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Ov,
                quant: None,
                language: None,
                value: "[1,4)".into(),
            }
        );
        assert_eq!(
            served("not.ov.[1,4)"),
            ParsedFilter::Served {
                negated: true,
                op: ServedOp::Ov,
                quant: None,
                language: None,
                value: "[1,4)".into(),
            }
        );
        assert_eq!(
            served("sl.[9,10)"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Sl,
                quant: None,
                language: None,
                value: "[9,10)".into(),
            }
        );
        assert_eq!(
            served("sr.[3,4)"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Sr,
                quant: None,
                language: None,
                value: "[3,4)".into(),
            }
        );
        assert_eq!(
            served("nxr.[4,7)"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Nxr,
                quant: None,
                language: None,
                value: "[4,7)".into(),
            }
        );
        assert_eq!(
            served("nxl.[4,7)"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Nxl,
                quant: None,
                language: None,
                value: "[4,7)".into(),
            }
        );
        assert_eq!(
            served("plfts.The Fat Rats"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Plfts,
                quant: None,
                language: None,
                value: "The Fat Rats".into(),
            }
        );
        assert_eq!(
            served("phfts(english).The Fat Cats"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Phfts,
                quant: None,
                language: Some("english".into()),
                value: "The Fat Cats".into(),
            }
        );
        assert_eq!(
            served("wfts(french).amusant impossible"),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Wfts,
                quant: None,
                language: Some("french".into()),
                value: "amusant impossible".into(),
            }
        );
        assert_eq!(
            served("plfts."),
            ParsedFilter::Served {
                negated: false,
                op: ServedOp::Plfts,
                quant: None,
                language: None,
                value: String::new(),
            }
        );
        assert!(parse_filter_value("nope.1").is_err());
        assert!(parse_filter_value("0").is_err());
        assert!(parse_filter_value("fts().x").is_err());
        assert!(parse_filter_value("eq(foo).1").is_err());
        assert!(parse_filter_value("is.foo").is_err());
        assert!(parse_filter_value("in.foo").is_err());
        assert!(parse_filter_value("neq(any).1").is_err());
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
            (
                "is.foo",
                5,
                "unexpected \"o\" expecting isVal: (null, not_null, true, false, unknown)",
            ),
            ("in.foo", 4, "unexpected \"f\" expecting \"(\""),
            (
                "not.is.foo",
                9,
                "unexpected \"o\" expecting isVal: (null, not_null, true, false, unknown)",
            ),
            ("in.(1", 6, "unexpected end of input expecting \")\""),
            (
                "is.",
                4,
                "unexpected end of input expecting isVal: (null, not_null, true, false, unknown)",
            ),
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

    fn bound(
        column: &str,
        op: ServedOp,
        quant: Option<Quant>,
        language: Option<&str>,
        value: &str,
        pg_type: &str,
    ) -> BoundFilter {
        BoundFilter {
            column: column.into(),
            negated: false,
            body: FilterBody::Op {
                op,
                quant,
                language: language.map(str::to_string),
                value: value.into(),
            },
            pg_type: Some(pg_type.into()),
        }
    }

    #[test]
    fn predicate_binds_values_and_casts_catalog_types() {
        let rendered = predicate_sql(
            "todos",
            &[
                bound("done", ServedOp::Eq, None, None, "true", "boolean"),
                bound("title", ServedOp::Ilike, None, None, "*spec*", "text"),
                bound(
                    "id",
                    ServedOp::Eq,
                    Some(Quant::Any),
                    None,
                    "{1,2}",
                    "bigint",
                ),
                bound(
                    "body",
                    ServedOp::Fts,
                    None,
                    Some("english"),
                    "cats",
                    "tsvector",
                ),
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
            &[bound(
                "id\".\"x",
                ServedOp::Eq,
                None,
                None,
                payload,
                "bigint",
            )],
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
            &[bound("id", ServedOp::Gt, None, None, "1", "int;drop")],
        );
        assert!(err.is_err());
    }

    #[test]
    fn issue_28_predicates_bind_and_whitelist() {
        let rendered = predicate_sql(
            "todos",
            &[
                BoundFilter {
                    column: "id".into(),
                    negated: true,
                    body: FilterBody::Op {
                        op: ServedOp::Eq,
                        quant: None,
                        language: None,
                        value: "1".into(),
                    },
                    pg_type: Some("bigint".into()),
                },
                bound("priority", ServedOp::Lt, None, None, "3", "integer"),
                bound("priority", ServedOp::Lte, None, None, "3", "integer"),
                bound("title", ServedOp::Like, None, None, "*a*", "text"),
                bound("title", ServedOp::Match, None, None, "yx$", "text"),
                bound("title", ServedOp::Imatch, None, None, ".*yy.*", "text"),
                bound("id", ServedOp::Neq, None, None, "2", "bigint"),
                BoundFilter {
                    column: "id".into(),
                    negated: false,
                    body: FilterBody::In(InList::Values(vec!["1".into(), "3".into()])),
                    pg_type: Some("bigint".into()),
                },
                BoundFilter {
                    column: "id".into(),
                    negated: false,
                    body: FilterBody::In(InList::Empty),
                    pg_type: Some("bigint".into()),
                },
                BoundFilter {
                    column: "done".into(),
                    negated: false,
                    body: FilterBody::Is(IsVal::Null),
                    pg_type: Some("boolean".into()),
                },
                BoundFilter {
                    column: "done".into(),
                    negated: true,
                    body: FilterBody::Is(IsVal::NotNull),
                    pg_type: Some("boolean".into()),
                },
                BoundFilter {
                    column: "id".into(),
                    negated: false,
                    body: FilterBody::IsDistinct("2".into()),
                    pg_type: Some("bigint".into()),
                },
            ],
        )
        .unwrap();
        assert_eq!(
            rendered.sql,
            "NOT \"todos\".\"id\" = ($1::text)::bigint \
             AND \"todos\".\"priority\" < ($2::text)::integer \
             AND \"todos\".\"priority\" <= ($3::text)::integer \
             AND \"todos\".\"title\" like ($4::text)::text \
             AND \"todos\".\"title\" ~ ($5::text)::text \
             AND \"todos\".\"title\" ~* ($6::text)::text \
             AND \"todos\".\"id\" <> ($7::text)::bigint \
             AND \"todos\".\"id\" = ANY (($8::text)::bigint[]) \
             AND \"todos\".\"id\" = ANY('{}') \
             AND \"todos\".\"done\" IS NULL \
             AND NOT \"todos\".\"done\" IS NOT NULL \
             AND \"todos\".\"id\" IS DISTINCT FROM ($9::text)::bigint"
        );
        assert_eq!(
            rendered.params,
            vec![
                "1",
                "3",
                "3",
                "%a%",
                "yx$",
                ".*yy.*",
                "2",
                "{\"1\",\"3\"}",
                "2"
            ]
        );
        let injected = predicate_sql(
            "todos",
            &[BoundFilter {
                column: "title".into(),
                negated: false,
                body: FilterBody::In(InList::Values(vec!["a\");drop".into()])),
                pg_type: Some("text".into()),
            }],
        )
        .unwrap();
        assert!(!injected.sql.contains("drop"));
        assert_eq!(injected.params, vec!["{\"a\\\");drop\"}"]);
    }

    #[test]
    fn issue_29_predicates_bind_range_and_fts() {
        let rendered = predicate_sql(
            "spans",
            &[
                bound("during", ServedOp::Ov, None, None, "[1,4)", "int4range"),
                bound("during", ServedOp::Sl, None, None, "[9,10)", "int4range"),
                bound("during", ServedOp::Sr, None, None, "[3,4)", "int4range"),
                bound("during", ServedOp::Nxr, None, None, "[4,7)", "int4range"),
                BoundFilter {
                    column: "during".into(),
                    negated: true,
                    body: FilterBody::Op {
                        op: ServedOp::Nxl,
                        quant: None,
                        language: None,
                        value: "[4,7)".into(),
                    },
                    pg_type: Some("int4range".into()),
                },
                bound("body", ServedOp::Plfts, None, None, "The Fat Rats", "text"),
                bound(
                    "body",
                    ServedOp::Phfts,
                    None,
                    Some("english"),
                    "The Fat Cats",
                    "text",
                ),
                bound(
                    "body",
                    ServedOp::Wfts,
                    None,
                    Some("french"),
                    "amusant impossible",
                    "text",
                ),
            ],
        )
        .unwrap();
        assert_eq!(
            rendered.sql,
            "\"spans\".\"during\" && ($1::text)::int4range \
             AND \"spans\".\"during\" << ($2::text)::int4range \
             AND \"spans\".\"during\" >> ($3::text)::int4range \
             AND \"spans\".\"during\" &< ($4::text)::int4range \
             AND NOT \"spans\".\"during\" &> ($5::text)::int4range \
             AND to_tsvector(\"spans\".\"body\") @@ plainto_tsquery(($6::text)::text) \
             AND to_tsvector(($7::text)::regconfig, \"spans\".\"body\") @@ phraseto_tsquery(($8::text)::regconfig, ($9::text)::text) \
             AND to_tsvector(($10::text)::regconfig, \"spans\".\"body\") @@ websearch_to_tsquery(($11::text)::regconfig, ($12::text)::text)"
        );
        assert_eq!(
            rendered.params,
            vec![
                "[1,4)",
                "[9,10)",
                "[3,4)",
                "[4,7)",
                "[4,7)",
                "The Fat Rats",
                "english",
                "english",
                "The Fat Cats",
                "french",
                "french",
                "amusant impossible",
            ]
        );
        let tsvector = predicate_sql(
            "docs",
            &[bound(
                "body",
                ServedOp::Plfts,
                None,
                Some("english"),
                "spec",
                "tsvector",
            )],
        )
        .unwrap();
        assert_eq!(
            tsvector.sql,
            "\"docs\".\"body\" @@ plainto_tsquery(($1::text)::regconfig, ($2::text)::text)"
        );
        let starred = predicate_sql(
            "spans",
            &[bound(
                "during",
                ServedOp::Ov,
                None,
                None,
                "[1,4)*",
                "int4range",
            )],
        )
        .unwrap();
        assert_eq!(starred.params, vec!["[1,4)*"]);
    }

    #[test]
    fn quote_ident_doubles_quotes_and_drops_nulls() {
        assert_eq!(quote_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_ident("a\0b"), "\"ab\"");
    }
}
