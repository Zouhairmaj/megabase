// Ported from postgrest src/library/PostgREST/ApiRequest/QueryParams.hs (MIT), pin v16.4,
// and postgrest src/library/PostgREST/Query/SqlFragment.hs (MIT), pin v16.4.

//! `GET` query parameters from `QueryParams.hs`.
//!
//! `parse` tries `order`, logic (`and` / `or`), `columns`, `select`, filters,
//! then `on_conflict`. The first failure wins. `columns` and `on_conflict` are
//! accepted on a read and do not change the row shape.

use crate::filter::{
    append_predicate, parse_filter_value, quote_ident, BoundFilter, FilterBody, FilterParseError,
    ParsedFilter, ServedOp, UnsafeType,
};
use crate::query::percent_decode;

/// Why a query string is not a served read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum QueryFail {
    /// `PGRST100` from the Parsec parsers.
    Parse { message: String, details: String },
    /// `PGRST108` for an `order`, `limit`, `offset`, `and`, or `or` on a resource that is not embedded.
    NotEmbedded { resource: String },
    /// A select aggregate whose unit is not served yet.
    Unimplemented(&'static str),
    /// An embed, spread, or JSON-path filter. The route unit stays 501.
    Route,
}

/// One successful `GET` query string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadQuery {
    select: SelectList,
    orders: Vec<OrderTerm>,
    page: Page,
    logic: Vec<LogicNode>,
    filters: Vec<(String, ParsedFilter)>,
    columns: Option<Vec<String>>,
    on_conflict: Option<Vec<String>>,
    handled: bool,
}

impl ReadQuery {
    fn inert() -> Self {
        Self {
            select: SelectList::Star,
            orders: Vec::new(),
            page: Page::inactive(),
            logic: Vec::new(),
            filters: Vec::new(),
            columns: None,
            on_conflict: None,
            handled: false,
        }
    }

    /// `true` when the query string carries a `key=value` pair.
    #[must_use]
    pub(crate) fn handled(&self) -> bool {
        self.handled
    }

    /// `true` when `limit` or `offset` is present.
    ///
    /// `PUT` rejects that with `PGRST114` (`ApiRequest.getRanges`).
    #[must_use]
    pub(crate) fn limits_rows(&self) -> bool {
        self.page.active
    }

    /// `columns` query parameter, when the client sent one.
    #[must_use]
    pub(crate) fn column_list(&self) -> Option<&[String]> {
        self.columns.as_deref()
    }

    /// Horizontal `eq` filters, or `None` when any filter is not a plain `eq`.
    ///
    /// `PUT` needs every filter to be `column=eq.value` with no `and`/`or`
    /// (`Plan.hs` single upsert). An empty list is still `Some`.
    #[must_use]
    pub(crate) fn eq_column_filters(&self) -> Option<Vec<(String, String)>> {
        if !self.logic.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        for (column, parsed) in &self.filters {
            match parsed {
                ParsedFilter::Served {
                    negated: false,
                    op: ServedOp::Eq,
                    quant: None,
                    language: None,
                    value,
                } => out.push((column.clone(), value.clone())),
                _ => return None,
            }
        }
        Some(out)
    }
}

/// SQL for one read, plus the `Content-Range` offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadSql {
    pub sql: String,
    pub params: Vec<String>,
    pub offset: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SelectList {
    Star,
    Fields(Vec<SelectField>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectField {
    column: String,
    alias: Option<String>,
    json: Vec<JsonStep>,
    cast: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum JsonStep {
    Arrow(JsonOperand),
    TwoArrow(JsonOperand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum JsonOperand {
    Key(String),
    /// Canonical `+N` or `-N`, bound and cast to `int`.
    Index(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OrderTerm {
    Column {
        column: String,
        json: Vec<JsonStep>,
        direction: Option<Direction>,
        nulls: Option<Nulls>,
    },
    Related {
        relation: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Nulls {
    First,
    Last,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Page {
    limit: Option<String>,
    offset: String,
    active: bool,
}

impl Page {
    fn inactive() -> Self {
        Self {
            limit: None,
            offset: "0".to_string(),
            active: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LogicNode {
    Expr {
        negated: bool,
        op: LogicOp,
        children: Vec<LogicNode>,
    },
    Filter {
        column: String,
        parsed: ParsedFilter,
    },
    /// JSON path inside a logic filter. The read is the route unit.
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogicOp {
    And,
    Or,
}

#[derive(Clone)]
struct PErr {
    line: usize,
    col: usize,
    unexpected: String,
    expects: Vec<String>,
}

#[derive(Clone)]
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

    fn rest(&self) -> &'a str {
        &self.input[self.byte..]
    }

    fn eof(&self) -> bool {
        self.byte >= self.input.len()
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn bump(&self, ch: char) -> Self {
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

    fn starts_with(&self, token: &str) -> bool {
        self.rest().starts_with(token)
    }
}

struct Collected {
    orders: Vec<(Vec<String>, String)>,
    logics: Vec<(Vec<String>, String)>,
    limits: Vec<(String, String)>,
    offsets: Vec<(String, String)>,
    filters: Vec<(String, String)>,
    columns: Option<String>,
    select: Option<String>,
    on_conflict: Option<String>,
    handled: bool,
}

/// Parse a `GET` query string the way `QueryParams.parse False` does.
///
/// # Errors
///
/// Returns [`QueryFail`] for a `PGRST100` parse error, a `PGRST108` embedded
/// parameter, or a select shape this crate does not serve yet.
pub(crate) fn parse_get_query(query: &str) -> Result<ReadQuery, QueryFail> {
    let collected = collect(query);
    if !collected.handled {
        return Ok(ReadQuery::inert());
    }
    let mut orders = Vec::new();
    for (path, value) in &collected.orders {
        let terms = parse_order_value(value)?;
        orders.push((path.clone(), terms));
    }
    let mut logic = Vec::new();
    let mut logic_json = false;
    for (path, expr) in &collected.logics {
        let node = parse_logic_tree(expr)?;
        if logic_has_json(&node) {
            logic_json = true;
        }
        logic.push((path.clone(), node));
    }
    let columns = match &collected.columns {
        Some(value) => Some(parse_name_list("columns parameter", value)?),
        None => None,
    };
    let select_raw = collected.select.as_deref().unwrap_or("*");
    let select = parse_select_value(select_raw)?;
    let mut filters = Vec::new();
    let mut embed_filter = logic_json;
    for (key, value) in &collected.filters {
        if let Err(error) = parse_filter_value(value) {
            return Err(filter_fail(error));
        }
        if key.is_empty() || key.contains('.') || key.contains("->") {
            embed_filter = true;
            continue;
        }
        let parsed = parse_filter_value(value).map_err(filter_fail)?;
        filters.push((key.clone(), parsed));
    }
    let on_conflict = match &collected.on_conflict {
        Some(value) => Some(parse_on_conflict_names(value)?),
        None => None,
    };
    if let ParsedSelect::Deferred(fail) = select {
        return Err(fail);
    }
    if embed_filter {
        return Err(QueryFail::Route);
    }
    let ParsedSelect::Ready(select) = select else {
        return Err(QueryFail::Route);
    };
    let (orders, order_embed) = first_root(orders);
    let orders = orders.unwrap_or_default();
    let page = page_from(&collected.limits, &collected.offsets);
    let (logic, logic_embed) = all_roots(logic);
    let related = orders.iter().find_map(|term| match term {
        OrderTerm::Related { relation } => Some(relation.clone()),
        OrderTerm::Column { .. } => None,
    });
    let resource = order_embed.or(page.embed).or(logic_embed).or(related);
    if let Some(resource) = resource {
        return Err(QueryFail::NotEmbedded { resource });
    }
    Ok(ReadQuery {
        select,
        orders,
        page: page.page,
        logic,
        filters,
        columns,
        on_conflict,
        handled: true,
    })
}

fn filter_fail(error: FilterParseError) -> QueryFail {
    QueryFail::Parse {
        message: error.message,
        details: error.details,
    }
}

fn logic_has_json(node: &LogicNode) -> bool {
    match node {
        LogicNode::Json => true,
        LogicNode::Filter { .. } => false,
        LogicNode::Expr { children, .. } => children.iter().any(logic_has_json),
    }
}

struct Paged {
    page: Page,
    embed: Option<String>,
}

fn collect(query: &str) -> Collected {
    let mut out = Collected {
        orders: Vec::new(),
        logics: Vec::new(),
        limits: Vec::new(),
        offsets: Vec::new(),
        filters: Vec::new(),
        columns: None,
        select: None,
        on_conflict: None,
        handled: false,
    };
    if query.is_empty() {
        return out;
    }
    for part in query.split('&') {
        if part.is_empty() {
            continue;
        }
        let Some((raw_key, raw_value)) = part.split_once('=') else {
            continue;
        };
        out.handled = true;
        let key = percent_decode(raw_key);
        let value = percent_decode(raw_value);
        if key == "select" {
            if out.select.is_none() {
                out.select = Some(value);
            }
            continue;
        }
        if key == "columns" {
            if out.columns.is_none() {
                out.columns = Some(value);
            }
            continue;
        }
        if key == "on_conflict" {
            if out.on_conflict.is_none() {
                out.on_conflict = Some(value);
            }
            continue;
        }
        let last = key.rsplit('.').next().unwrap_or(key.as_str());
        match last {
            "order" => out.orders.push((embed_path(&key), value)),
            "and" | "or" => out.logics.push(logic_call(&key, &value)),
            "limit" => out.limits.push((key, value)),
            "offset" => out.offsets.push((key, value)),
            _ => out.filters.push((key, value)),
        }
    }
    out
}

fn embed_path(key: &str) -> Vec<String> {
    let mut parts: Vec<String> = key.split('.').map(str::to_string).collect();
    parts.pop();
    parts
}

/// `pLogicPath`: `not` anywhere in the path rewrites the operator to `not.and` / `not.or`.
/// The returned path is the embed path. The call expression is `op` concatenated with the value.
fn logic_call(key: &str, value: &str) -> (Vec<String>, String) {
    let parts: Vec<&str> = key.split('.').collect();
    let op = parts.last().copied().unwrap_or(key);
    let negated = parts.contains(&"not");
    let path = parts
        .iter()
        .take(parts.len().saturating_sub(1))
        .filter(|part| **part != "not")
        .map(|part| (*part).to_string())
        .collect();
    let expr = if negated {
        format!("not.{op}{value}")
    } else {
        format!("{op}{value}")
    };
    (path, expr)
}

/// Last non-root parameter wins, matching `foldr` in `Plan.hs`.
/// Root `order` keeps the first list. Later root orders are overwritten.
fn first_root<T>(items: Vec<(Vec<String>, T)>) -> (Option<T>, Option<String>) {
    if let Some((path, _)) = items.iter().rev().find(|(path, _)| !path.is_empty()) {
        return (None, Some(path[0].clone()));
    }
    (items.into_iter().next().map(|(_, value)| value), None)
}

/// Every root logic tree is kept. A non-root tree is `PGRST108`.
fn all_roots<T>(items: Vec<(Vec<String>, T)>) -> (Vec<T>, Option<String>) {
    if let Some((path, _)) = items.iter().rev().find(|(path, _)| !path.is_empty()) {
        return (Vec::new(), Some(path[0].clone()));
    }
    (items.into_iter().map(|(_, value)| value).collect(), None)
}

fn page_from(limits: &[(String, String)], offsets: &[(String, String)]) -> Paged {
    let mut root_limit: Option<Option<String>> = None;
    let mut root_offset: Option<Option<String>> = None;
    let mut embed = None;
    for (key, value) in limits {
        let path = embed_path(key);
        if path.is_empty() {
            root_limit = Some(read_integer(value));
        } else {
            embed = Some(path[0].clone());
        }
    }
    for (key, value) in offsets {
        let path = embed_path(&replace_last_limit(key));
        if path.is_empty() {
            root_offset = Some(read_integer(value));
        } else {
            embed = Some(path[0].clone());
        }
    }
    // `foldr` reports the rightmost embed. Scan limits then offsets, keeping the last.
    let limit = root_limit.flatten();
    let offset = root_offset.flatten();
    let page = match (limit, offset) {
        (None, None) => Page::inactive(),
        (None, Some(offset)) if offset == "0" => Page::inactive(),
        (limit, offset) => Page {
            limit,
            offset: offset.unwrap_or_else(|| "0".to_string()),
            active: true,
        },
    };
    Paged { page, embed }
}

fn replace_last_limit(key: &str) -> String {
    match key.rsplit_once('.') {
        Some((head, _)) => format!("{head}.limit"),
        None => "limit".to_string(),
    }
}

/// Haskell `readMaybe` for `Integer`: leading whitespace, optional sign, digits, nothing left.
fn read_integer(input: &str) -> Option<String> {
    let rest = input.trim_start_matches(|c: char| c.is_whitespace());
    if rest.is_empty() {
        return None;
    }
    let (sign, digits) = if let Some(rest) = rest.strip_prefix('+') {
        ("", rest)
    } else if let Some(rest) = rest.strip_prefix('-') {
        ("-", rest)
    } else {
        ("", rest)
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let trimmed = digits.trim_start_matches('0');
    let body = if trimmed.is_empty() { "0" } else { trimmed };
    if sign == "-" && body == "0" {
        Some("0".to_string())
    } else if sign == "-" {
        Some(format!("-{body}"))
    } else {
        Some(body.to_string())
    }
}

// megabase:unit rest:query-param:columns
fn parse_name_list(kind: &str, input: &str) -> Result<Vec<String>, QueryFail> {
    parse_columns(input).map_err(|err| named_error(kind, input, &err))
}

// megabase:unit rest:query-param:on_conflict
fn parse_on_conflict_names(input: &str) -> Result<Vec<String>, QueryFail> {
    parse_name_list("on_conflict parameter", input)
}

fn parse_columns(input: &str) -> Result<Vec<String>, PErr> {
    let mut at = At::start(input);
    let mut names = Vec::new();
    loop {
        let (name, next) = parse_field_name(&at)?;
        names.push(name);
        at = skip_ws(&next);
        if at.peek() == Some(',') {
            at = skip_ws(&at.bump(','));
            continue;
        }
        if at.eof() {
            return Ok(names);
        }
        return Err(char_err(&at, "\",\" or end of input"));
    }
}

fn named_error(kind: &str, input: &str, err: &PErr) -> QueryFail {
    QueryFail::Parse {
        message: format!(
            "\"failed to parse {kind} ({input})\" (line {}, column {})",
            err.line, err.col
        ),
        details: format!(
            "unexpected {} expecting {}",
            shown_unexpected(err),
            join_expecting(&err.expects)
        ),
    }
}

fn parse_order_value(input: &str) -> Result<Vec<OrderTerm>, QueryFail> {
    parse_order(input).map_err(|err| named_error("order", input, &err))
}

fn parse_logic_tree(input: &str) -> Result<LogicNode, QueryFail> {
    let mut at = At::start(input);
    let node = logic_tree(&mut at).map_err(|err| named_error("logic tree", input, &err))?;
    let at = skip_ws(&at);
    if !at.eof() {
        let err = char_err(&at, "end of input");
        return Err(named_error("logic tree", input, &err));
    }
    Ok(node)
}

fn parse_select_value(input: &str) -> Result<ParsedSelect, QueryFail> {
    parse_select(input).map_err(|err| named_error("select parameter", input, &err))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedSelect {
    Ready(SelectList),
    Deferred(QueryFail),
}

fn parse_select(input: &str) -> Result<ParsedSelect, PErr> {
    if input.is_empty() {
        return Ok(ParsedSelect::Ready(SelectList::Star));
    }
    let mut at = At::start(input);
    let mut fields = Vec::new();
    let mut deferred: Option<QueryFail> = None;
    loop {
        at = skip_ws(&at);
        if at.eof() {
            if fields.is_empty() && deferred.is_none() {
                return Err(field_name_err(&at));
            }
            break;
        }
        let start = at.clone();
        match parse_select_item(&mut at)? {
            SelectItem::Field(field) => fields.push(field),
            SelectItem::Defer(fail) => {
                if deferred.is_none() {
                    deferred = Some(fail);
                }
            }
        }
        if at.byte == start.byte {
            return Err(field_name_err(&at));
        }
        at = skip_ws(&at);
        if at.peek() == Some(',') {
            at = at.bump(',');
            continue;
        }
        if at.eof() {
            break;
        }
        return Err(char_err(&at, "\",\" or end of input"));
    }
    if let Some(fail) = deferred {
        return Ok(ParsedSelect::Deferred(fail));
    }
    if fields.is_empty() {
        return Ok(ParsedSelect::Ready(SelectList::Star));
    }
    if fields.len() == 1
        && fields[0].column == "*"
        && fields[0].alias.is_none()
        && fields[0].json.is_empty()
        && fields[0].cast.is_none()
    {
        return Ok(ParsedSelect::Ready(SelectList::Star));
    }
    Ok(ParsedSelect::Ready(SelectList::Fields(fields)))
}

enum SelectItem {
    Field(SelectField),
    Defer(QueryFail),
}

fn parse_select_item<'a>(at: &mut At<'a>) -> Result<SelectItem, PErr> {
    if at.starts_with("...") {
        return parse_spread(at);
    }
    let saved = at.clone();
    if let Some(item) = try_relation(at)? {
        return Ok(item);
    }
    *at = saved;
    parse_field_select(at)
}

fn parse_spread<'a>(at: &mut At<'a>) -> Result<SelectItem, PErr> {
    *at = bump_str(at, "...");
    let (_name, next) = parse_field_name(at)?;
    *at = next;
    let join = parse_embed_params(at)?;
    if at.peek() != Some('(') {
        return Err(char_err(at, "\"(\""));
    }
    skip_balanced(at)?;
    Ok(SelectItem::Defer(join_fail(join)))
}

fn try_relation<'a>(at: &mut At<'a>) -> Result<Option<SelectItem>, PErr> {
    let alias_saved = at.clone();
    let alias = optional_alias(at)?;
    let (name, next) = match parse_field_name(at) {
        Ok(parsed) => parsed,
        Err(_) => {
            *at = alias_saved;
            return Ok(None);
        }
    };
    if name == "count" {
        *at = alias_saved;
        return Ok(None);
    }
    *at = next;
    let join = match parse_embed_params(at) {
        Ok(join) => join,
        Err(_) => {
            *at = alias_saved;
            return Ok(None);
        }
    };
    if at.peek() != Some('(') {
        *at = alias_saved;
        return Ok(None);
    }
    skip_balanced(at)?;
    let _ = alias;
    Ok(Some(SelectItem::Defer(join_fail(join))))
}

fn join_fail(join: Option<JoinKind>) -> QueryFail {
    match join {
        Some(JoinKind::Inner) => QueryFail::Unimplemented("rest:embed-join:inner"),
        Some(JoinKind::Left) => QueryFail::Unimplemented("rest:embed-join:left"),
        None => QueryFail::Route,
    }
}

#[derive(Clone, Copy)]
enum JoinKind {
    Inner,
    Left,
}

fn parse_embed_params<'a>(at: &mut At<'a>) -> Result<Option<JoinKind>, PErr> {
    let mut join = None;
    for _ in 0..2 {
        if at.peek() != Some('!') {
            break;
        }
        *at = at.bump('!');
        if at.starts_with("inner") {
            *at = bump_str(at, "inner");
            join = Some(JoinKind::Inner);
        } else if at.starts_with("left") {
            *at = bump_str(at, "left");
            join = Some(JoinKind::Left);
        } else {
            let (_hint, next) = parse_field_name(at)?;
            *at = next;
        }
    }
    Ok(join)
}

fn parse_field_select<'a>(at: &mut At<'a>) -> Result<SelectItem, PErr> {
    if at.starts_with("*") {
        let next = at.bump('*');
        if matches!(next.peek(), None | Some(',' | ')')) {
            *at = next;
            return Ok(SelectItem::Field(SelectField {
                column: "*".to_string(),
                alias: None,
                json: Vec::new(),
                cast: None,
            }));
        }
        return Err(char_err(&next, "\")\", \",\" or end of input"));
    }
    let alias = optional_alias(at)?;
    if at.starts_with("count()") {
        *at = bump_str(at, "count()");
        let cast = optional_cast(at)?;
        let _ = cast;
        return Ok(SelectItem::Defer(QueryFail::Unimplemented(
            "rest:aggregate:count",
        )));
    }
    let (name, next) = parse_field_name(at)?;
    *at = next;
    let json = parse_json_path(at)?;
    let cast = optional_cast(at)?;
    if at.peek() == Some('.') {
        let after = at.bump('.');
        if let Some(unit) = aggregate_unit(&after) {
            *at = bump_str(&after, unit.0);
            let _ = optional_cast(at)?;
            return Ok(SelectItem::Defer(QueryFail::Unimplemented(unit.1)));
        }
    }
    if !matches!(at.peek(), None | Some(',' | ')')) {
        return Err(char_err(at, "\")\", \",\" or end of input"));
    }
    if let Some(cast) = &cast {
        if !cast_ok(cast) {
            return Err(char_err(at, "letter or digit"));
        }
    }
    Ok(SelectItem::Field(SelectField {
        column: name,
        alias,
        json,
        cast,
    }))
}

fn aggregate_unit<'a>(at: &At<'a>) -> Option<(&'static str, &'static str)> {
    for (token, unit) in [
        ("sum()", "rest:aggregate:sum"),
        ("avg()", "rest:aggregate:avg"),
        ("count()", "rest:aggregate:count"),
        ("max()", "rest:aggregate:max"),
        ("min()", "rest:aggregate:min"),
    ] {
        if at.starts_with(token) {
            return Some((token, unit));
        }
    }
    None
}

fn optional_alias<'a>(at: &mut At<'a>) -> Result<Option<String>, PErr> {
    let saved = at.clone();
    let Ok((name, next)) = parse_field_name(at) else {
        *at = saved;
        return Ok(None);
    };
    if next.peek() == Some(':') && !next.bump(':').starts_with(":") {
        *at = next.bump(':');
        return Ok(Some(name));
    }
    *at = saved;
    Ok(None)
}

fn optional_cast<'a>(at: &mut At<'a>) -> Result<Option<String>, PErr> {
    if !at.starts_with("::") {
        return Ok(None);
    }
    let after = bump_str(at, "::");
    let (ident, next) = parse_identifier(&after)?;
    if !cast_ok(&ident) {
        return Err(char_err(&after, "letter or digit"));
    }
    *at = next;
    Ok(Some(ident))
}

fn cast_ok(cast: &str) -> bool {
    !cast.is_empty()
        && !cast.contains("--")
        && !cast.contains("/*")
        && cast
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '$'))
}

fn skip_balanced<'a>(at: &mut At<'a>) -> Result<(), PErr> {
    if at.peek() != Some('(') {
        return Err(char_err(at, "\"(\""));
    }
    let mut depth = 0_i32;
    loop {
        let Some(ch) = at.peek() else {
            return Err(PErr {
                line: at.line,
                col: at.col,
                unexpected: String::new(),
                expects: vec!["\")\"".to_string()],
            });
        };
        *at = at.bump(ch);
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(());
                }
            }
            _ => {}
        }
    }
}

fn parse_order(input: &str) -> Result<Vec<OrderTerm>, PErr> {
    let mut at = skip_ws(&At::start(input));
    if at.eof() {
        return Err(field_name_err(&at));
    }
    let mut terms = Vec::new();
    loop {
        let (term, next) = parse_order_term(&at)?;
        terms.push(term);
        at = skip_ws(&next);
        if at.peek() == Some(',') {
            at = skip_ws(&at.bump(','));
            continue;
        }
        if at.eof() {
            return Ok(terms);
        }
        return Err(char_err(&at, "\",\" or end of input"));
    }
}

fn parse_order_term<'a>(at: &At<'a>) -> Result<(OrderTerm, At<'a>), PErr> {
    let saved = at.clone();
    if let Some(parsed) = try_order_relation(at)? {
        return Ok(parsed);
    }
    parse_order_column(&saved)
}

fn try_order_relation<'a>(at: &At<'a>) -> Result<Option<(OrderTerm, At<'a>)>, PErr> {
    let (name, next) = match parse_field_name(at) {
        Ok(parsed) => parsed,
        Err(_) => return Ok(None),
    };
    if next.peek() != Some('(') {
        return Ok(None);
    }
    let mut inner = next.bump('(');
    inner = skip_ws(&inner);
    let (column, after_field) = parse_order_field(&inner)?;
    let mut after = after_field;
    if after.peek() != Some(')') {
        return Err(char_err(&after, "\")\""));
    }
    after = after.bump(')');
    let (_dir, _nulls, after) = parse_suffix(&after)?;
    let _ = column;
    Ok(Some((OrderTerm::Related { relation: name }, after)))
}

fn parse_order_column<'a>(at: &At<'a>) -> Result<(OrderTerm, At<'a>), PErr> {
    let (field, next) = parse_order_field(at)?;
    let (direction, nulls, next) = parse_suffix(&next)?;
    Ok((
        OrderTerm::Column {
            column: field.0,
            json: field.1,
            direction,
            nulls,
        },
        next,
    ))
}

fn parse_order_field<'a>(at: &At<'a>) -> Result<((String, Vec<JsonStep>), At<'a>), PErr> {
    let at = skip_ws(at);
    let (name, next) = parse_field_name(&at)?;
    let mut cursor = next;
    let json = parse_json_path(&mut cursor)?;
    let cursor = skip_ws(&cursor);
    Ok(((name, json), cursor))
}

fn parse_suffix<'a>(at: &At<'a>) -> Result<(Option<Direction>, Option<Nulls>, At<'a>), PErr> {
    let mut cursor = at.clone();
    let mut direction = None;
    let mut direction_err = None;
    match try_dotted(&cursor, &["asc", "desc"]) {
        Ok((0, next)) => {
            direction = Some(Direction::Asc);
            cursor = next;
        }
        Ok((1, next)) => {
            direction = Some(Direction::Desc);
            cursor = next;
        }
        Ok(_) => unreachable!("direction keyword index"),
        Err(err) => direction_err = Some(err),
    }
    let mut nulls = None;
    let mut nulls_err = None;
    match try_dotted(&cursor, &["nullsfirst", "nullslast"]) {
        Ok((0, next)) => {
            nulls = Some(Nulls::First);
            cursor = next;
        }
        Ok((1, next)) => {
            nulls = Some(Nulls::Last);
            cursor = next;
        }
        Ok(_) => unreachable!("nulls keyword index"),
        Err(err) => nulls_err = Some(err),
    }
    match parse_end(&cursor) {
        Ok(next) => Ok((direction, nulls, next)),
        Err(end_err) => {
            let mut err = end_err;
            if let Some(nulls_err) = nulls_err {
                err = merge(nulls_err, err);
            }
            if let Some(direction_err) = direction_err {
                err = merge(direction_err, err);
            }
            Err(err)
        }
    }
}

fn try_dotted<'a>(at: &At<'a>, words: &[&str]) -> Result<(usize, At<'a>), PErr> {
    if at.peek() != Some('.') {
        return Err(delimiter_err(at));
    }
    let after_dot = at.bump('.');
    let mut acc: Option<PErr> = None;
    for (index, word) in words.iter().enumerate() {
        match parse_string(&after_dot, word) {
            Ok(next) => return Ok((index, next)),
            Err(err) => {
                acc = Some(match acc {
                    Some(prev) => merge(prev, err),
                    None => err,
                })
            }
        }
    }
    Err(acc.unwrap_or_else(|| delimiter_err(at)))
}

fn parse_end<'a>(at: &At<'a>) -> Result<At<'a>, PErr> {
    if at.eof() || at.peek() == Some(',') {
        return Ok(at.clone());
    }
    let comma = PErr {
        line: at.line,
        col: at.col,
        unexpected: at.peek().map(show_haskell_char).unwrap_or_default(),
        expects: vec!["\",\"".to_string()],
    };
    let end = PErr {
        line: at.line,
        col: at.col,
        unexpected: at.peek().map(show_haskell_char).unwrap_or_default(),
        expects: vec!["end of input".to_string()],
    };
    Err(merge(comma, end))
}

fn delimiter_err<'a>(at: &At<'a>) -> PErr {
    PErr {
        line: at.line,
        col: at.col,
        unexpected: at.peek().map(show_haskell_char).unwrap_or_default(),
        expects: vec!["delimiter (.)".to_string()],
    }
}

fn logic_tree<'a>(at: &mut At<'a>) -> Result<LogicNode, PErr> {
    let saved = at.clone();
    match logic_filter(at) {
        Ok(node) => Ok(node),
        Err(filter_err) => {
            *at = saved;
            match logic_expr(at) {
                Ok(node) => Ok(node),
                Err(expr_err) => Err(merge(filter_err, expr_err)),
            }
        }
    }
}

fn logic_expr<'a>(at: &mut At<'a>) -> Result<LogicNode, PErr> {
    let negated = if at.starts_with("not.") {
        *at = bump_str(at, "not.");
        true
    } else {
        false
    };
    let op = if at.starts_with("and") {
        *at = bump_str(at, "and");
        LogicOp::And
    } else if at.starts_with("or") {
        *at = bump_str(at, "or");
        LogicOp::Or
    } else {
        return Err(PErr {
            line: at.line,
            col: at.col,
            unexpected: at.peek().map(show_string_char).unwrap_or_default(),
            expects: vec!["logic operator (and, or)".to_string()],
        });
    };
    at_skip(at);
    if at.peek() != Some('(') {
        return Err(char_err(at, "\"(\""));
    }
    *at = at.bump('(');
    let mut children = Vec::new();
    loop {
        at_skip(at);
        children.push(logic_tree(at)?);
        at_skip(at);
        if at.peek() == Some(',') {
            *at = at.bump(',');
            continue;
        }
        if at.peek() == Some(')') {
            *at = at.bump(')');
            break;
        }
        return Err(char_err(at, "\")\""));
    }
    if children.is_empty() {
        return Err(char_err(at, "field name (* or [a..z0..9_$])"));
    }
    Ok(LogicNode::Expr {
        negated,
        op,
        children,
    })
}

fn logic_filter<'a>(at: &mut At<'a>) -> Result<LogicNode, PErr> {
    let (name, next) = parse_field_name(at)?;
    *at = next;
    let mut json = false;
    while at.starts_with("->") {
        json = true;
        if at.starts_with("->>") {
            *at = bump_str(at, "->>");
        } else {
            *at = bump_str(at, "->");
        }
        let _ = parse_json_operand(at)?;
    }
    if at.peek() != Some('.') {
        return Err(delimiter_err(at));
    }
    let op_at = at.bump('.');
    let end = logic_operator_end(op_at.rest()).map_err(|_| operator_err(&op_at))?;
    let expr = &op_at.rest()[..end];
    let parsed = parse_filter_value(expr).map_err(|error| PErr {
        line: op_at.line,
        col: op_at.col,
        unexpected: unexpected_from_details(&error.details),
        expects: expects_from_details(&error.details),
    })?;
    *at = bump_str(&op_at, expr);
    if json {
        return Ok(LogicNode::Json);
    }
    Ok(LogicNode::Filter {
        column: name,
        parsed,
    })
}

fn unexpected_from_details(details: &str) -> String {
    details
        .split(" expecting ")
        .next()
        .and_then(|part| part.strip_prefix("unexpected "))
        .unwrap_or("end of input")
        .to_string()
}

fn expects_from_details(details: &str) -> Vec<String> {
    let Some(expecting) = details.split(" expecting ").nth(1) else {
        return vec!["operator (eq, gt, ...)".to_string()];
    };
    // Keep the joined phrase. `named_error` joins again, so store one label.
    vec![expecting.to_string()]
}

fn operator_err<'a>(at: &At<'a>) -> PErr {
    PErr {
        line: at.line,
        col: at.col,
        unexpected: at.peek().map(show_string_char).unwrap_or_default(),
        expects: vec!["operator (eq, gt, ...)".to_string()],
    }
}

fn logic_operator_end(input: &str) -> Result<usize, ()> {
    let bytes = input.as_bytes();
    let mut i = 0;
    if input[i..].starts_with("not.") {
        i += 4;
    }
    let start = i;
    while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
        i += 1;
    }
    if i == start {
        return Err(());
    }
    let op = &input[start..i];
    if op == "in" {
        if !input[i..].starts_with(".(") {
            return Err(());
        }
        i += 2;
        let mut depth = 1_i32;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        if depth != 0 {
            return Err(());
        }
        return Ok(i);
    }
    if i < bytes.len() && bytes[i] == b'(' {
        i += 1;
        while i < bytes.len() && bytes[i] != b')' {
            i += 1;
        }
        if i >= bytes.len() {
            return Err(());
        }
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'.' {
        return Err(());
    }
    i += 1;
    i += logic_single_len(&input[i..]);
    Ok(i)
}

fn logic_single_len(input: &str) -> usize {
    let bytes = input.as_bytes();
    if bytes.first() == Some(&b'"') {
        let mut i = 1;
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                let next = bytes.get(i + 1).copied();
                if next.is_none() || next == Some(b',') || next == Some(b')') {
                    return i + 1;
                }
                break;
            }
            i += 1;
        }
    }
    if bytes.first() == Some(&b'{') {
        if let Some(end) = input.find('}') {
            if !input[1..end].contains('{') {
                return end + 1;
            }
        }
    }
    input.find([',', ')']).unwrap_or(input.len())
}

fn parse_json_path<'a>(at: &mut At<'a>) -> Result<Vec<JsonStep>, PErr> {
    let mut steps = Vec::new();
    while at.starts_with("->") {
        let two = at.starts_with("->>");
        *at = if two {
            bump_str(at, "->>")
        } else {
            bump_str(at, "->")
        };
        let operand = parse_json_operand(at)?;
        steps.push(if two {
            JsonStep::TwoArrow(operand)
        } else {
            JsonStep::Arrow(operand)
        });
    }
    Ok(steps)
}

fn parse_json_operand<'a>(at: &mut At<'a>) -> Result<JsonOperand, PErr> {
    if let Some(index) = json_index(at.rest()) {
        *at = bump_str(at, &index.raw);
        return Ok(JsonOperand::Index(index.canonical));
    }
    let (key, next) = parse_json_key(at)?;
    *at = next;
    Ok(JsonOperand::Key(key))
}

struct IndexParse {
    raw: String,
    canonical: String,
}

fn json_index(input: &str) -> Option<IndexParse> {
    let mut chars = input.chars();
    let sign = match chars.next() {
        Some('-') => "-",
        Some(ch) if ch.is_ascii_digit() => {
            return finish_index(input, "", 0);
        }
        _ => return None,
    };
    let _ = sign;
    if !matches!(input.chars().nth(1), Some(ch) if ch.is_ascii_digit()) {
        return None;
    }
    finish_index(input, "-", 1)
}

fn finish_index(input: &str, sign: &str, start: usize) -> Option<IndexParse> {
    let digits: String = input[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    let raw_len = start + digits.len();
    let rest = &input[raw_len..];
    if !(rest.is_empty()
        || rest.starts_with("->")
        || rest.starts_with("::")
        || rest.starts_with('.')
        || rest.starts_with(','))
    {
        return None;
    }
    let canonical = if sign == "-" {
        format!("-{digits}")
    } else {
        format!("+{digits}")
    };
    Some(IndexParse {
        raw: input[..raw_len].to_string(),
        canonical,
    })
}

fn parse_json_key<'a>(at: &At<'a>) -> Result<(String, At<'a>), PErr> {
    if at.peek() == Some('"') {
        return parse_quoted(at);
    }
    let mut cursor = at.clone();
    let start = cursor.byte;
    if cursor.eof() {
        return Err(char_err(at, "json key"));
    }
    while let Some(ch) = cursor.peek() {
        if matches!(ch, '(' | '-' | ':' | '.' | ',' | '>' | ')') {
            break;
        }
        cursor = cursor.bump(ch);
    }
    if cursor.byte == start {
        return Err(char_err(at, "json key"));
    }
    let raw = &at.input[start..cursor.byte];
    Ok((raw.trim().to_string(), cursor))
}

fn parse_field_name<'a>(at: &At<'a>) -> Result<(String, At<'a>), PErr> {
    let at = skip_ws(at);
    if at.peek() == Some('"') {
        return parse_quoted(&at);
    }
    parse_dashed_ident(&at)
}

fn parse_dashed_ident<'a>(at: &At<'a>) -> Result<(String, At<'a>), PErr> {
    let (first, mut cursor) = parse_identifier(at)?;
    let mut name = first;
    while cursor.peek() == Some('-') && !cursor.bump('-').starts_with(">") {
        let after = cursor.bump('-');
        let (next, next_at) = parse_identifier(&after)?;
        name.push('-');
        name.push_str(&next);
        cursor = next_at;
    }
    Ok((name, cursor))
}

fn parse_identifier<'a>(at: &At<'a>) -> Result<(String, At<'a>), PErr> {
    let mut cursor = at.clone();
    let start = cursor.byte;
    while let Some(ch) = cursor.peek() {
        if is_ident_char(ch) {
            cursor = cursor.bump(ch);
        } else {
            break;
        }
    }
    if cursor.byte == start {
        return Err(field_name_err(at));
    }
    let raw = &at.input[start..cursor.byte];
    let name = raw.trim().to_string();
    if name.is_empty() {
        return Err(field_name_err(at));
    }
    Ok((name, cursor))
}

fn is_ident_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | ' ' | '$')
}

fn parse_quoted<'a>(at: &At<'a>) -> Result<(String, At<'a>), PErr> {
    if at.peek() != Some('"') {
        return Err(field_name_err(at));
    }
    let mut cursor = at.bump('"');
    let mut out = String::new();
    loop {
        let Some(ch) = cursor.peek() else {
            return Err(PErr {
                line: cursor.line,
                col: cursor.col,
                unexpected: String::new(),
                expects: vec!["\"\\\"\"".to_string()],
            });
        };
        cursor = cursor.bump(ch);
        if ch == '\\' {
            let Some(escaped) = cursor.peek() else {
                return Err(field_name_err(&cursor));
            };
            out.push(escaped);
            cursor = cursor.bump(escaped);
            continue;
        }
        if ch == '"' {
            return Ok((out, cursor));
        }
        out.push(ch);
    }
}

fn field_name_err<'a>(at: &At<'a>) -> PErr {
    PErr {
        line: at.line,
        col: at.col,
        unexpected: at.peek().map(show_haskell_char).unwrap_or_default(),
        expects: vec!["field name (* or [a..z0..9_$])".to_string()],
    }
}

fn char_err<'a>(at: &At<'a>, expect: &str) -> PErr {
    PErr {
        line: at.line,
        col: at.col,
        unexpected: at.peek().map(show_haskell_char).unwrap_or_default(),
        expects: vec![expect.to_string()],
    }
}

fn parse_string<'a>(at: &At<'a>, token: &str) -> Result<At<'a>, PErr> {
    let mut cursor = at.clone();
    for expected in token.chars() {
        match cursor.peek() {
            Some(got) if got == expected => cursor = cursor.bump(got),
            Some(got) => {
                return Err(PErr {
                    line: at.line,
                    col: at.col,
                    unexpected: show_string_char(got),
                    expects: vec![format!("\"{token}\"")],
                });
            }
            None => {
                return Err(PErr {
                    line: at.line,
                    col: at.col,
                    unexpected: String::new(),
                    expects: vec![format!("\"{token}\"")],
                });
            }
        }
    }
    Ok(cursor)
}

fn merge(left: PErr, right: PErr) -> PErr {
    match (left.line, left.col).cmp(&(right.line, right.col)) {
        std::cmp::Ordering::Greater => left,
        std::cmp::Ordering::Less => right,
        std::cmp::Ordering::Equal => PErr {
            line: left.line,
            col: left.col,
            unexpected: left.unexpected,
            expects: {
                let mut expects = left.expects;
                expects.extend(right.expects);
                expects
            },
        },
    }
}

fn shown_unexpected(err: &PErr) -> String {
    if err.unexpected.is_empty() {
        "end of input".to_string()
    } else {
        err.unexpected.clone()
    }
}

fn join_expecting(expects: &[String]) -> String {
    let mut unique = Vec::new();
    for item in expects {
        if item.is_empty() || unique.iter().any(|seen: &String| seen == item) {
            continue;
        }
        unique.push(item.clone());
    }
    let Some(last) = unique.pop() else {
        return String::new();
    };
    if unique.is_empty() {
        return last;
    }
    let mut out = unique.join(", ");
    out.push_str(" or ");
    out.push_str(&last);
    out
}

fn show_haskell_char(ch: char) -> String {
    format!("'{}'", escape_haskell(ch, false))
}

fn show_string_char(ch: char) -> String {
    format!("\"{}\"", escape_haskell(ch, true))
}

fn escape_haskell(ch: char, in_string: bool) -> String {
    match ch {
        '\\' => "\\\\".to_string(),
        '"' if in_string => "\\\"".to_string(),
        '\'' if !in_string => "\\'".to_string(),
        '\n' => "\\n".to_string(),
        '\r' => "\\r".to_string(),
        '\t' => "\\t".to_string(),
        other => other.to_string(),
    }
}

fn skip_ws<'a>(at: &At<'a>) -> At<'a> {
    let mut cursor = at.clone();
    while matches!(cursor.peek(), Some(' ' | '\t')) {
        cursor = cursor.bump(cursor.peek().unwrap_or(' '));
    }
    cursor
}

fn at_skip<'a>(at: &mut At<'a>) {
    *at = skip_ws(at);
}

fn bump_str<'a>(at: &At<'a>, token: &str) -> At<'a> {
    let mut cursor = at.clone();
    for ch in token.chars() {
        cursor = cursor.bump(ch);
    }
    cursor
}

/// Build the read statement for `query`.
///
/// `column_types` is `(name, format_type)` in `attnum` order. Values, JSON
/// keys, and limit/offset integers are bound parameters.
///
/// # Errors
///
/// Returns [`UnsafeType`] when a filter's catalog type cannot be spliced.
pub(crate) fn build_read_sql(
    relation: &str,
    query: &ReadQuery,
    column_types: &[(&str, &str)],
) -> Result<ReadSql, UnsafeType> {
    let mut params = Vec::new();
    let relation_ident = quote_ident(relation);
    let schema = quote_ident("public");
    let mut select_sql = String::new();
    // megabase:unit rest:query-param:select
    push_select(&mut select_sql, &mut params, &relation_ident, &query.select);
    let mut inner = format!("SELECT {select_sql} FROM {schema}.{relation_ident}");
    if !query.filters.is_empty() || !query.logic.is_empty() {
        inner.push_str(" WHERE ");
        push_where(
            &mut inner,
            &mut params,
            relation,
            &query.filters,
            &query.logic,
            column_types,
        )?;
    }
    if !query.orders.is_empty() {
        inner.push(' ');
        // megabase:unit rest:query-param:order
        push_order(&mut inner, &mut params, &relation_ident, &query.orders);
    }
    if query.page.active {
        inner.push(' ');
        // megabase:unit rest:query-param:limit
        // megabase:unit rest:query-param:offset
        push_page(&mut inner, &mut params, &query.page);
    }
    let sql = format!(
        "WITH _megabase_src AS MATERIALIZED ({inner}) \
         SELECT (SELECT pg_catalog.count(*) FROM _megabase_src), \
         coalesce((SELECT json_agg(t.row ORDER BY t.rn) FROM \
         (SELECT to_json(_megabase_src) AS row, row_number() OVER () AS rn \
         FROM _megabase_src) t), '[]'::json)::text"
    );
    let offset = query.page.offset.parse::<i64>().unwrap_or(0);
    let _ = (&query.columns, &query.on_conflict);
    Ok(ReadSql {
        sql,
        params,
        offset,
    })
}

fn push_select(sql: &mut String, params: &mut Vec<String>, relation: &str, select: &SelectList) {
    match select {
        SelectList::Star => {
            sql.push_str(relation);
            sql.push_str(".*");
        }
        SelectList::Fields(fields) => {
            for (index, field) in fields.iter().enumerate() {
                if index > 0 {
                    sql.push_str(", ");
                }
                push_select_field(sql, params, relation, field);
            }
        }
    }
}

fn push_select_field(
    sql: &mut String,
    params: &mut Vec<String>,
    relation: &str,
    field: &SelectField,
) {
    let mut expr = String::new();
    if field.column == "*" {
        expr.push_str(relation);
        expr.push_str(".*");
    } else {
        expr.push_str(relation);
        expr.push('.');
        expr.push_str(&quote_ident(&field.column));
        push_json(&mut expr, params, &field.json);
    }
    if let Some(cast) = &field.cast {
        sql.push_str("CAST( ");
        sql.push_str(&expr);
        sql.push_str(" AS ");
        sql.push_str(cast);
        sql.push_str(" )");
    } else {
        sql.push_str(&expr);
    }
    if let Some(alias) = output_alias(field) {
        sql.push_str(" AS ");
        sql.push_str(&quote_ident(alias));
    }
}

/// Alias PostgREST adds when the client did not (`Plan.hs` `addAliases`).
///
/// A JSON path uses the last key. An index uses the previous key, or the
/// column name when the path has none. An explicit alias wins.
fn output_alias(field: &SelectField) -> Option<&str> {
    if let Some(alias) = &field.alias {
        return Some(alias);
    }
    let last = field.json.last()?;
    match json_operand(last) {
        JsonOperand::Key(key) => Some(key),
        JsonOperand::Index(_) => Some(
            field
                .json
                .iter()
                .rev()
                .find_map(|step| match json_operand(step) {
                    JsonOperand::Key(key) => Some(key.as_str()),
                    JsonOperand::Index(_) => None,
                })
                .unwrap_or(field.column.as_str()),
        ),
    }
}

fn json_operand(step: &JsonStep) -> &JsonOperand {
    match step {
        JsonStep::Arrow(operand) | JsonStep::TwoArrow(operand) => operand,
    }
}

fn push_json(sql: &mut String, params: &mut Vec<String>, steps: &[JsonStep]) {
    for step in steps {
        let (arrow, operand) = match step {
            JsonStep::Arrow(operand) => ("->", operand),
            JsonStep::TwoArrow(operand) => ("->>", operand),
        };
        sql.push_str(arrow);
        match operand {
            JsonOperand::Key(key) => push_param(sql, params, key),
            JsonOperand::Index(index) => {
                push_param(sql, params, index);
                sql.push_str("::int");
            }
        }
    }
}

/// `WHERE` fragment for a mutation, without the `WHERE` keyword.
///
/// Placeholders start at `$1`. `None` means the client sent no filter.
///
/// # Errors
///
/// Returns [`UnsafeType`] when a filter's catalog type cannot be spliced.
pub(crate) fn where_clause(
    relation: &str,
    query: &ReadQuery,
    column_types: &[(&str, &str)],
) -> Result<Option<ReadSql>, UnsafeType> {
    if query.filters.is_empty() && query.logic.is_empty() {
        return Ok(None);
    }
    let mut sql = String::new();
    let mut params = Vec::new();
    push_where(
        &mut sql,
        &mut params,
        relation,
        &query.filters,
        &query.logic,
        column_types,
    )?;
    Ok(Some(ReadSql {
        sql,
        params,
        offset: 0,
    }))
}

fn push_where(
    sql: &mut String,
    params: &mut Vec<String>,
    relation: &str,
    filters: &[(String, ParsedFilter)],
    logic: &[LogicNode],
    column_types: &[(&str, &str)],
) -> Result<(), UnsafeType> {
    let mut wrote = false;
    for (column, parsed) in filters {
        if wrote {
            sql.push_str(" AND ");
        }
        let bound = bound_filter(column, parsed, lookup_type(column_types, column));
        append_predicate(sql, params, relation, &bound)?;
        wrote = true;
    }
    for node in logic {
        if wrote {
            sql.push_str(" AND ");
        }
        // megabase:unit rest:query-param:and
        // megabase:unit rest:query-param:or
        push_logic(sql, params, relation, node, column_types)?;
        wrote = true;
    }
    let _ = wrote;
    Ok(())
}

fn push_logic(
    sql: &mut String,
    params: &mut Vec<String>,
    relation: &str,
    node: &LogicNode,
    column_types: &[(&str, &str)],
) -> Result<(), UnsafeType> {
    match node {
        LogicNode::Expr {
            negated,
            op,
            children,
        } => {
            if *negated {
                sql.push_str("NOT ");
            }
            sql.push('(');
            for (index, child) in children.iter().enumerate() {
                if index > 0 {
                    sql.push_str(match op {
                        LogicOp::And => " AND ",
                        LogicOp::Or => " OR ",
                    });
                }
                push_logic(sql, params, relation, child, column_types)?;
            }
            sql.push(')');
            Ok(())
        }
        LogicNode::Filter { column, parsed } => {
            let bound = bound_filter(column, parsed, lookup_type(column_types, column));
            append_predicate(sql, params, relation, &bound)
        }
        LogicNode::Json => Ok(()),
    }
}

fn push_order(sql: &mut String, params: &mut Vec<String>, relation: &str, orders: &[OrderTerm]) {
    sql.push_str("ORDER BY ");
    let mut wrote = false;
    for term in orders {
        let OrderTerm::Column {
            column,
            json,
            direction,
            nulls,
        } = term
        else {
            continue;
        };
        if wrote {
            sql.push_str(", ");
        }
        sql.push_str(relation);
        sql.push('.');
        sql.push_str(&quote_ident(column));
        push_json(sql, params, json);
        if let Some(direction) = direction {
            sql.push(' ');
            sql.push_str(match direction {
                Direction::Asc => "ASC",
                Direction::Desc => "DESC",
            });
        }
        if let Some(nulls) = nulls {
            sql.push(' ');
            sql.push_str(match nulls {
                Nulls::First => "NULLS FIRST",
                Nulls::Last => "NULLS LAST",
            });
        }
        wrote = true;
    }
    let _ = wrote;
}

fn push_page(sql: &mut String, params: &mut Vec<String>, page: &Page) {
    sql.push_str("LIMIT ");
    match &page.limit {
        Some(limit) => {
            sql.push('(');
            push_param(sql, params, limit);
            sql.push_str("::bigint)");
        }
        None => sql.push_str("ALL"),
    }
    sql.push_str(" OFFSET (");
    push_param(sql, params, &page.offset);
    sql.push_str("::bigint)");
}

fn push_param(sql: &mut String, params: &mut Vec<String>, value: &str) {
    params.push(value.to_string());
    let index = params.len();
    sql.push('$');
    sql.push_str(&index.to_string());
}

fn lookup_type(column_types: &[(&str, &str)], column: &str) -> Option<String> {
    column_types
        .iter()
        .find(|(name, _)| *name == column)
        .map(|(_, pg_type)| (*pg_type).to_string())
}

fn bound_filter(column: &str, parsed: &ParsedFilter, pg_type: Option<String>) -> BoundFilter {
    let (negated, body) = match parsed {
        ParsedFilter::Served {
            negated,
            op,
            quant,
            language,
            value,
        } => (
            *negated,
            FilterBody::Op {
                op: *op,
                quant: *quant,
                language: language.clone(),
                value: value.clone(),
            },
        ),
        ParsedFilter::In { negated, values } => (*negated, FilterBody::In(values.clone())),
        ParsedFilter::Is { negated, value } => (*negated, FilterBody::Is(*value)),
        ParsedFilter::IsDistinct { negated, value } => {
            (*negated, FilterBody::IsDistinct(value.clone()))
        }
    };
    BoundFilter {
        column: column.to_string(),
        negated,
        body,
        pg_type,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order_err(input: &str) -> QueryFail {
        parse_order_value(input).unwrap_err()
    }

    fn parse_msg(input: &str, column: usize, unexpected: &str, expecting: &str) -> QueryFail {
        QueryFail::Parse {
            message: format!("\"failed to parse order ({input})\" (line 1, column {column})"),
            details: format!("unexpected {unexpected} expecting {expecting}"),
        }
    }

    #[test]
    fn order_keywords_and_error_positions() {
        let query = parse_get_query("order=name.desc.nullsfirst").unwrap();
        match &query.orders[0] {
            OrderTerm::Column {
                column,
                direction,
                nulls,
                ..
            } => {
                assert_eq!(column, "name");
                assert_eq!(*direction, Some(Direction::Desc));
                assert_eq!(*nulls, Some(Nulls::First));
            }
            OrderTerm::Related { .. } => panic!("column term"),
        }
        assert_eq!(
            order_err("id.ac"),
            parse_msg(
                "id.ac",
                4,
                "\"c\"",
                "\"asc\", \"desc\", \"nullsfirst\" or \"nullslast\""
            )
        );
        assert_eq!(
            order_err("id.descc"),
            parse_msg("id.descc", 8, "'c'", "delimiter (.), \",\" or end of input")
        );
        assert_eq!(
            order_err("id.nulsfist"),
            parse_msg(
                "id.nulsfist",
                4,
                "\"n\"",
                "\"asc\", \"desc\", \"nullsfirst\" or \"nullslast\""
            )
        );
        assert_eq!(
            order_err("id.nullslasttt"),
            parse_msg("id.nullslasttt", 13, "'t'", "\",\" or end of input")
        );
        assert_eq!(
            order_err("id.smth34"),
            parse_msg(
                "id.smth34",
                4,
                "\"s\"",
                "\"asc\", \"desc\", \"nullsfirst\" or \"nullslast\""
            )
        );
        assert_eq!(
            order_err("id.asc.nlsfst"),
            parse_msg(
                "id.asc.nlsfst",
                8,
                "\"l\"",
                "\"nullsfirst\" or \"nullslast\""
            )
        );
        assert_eq!(
            order_err("id.asc.nullslasttt"),
            parse_msg("id.asc.nullslasttt", 17, "'t'", "\",\" or end of input")
        );
        assert_eq!(
            order_err("id.asc.smth34"),
            parse_msg(
                "id.asc.smth34",
                8,
                "\"s\"",
                "\"nullsfirst\" or \"nullslast\""
            )
        );
    }

    #[test]
    fn order_parse_failure_beats_a_later_filter() {
        let err = parse_get_query("order=id.ac&id=nope").unwrap_err();
        assert!(matches!(err, QueryFail::Parse { ref message, .. } if message.contains("order")));
    }

    #[test]
    fn select_projects_and_defers_embeds() {
        let query = parse_get_query("select=id,title").unwrap();
        match &query.select {
            SelectList::Fields(fields) => {
                assert_eq!(fields[0].column, "id");
                assert_eq!(fields[1].column, "title");
            }
            _ => panic!("fields"),
        }
        assert!(matches!(
            parse_get_query("select=notes(body)").unwrap_err(),
            QueryFail::Route
        ));
        assert!(matches!(
            parse_get_query("select=notes!inner(body)").unwrap_err(),
            QueryFail::Unimplemented("rest:embed-join:inner")
        ));
        assert!(matches!(
            parse_get_query("select=id.count()").unwrap_err(),
            QueryFail::Unimplemented("rest:aggregate:count")
        ));
        let err = parse_get_query("select=notes(body)&id=nope").unwrap_err();
        assert!(matches!(err, QueryFail::Parse { .. }));
    }

    #[test]
    fn limit_offset_pairing_and_last_wins() {
        let page = page_from(
            &[
                ("limit".into(), "1".into()),
                ("limit".into(), "nope".into()),
            ],
            &[],
        );
        assert!(!page.page.active);
        let page = page_from(
            &[("limit".into(), "1".into())],
            &[("offset".into(), "2".into())],
        );
        assert!(page.page.active);
        assert_eq!(page.page.limit.as_deref(), Some("1"));
        assert_eq!(page.page.offset, "2");
        let page = page_from(&[], &[("offset".into(), "0".into())]);
        assert!(!page.page.active);
        let page = page_from(&[("items.limit".into(), "1".into())], &[]);
        assert_eq!(page.embed.as_deref(), Some("items"));
        let err = parse_get_query("select=id&items.order=id").unwrap_err();
        assert_eq!(
            err,
            QueryFail::NotEmbedded {
                resource: "items".into()
            }
        );
    }

    #[test]
    fn logic_or_and_not_and_columns() {
        let query = parse_get_query("or=(id.eq.1,id.eq.3)&order=id").unwrap();
        match &query.logic[0] {
            LogicNode::Expr {
                negated: false,
                op: LogicOp::Or,
                children,
            } => assert_eq!(children.len(), 2),
            other => panic!("{other:?}"),
        }
        let query = parse_get_query("and=(done.eq.true,priority.gte.1)").unwrap();
        assert!(matches!(
            &query.logic[0],
            LogicNode::Expr {
                op: LogicOp::And,
                ..
            }
        ));
        let query = parse_get_query("not.or=(id.eq.1,id.eq.2)").unwrap();
        assert!(matches!(
            &query.logic[0],
            LogicNode::Expr {
                negated: true,
                op: LogicOp::Or,
                ..
            }
        ));
        let query = parse_get_query("columns=id,title&on_conflict=id&select=*").unwrap();
        assert_eq!(
            query.columns,
            Some(vec!["id".to_string(), "title".to_string()])
        );
        assert_eq!(query.on_conflict, Some(vec!["id".to_string()]));
        assert!(matches!(
            parse_get_query("columns=").unwrap_err(),
            QueryFail::Parse { .. }
        ));
        assert!(matches!(
            parse_get_query("on_conflict=").unwrap_err(),
            QueryFail::Parse { .. }
        ));
    }

    #[test]
    fn sql_orders_limits_and_binds_filter_values() {
        let query = parse_get_query("select=id,title&order=id.desc&limit=1&offset=2").unwrap();
        let sql = build_read_sql("todos", &query, &[]).unwrap();
        assert!(sql.sql.contains("\"todos\".\"id\", \"todos\".\"title\""));
        assert!(sql.sql.contains("ORDER BY \"todos\".\"id\" DESC"));
        assert!(sql.sql.contains("LIMIT ($1::bigint) OFFSET ($2::bigint)"));
        assert_eq!(sql.params, vec!["1", "2"]);
        assert_eq!(sql.offset, 2);

        let query = parse_get_query("or=(id.eq.1,id.eq.3)&order=id").unwrap();
        let sql = build_read_sql("todos", &query, &[]).unwrap();
        assert!(sql.sql.contains(" OR "));
        assert_eq!(sql.params, vec!["1", "3"]);
        assert!(!sql.sql.contains("drop"));

        let query = parse_get_query("columns=id,title&order=id").unwrap();
        let sql = build_read_sql("todos", &query, &[]).unwrap();
        assert!(sql.sql.contains("\"todos\".*"));
        assert!(!sql.sql.contains("\"todos\".\"id\","));

        let query = parse_get_query("body=eq.victim-secret') or true--&select=body").unwrap();
        let sql = build_read_sql("notes", &query, &[("body", "text")]).unwrap();
        assert_eq!(sql.params, vec!["victim-secret') or true--"]);
        assert!(!sql.sql.contains("victim-secret"));
    }

    #[test]
    fn related_order_is_not_embedded() {
        let err = parse_get_query("order=clients(id)").unwrap_err();
        assert_eq!(
            err,
            QueryFail::NotEmbedded {
                resource: "clients".into()
            }
        );
    }

    #[test]
    fn bare_pairs_are_required() {
        assert!(!parse_get_query("").unwrap().handled());
        assert!(!parse_get_query("select").unwrap().handled());
    }

    #[test]
    fn on_conflict_parser_is_used() {
        let names = parse_on_conflict_names("id").unwrap();
        assert_eq!(names, vec!["id".to_string()]);
    }

    #[test]
    fn json_select_uses_the_last_key_as_the_alias() {
        let query =
            parse_get_query("select=data->a,data->>b::text,data->1,data->1->mycol->>2").unwrap();
        let sql = build_read_sql("todos", &query, &[]).unwrap();
        assert!(sql.sql.contains("\"todos\".\"data\"->$1 AS \"a\""));
        assert!(sql
            .sql
            .contains("CAST( \"todos\".\"data\"->>$2 AS text ) AS \"b\""));
        assert!(sql.sql.contains("\"todos\".\"data\"->$3::int AS \"data\""));
        assert!(sql
            .sql
            .contains("\"todos\".\"data\"->$4::int->$5->>$6::int AS \"mycol\""));
        assert_eq!(sql.params, vec!["a", "b", "+1", "+1", "mycol", "+2"]);
    }
}
