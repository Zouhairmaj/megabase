// Ported from postgrest src/library/PostgREST/ApiRequest/QueryParams.hs (MIT), pin v16.4,
// and postgrest src/library/PostgREST/Query/SqlFragment.hs (MIT), pin v16.4.

//! `GET` query parameters from `QueryParams.hs`.
//!
//! `parse` tries `order`, logic (`and` / `or`), `columns`, `select`, filters,
//! then `on_conflict`. The first failure wins. `columns` and `on_conflict` are
//! accepted on a read and do not change the row shape.

use std::collections::HashMap;

use crate::filter::{
    append_predicate, parse_filter_value, quote_ident, BoundFilter, FilterBody, FilterParseError,
    IsVal, ParsedFilter, ServedOp, UnsafeType,
};
use crate::query::{percent_decode, query_parts};

/// Why a query string is not a served read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum QueryFail {
    /// `PGRST100` from the Parsec parsers.
    Parse { message: String, details: String },
    /// `PGRST108` for an `order`, `limit`, `offset`, `and`, or `or` on a resource that is not embedded.
    NotEmbedded { resource: String },
    /// An embed or spread shape, or a filter key, this crate does not serve.
    /// The route unit stays 501.
    Route,
}

/// One `key=op.value` filter (`Filter` in `ApiRequest/Types.hs`): the
/// column, its JSON path (`payload->user->>name`), and the operator.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldFilter {
    column: String,
    json: Vec<JsonStep>,
    parsed: ParsedFilter,
}

/// One successful `GET` query string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadQuery {
    select: SelectList,
    orders: Vec<OrderTerm>,
    page: Page,
    logic: Vec<LogicNode>,
    filters: Vec<FieldFilter>,
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

    /// Names of the embedded resources the select list asks for.
    #[must_use]
    pub(crate) fn embed_names(&self) -> Vec<&str> {
        self.embeds().map(|embed| embed.name.as_str()).collect()
    }

    fn embeds(&self) -> impl Iterator<Item = &Embed> {
        let entries = match &self.select {
            SelectList::Mixed(entries) => entries.as_slice(),
            SelectList::Star | SelectList::Fields(_) => &[],
        };
        entries.iter().filter_map(|entry| match entry {
            SelectEntry::Embed(embed) => Some(embed),
            SelectEntry::Field(_) => None,
        })
    }

    /// `true` when a root filter or logic tree is `<key>=is.null` (or
    /// `not.is.null`) on the embed called `key`.
    fn has_null_embed_filter(&self, key: &str) -> bool {
        self.filters.iter().any(|filter| {
            filter.json.is_empty() && filter.column == key && is_null_filter(&filter.parsed)
        }) || self.logic.iter().any(|node| logic_names_null(node, key))
    }

    /// The embeds the select list asks for: `(target, hint, relationship key)`.
    ///
    /// The key is the target, or `target!hint` for a hinted embed; the
    /// relationship map passed to [`build_read_sql_as`] uses it.
    #[must_use]
    pub(crate) fn embed_targets(&self) -> Vec<(&str, Option<&str>, String)> {
        match &self.select {
            SelectList::Mixed(entries) => entries
                .iter()
                .filter_map(|entry| match entry {
                    SelectEntry::Embed(embed) => {
                        Some((embed.name.as_str(), embed.hint.as_deref(), embed.key()))
                    }
                    SelectEntry::Field(_) => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// `true` when `select` applies an aggregate function.
    /// `hasDefaultSelect` in `Plan.hs`: the select is a lone `*` (or absent)
    /// with no embed, so a relation-targeted custom media handler applies.
    #[must_use]
    pub(crate) fn is_default_select(&self) -> bool {
        matches!(self.select, SelectList::Star)
    }

    #[must_use]
    pub(crate) fn has_aggregate(&self) -> bool {
        match &self.select {
            SelectList::Star => false,
            SelectList::Fields(fields) => fields.iter().any(|field| field.agg.is_some()),
            SelectList::Mixed(entries) => entries
                .iter()
                .any(|entry| matches!(entry, SelectEntry::Field(field) if field.agg.is_some())),
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

    /// `on_conflict` columns, when the client sent them.
    #[must_use]
    pub(crate) fn conflict_columns(&self) -> Option<&[String]> {
        self.on_conflict.as_deref()
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
        for filter in &self.filters {
            match &filter.parsed {
                ParsedFilter::Served {
                    negated: false,
                    op: ServedOp::Eq,
                    quant: None,
                    language: None,
                    value,
                } if filter.json.is_empty() => out.push((filter.column.clone(), value.clone())),
                _ => return None,
            }
        }
        Some(out)
    }

    /// `SELECT … FROM source AS relation [ORDER BY …]` for a mutation's
    /// `return=representation` body. Values are appended to `params`.
    pub(crate) fn representation_select(
        &self,
        source: &str,
        relation: &str,
        params: &mut Vec<String>,
    ) -> String {
        let relation_ident = quote_ident(relation);
        let mut select_sql = String::new();
        push_select(&mut select_sql, params, &relation_ident, &self.select);
        let mut sql = format!("SELECT {select_sql} FROM {source} AS {relation_ident}");
        if !self.orders.is_empty() {
            sql.push(' ');
            push_order(&mut sql, params, &relation_ident, &self.orders);
        }
        sql
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
    /// A select list that carries at least one embedded resource.
    Mixed(Vec<SelectEntry>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SelectEntry {
    Field(SelectField),
    Embed(Embed),
}

/// One `name(...)`, `name!inner(...)` or `name!left(...)` select item.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Embed {
    alias: Option<String>,
    name: String,
    /// `selHint`: the `!constraint` or `!column` after the target.
    hint: Option<String>,
    inner: bool,
    select: SelectList,
    params: EmbedParams,
}

impl Embed {
    /// `updateNode` with legacy target names (the default): the name or the alias.
    fn targeted_by(&self, target: &str) -> bool {
        self.name == target || self.alias.as_deref() == Some(target)
    }

    /// The key a null embed filter (`books=is.null`) names: the alias, else the name.
    fn output_key(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.name)
    }
}

/// Filters, logic trees, order and page of one embedded resource
/// (`books.published=eq.true`, `books.order=title.desc`, `books.limit=1`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct EmbedParams {
    filters: Vec<FieldFilter>,
    logic: Vec<LogicNode>,
    orders: Vec<OrderTerm>,
    page: Page,
    /// A parameter named the relation of an aliased embed (`books.x` for
    /// `b:books(...)`). Upstream answers with a deprecation `Warning`
    /// header (`App.hs` `legacyWarnings`), which is not served.
    legacy_target: bool,
}

impl EmbedParams {
    fn none() -> Self {
        Self {
            filters: Vec::new(),
            logic: Vec::new(),
            orders: Vec::new(),
            page: Page::inactive(),
            legacy_target: false,
        }
    }
}

impl Embed {
    /// Relationship map key: a field name never holds `!`, so `name!hint`
    /// cannot collide with an unhinted target.
    fn key(&self) -> String {
        match &self.hint {
            Some(hint) => format!("{}!{hint}", self.name),
            None => self.name.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectField {
    column: String,
    alias: Option<String>,
    json: Vec<JsonStep>,
    cast: Option<String>,
    agg: Option<Aggregate>,
    agg_cast: Option<String>,
}

/// `AggregateFunction` in `ApiRequest/Types.hs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Aggregate {
    Sum,
    Avg,
    Count,
    Max,
    Min,
}

impl Aggregate {
    fn sql(self) -> &'static str {
        match self {
            Self::Sum => "SUM",
            Self::Avg => "AVG",
            Self::Count => "COUNT",
            Self::Max => "MAX",
            Self::Min => "MIN",
        }
    }
}

/// One `->` or `->>` step of a JSON path (`JsonOperation` in `ApiRequest/Types.hs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum JsonStep {
    Arrow(JsonOperand),
    TwoArrow(JsonOperand),
}

/// The key or array index after a JSON arrow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum JsonOperand {
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
    Filter(FieldFilter),
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
    for (path, expr) in &collected.logics {
        logic.push((path.clone(), parse_logic_tree(expr)?));
    }
    let columns = match &collected.columns {
        Some(value) => Some(parse_name_list("columns parameter", value)?),
        None => None,
    };
    let select_raw = collected.select.as_deref().unwrap_or("*");
    let select = parse_select_value(select_raw)?;
    let mut filters = Vec::new();
    let mut embed_filters = Vec::new();
    let mut route = false;
    for (key, value) in &collected.filters {
        let parsed = parse_filter_value(value).map_err(filter_fail)?;
        match filter_path(key) {
            Some((path, column, json)) => {
                let filter = FieldFilter {
                    column,
                    json,
                    parsed,
                };
                if path.is_empty() {
                    filters.push(filter);
                } else {
                    embed_filters.push((path, filter));
                }
            }
            // An empty key or segment, or a quoted name: the route unit stays 501.
            None => route = true,
        }
    }
    let on_conflict = match &collected.on_conflict {
        Some(value) => Some(parse_on_conflict_names(value)?),
        None => None,
    };
    if let ParsedSelect::Deferred(fail) = select {
        return Err(fail);
    }
    if route {
        return Err(QueryFail::Route);
    }
    let ParsedSelect::Ready(mut select) = select else {
        return Err(QueryFail::Route);
    };
    // `readPlan` adds filters, then orders, ranges and logic trees. Each kind
    // is a `foldr`, so the rightmost parameter is placed (and fails) first.
    for (path, filter) in embed_filters.into_iter().rev() {
        embed_mut(&mut select, &path)?
            .params
            .filters
            .insert(0, filter);
    }
    let mut root_orders = None;
    for (path, terms) in orders.into_iter().rev() {
        // Placed right to left, so the leftmost list for one target wins.
        if path.is_empty() {
            root_orders = Some(terms);
        } else {
            embed_mut(&mut select, &path)?.params.orders = terms;
        }
    }
    let orders = root_orders.unwrap_or_default();
    let (page, embed_pages) = pages_from(&collected.limits, &collected.offsets);
    for (path, embed_page) in embed_pages.into_iter().rev() {
        embed_mut(&mut select, &path)?.params.page = embed_page;
    }
    let mut root_logic = Vec::new();
    for (path, node) in logic.into_iter().rev() {
        if path.is_empty() {
            root_logic.insert(0, node);
        } else {
            embed_mut(&mut select, &path)?.params.logic.insert(0, node);
        }
    }
    let related = orders.iter().find_map(|term| match term {
        OrderTerm::Related { relation } => Some(relation.clone()),
        OrderTerm::Column { .. } => None,
    });
    if let Some(resource) = related {
        // A related order on an embedded resource is not served: 501.
        let embedded = matches!(&select, SelectList::Mixed(entries) if entries.iter().any(|entry|
            matches!(entry, SelectEntry::Embed(embed) if embed.targeted_by(&resource))));
        return Err(if embedded {
            QueryFail::Route
        } else {
            QueryFail::NotEmbedded { resource }
        });
    }
    // Nested embeds are not served, so a related order inside an embed is
    // `PGRST108` upstream only after the relationship lookup, and a legacy
    // target name adds a `Warning` header: keep both 501.
    if let SelectList::Mixed(entries) = &select {
        let unserved = entries.iter().any(|entry| {
            matches!(entry, SelectEntry::Embed(embed) if embed.params.legacy_target
                || embed.params.orders.iter().any(|term|
                    matches!(term, OrderTerm::Related { .. })))
        });
        if unserved {
            return Err(QueryFail::Route);
        }
    }
    Ok(ReadQuery {
        select,
        orders,
        page,
        logic: root_logic,
        filters,
        columns,
        on_conflict,
        handled: true,
    })
}

/// `pTreePath` for a filter key: the embed path, the column, and its JSON path.
///
/// `None` for a key this crate does not split: empty, an empty segment, a
/// quoted name, or a JSON path with text after it.
fn filter_path(key: &str) -> Option<(Vec<String>, String, Vec<JsonStep>)> {
    if key.contains('"') {
        return None;
    }
    if key.contains("->") {
        return json_filter_path(key);
    }
    let mut parts: Vec<String> = key.split('.').map(str::to_string).collect();
    if parts.iter().any(String::is_empty) {
        return None;
    }
    let column = parts.pop()?;
    Some((parts, column, Vec::new()))
}

/// `pFieldName `sepBy1` pDelimiter` then `pJsonPath`, for a key with `->`.
fn json_filter_path(key: &str) -> Option<(Vec<String>, String, Vec<JsonStep>)> {
    let mut at = At::start(key);
    let mut names = Vec::new();
    loop {
        let (name, next) = parse_field_name(&at).ok()?;
        names.push(name);
        at = next;
        if at.peek() != Some('.') {
            break;
        }
        at = at.bump('.');
    }
    let json = parse_json_path(&mut at).ok()?;
    if json.is_empty() || !at.eof() {
        return None;
    }
    let column = names.pop()?;
    Some((names, column, json))
}

/// The embed a non-root parameter targets (`updateNode` in `Plan.hs`, with
/// the default legacy target names: the embed's name or its alias).
///
/// # Errors
///
/// `PGRST108` when no embed in `select` matches. Nested embeds are not
/// served, so a second path segment is never an embedded resource.
fn embed_mut<'s>(select: &'s mut SelectList, path: &[String]) -> Result<&'s mut Embed, QueryFail> {
    let Some((first, rest)) = path.split_first() else {
        return Err(QueryFail::Route);
    };
    let not_embedded = |resource: &String| QueryFail::NotEmbedded {
        resource: resource.clone(),
    };
    let SelectList::Mixed(entries) = select else {
        return Err(not_embedded(first));
    };
    let embed = entries
        .iter_mut()
        .find_map(|entry| match entry {
            SelectEntry::Embed(embed) if embed.targeted_by(first) => Some(embed),
            SelectEntry::Embed(_) | SelectEntry::Field(_) => None,
        })
        .ok_or_else(|| not_embedded(first))?;
    if embed.name == *first && embed.alias.as_ref().is_some_and(|alias| alias != first) {
        embed.params.legacy_target = true;
    }
    match rest.first() {
        Some(next) => Err(not_embedded(next)),
        None => Ok(embed),
    }
}

fn filter_fail(error: FilterParseError) -> QueryFail {
    QueryFail::Parse {
        message: error.message,
        details: error.details,
    }
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
    for part in query_parts(query) {
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

/// `limit` / `offset` pairs, keyed by the path before `.limit` / `.offset`.
#[derive(Default)]
struct PageSlot {
    limit: Option<String>,
    offset: Option<String>,
}

impl PageSlot {
    fn page(self) -> Page {
        match (self.limit, self.offset) {
            (None, None) => Page::inactive(),
            (None, Some(offset)) if offset == "0" => Page::inactive(),
            (limit, offset) => Page {
                limit,
                offset: offset.unwrap_or_else(|| "0".to_string()),
                active: true,
            },
        }
    }
}

/// The root page and one page per embed path (`qsRanges`).
///
/// A repeated key keeps its last value (`HM.fromList`). Embed paths are in
/// the order limits then offsets were last seen, so the caller places them
/// right to left and reports the rightmost unknown embed, as `foldr` does.
fn pages_from(
    limits: &[(String, String)],
    offsets: &[(String, String)],
) -> (Page, Vec<(Vec<String>, Page)>) {
    let mut root = PageSlot::default();
    let mut slots: Vec<(Vec<String>, PageSlot)> = Vec::new();
    let keyed = limits
        .iter()
        .map(|(key, value)| (key, value, true))
        .chain(offsets.iter().map(|(key, value)| (key, value, false)));
    for (key, value, is_limit) in keyed {
        let path = embed_path(key);
        let slot = if path.is_empty() {
            &mut root
        } else {
            let entry = match slots.iter().position(|(seen, _)| *seen == path) {
                Some(at) => slots.remove(at),
                None => (path, PageSlot::default()),
            };
            slots.push(entry);
            match slots.last_mut() {
                Some((_, slot)) => slot,
                None => continue,
            }
        };
        let parsed = read_integer(value);
        if is_limit {
            slot.limit = parsed;
        } else {
            slot.offset = parsed;
        }
    }
    let embeds = slots
        .into_iter()
        .map(|(path, slot)| (path, slot.page()))
        .collect();
    (root.page(), embeds)
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

/// PostgREST echoes the value without its `not.` / `and` / `or` prefix.
fn logic_echo(input: &str) -> &str {
    let rest = input.strip_prefix("not.").unwrap_or(input);
    rest.strip_prefix("and")
        .or_else(|| rest.strip_prefix("or"))
        .unwrap_or(rest)
}

fn parse_logic_tree(input: &str) -> Result<LogicNode, QueryFail> {
    let echo = logic_echo(input);
    let mut at = At::start(input);
    let node = logic_tree(&mut at).map_err(|err| named_error("logic tree", echo, &err))?;
    let at = skip_ws(&at);
    if !at.eof() {
        let err = char_err(&at, "end of input");
        return Err(named_error("logic tree", echo, &err));
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
    let mut entries = Vec::new();
    let mut has_embed = false;
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
            SelectItem::Field(field) => {
                entries.push(SelectEntry::Field(field.clone()));
                fields.push(field);
            }
            SelectItem::Embed(embed) => {
                has_embed = true;
                entries.push(SelectEntry::Embed(embed));
            }
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
    if has_embed {
        return Ok(ParsedSelect::Ready(SelectList::Mixed(entries)));
    }
    if fields.is_empty() {
        return Ok(ParsedSelect::Ready(SelectList::Star));
    }
    if fields.len() == 1
        && fields[0].column == "*"
        && fields[0].alias.is_none()
        && fields[0].json.is_empty()
        && fields[0].cast.is_none()
        && fields[0].agg.is_none()
    {
        return Ok(ParsedSelect::Ready(SelectList::Star));
    }
    Ok(ParsedSelect::Ready(SelectList::Fields(fields)))
}

enum SelectItem {
    Field(SelectField),
    Embed(Embed),
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
    let (_join, _hint) = parse_embed_params(at)?;
    if at.peek() != Some('(') {
        return Err(char_err(at, "\"(\""));
    }
    skip_balanced(at)?;
    Ok(SelectItem::Defer(QueryFail::Route))
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
    let (join, hint) = match parse_embed_params(at) {
        Ok(parsed) => parsed,
        Err(_) => {
            *at = alias_saved;
            return Ok(None);
        }
    };
    if at.peek() != Some('(') {
        *at = alias_saved;
        return Ok(None);
    }
    let open = at.byte + 1;
    skip_balanced(at)?;
    let inner_text = &at.input[open..at.byte - 1];
    // An empty embed (`notes()`) means "filter only" upstream; not served yet.
    if inner_text.trim().is_empty() {
        return Ok(Some(SelectItem::Defer(QueryFail::Route)));
    }
    let select = match parse_select(inner_text)? {
        ParsedSelect::Ready(select) if !matches!(select, SelectList::Mixed(_)) => select,
        ParsedSelect::Ready(_) => return Ok(Some(SelectItem::Defer(QueryFail::Route))),
        ParsedSelect::Deferred(fail) => return Ok(Some(SelectItem::Defer(fail))),
    };
    Ok(Some(SelectItem::Embed(Embed {
        alias,
        name,
        hint,
        inner: matches!(join, Some(JoinKind::Inner)),
        select,
        params: EmbedParams::none(),
    })))
}

#[derive(Clone, Copy)]
enum JoinKind {
    Inner,
    Left,
}

/// `pEmbedParams`: up to two `!` params; the first hint and the first join
/// type win (`embedParamHint prm1 <|> embedParamHint prm2`).
fn parse_embed_params<'a>(at: &mut At<'a>) -> Result<(Option<JoinKind>, Option<String>), PErr> {
    let mut join = None;
    let mut hint = None;
    for _ in 0..2 {
        if at.peek() != Some('!') {
            break;
        }
        *at = at.bump('!');
        if at.starts_with("left") {
            *at = bump_str(at, "left");
            join = join.or(Some(JoinKind::Left));
        } else if at.starts_with("inner") {
            *at = bump_str(at, "inner");
            join = join.or(Some(JoinKind::Inner));
        } else {
            let (name, next) = parse_field_name(at)?;
            *at = next;
            hint = hint.or(Some(name));
        }
    }
    Ok((join, hint))
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
                agg: None,
                agg_cast: None,
            }));
        }
        return Err(char_err(&next, "\")\", \",\" or end of input"));
    }
    let alias = optional_alias(at)?;
    // megabase:unit rest:aggregate:count
    if at.starts_with("count()") {
        *at = bump_str(at, "count()");
        let agg_cast = optional_cast(at)?;
        if !matches!(at.peek(), None | Some(',' | ')')) {
            return Err(char_err(at, "\")\", \",\" or end of input"));
        }
        return Ok(SelectItem::Field(SelectField {
            column: "*".to_string(),
            alias,
            json: Vec::new(),
            cast: None,
            agg: Some(Aggregate::Count),
            agg_cast,
        }));
    }
    let (name, next) = parse_field_name(at)?;
    *at = next;
    let json = parse_json_path(at)?;
    let cast = optional_cast(at)?;
    let mut agg = None;
    if at.peek() == Some('.') {
        let after = at.bump('.');
        if let Some((token, function)) = aggregate_function(&after) {
            *at = bump_str(&after, token);
            agg = Some(function);
        }
    }
    let agg_cast = if agg.is_some() {
        optional_cast(at)?
    } else {
        None
    };
    if !matches!(at.peek(), None | Some(',' | ')')) {
        return Err(char_err(at, "\")\", \",\" or end of input"));
    }
    Ok(SelectItem::Field(SelectField {
        column: name,
        alias,
        json,
        cast,
        agg,
        agg_cast,
    }))
}

fn aggregate_function<'a>(at: &At<'a>) -> Option<(&'static str, Aggregate)> {
    // megabase:unit rest:aggregate:sum
    // megabase:unit rest:aggregate:avg
    // megabase:unit rest:aggregate:max
    // megabase:unit rest:aggregate:min
    for (token, function) in [
        ("sum()", Aggregate::Sum),
        ("avg()", Aggregate::Avg),
        ("count()", Aggregate::Count),
        ("max()", Aggregate::Max),
        ("min()", Aggregate::Min),
    ] {
        if at.starts_with(token) {
            return Some((token, function));
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
                Err(expr_err) => {
                    // Both alternatives failed at the same spot: PostgREST quotes with `show`.
                    let same_spot =
                        (filter_err.line, filter_err.col) == (expr_err.line, expr_err.col);
                    let unexpected = expr_err.unexpected.clone();
                    let mut merged = merge(filter_err, expr_err);
                    if same_spot {
                        merged.unexpected = unexpected;
                    }
                    Err(merged)
                }
            }
        }
    }
}

fn logic_expr<'a>(at: &mut At<'a>) -> Result<LogicNode, PErr> {
    // megabase:unit rest:logic-operator:not
    let negated = if at.starts_with("not.") {
        *at = bump_str(at, "not.");
        true
    } else {
        false
    };
    let op = if at.starts_with("and") {
        // megabase:unit rest:logic-operator:and
        *at = bump_str(at, "and");
        LogicOp::And
    } else if at.starts_with("or") {
        // megabase:unit rest:logic-operator:or
        *at = bump_str(at, "or");
        LogicOp::Or
    } else {
        return Err(PErr {
            line: at.line,
            col: at.col,
            unexpected: at.peek().map(show_string_char).unwrap_or_default(),
            expects: if negated {
                vec!["logic operator (and, or)".to_string()]
            } else {
                vec![
                    "negation operator (not)".to_string(),
                    "logic operator (and, or)".to_string(),
                ]
            },
        });
    };
    at_skip(at);
    if at.peek() != Some('(') {
        return Err(string_char_err(at, &["\"(\""]));
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
        return Err(string_char_err(at, &["\",\"", "\")\""]));
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
    let json = parse_json_path(at)?;
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
    Ok(LogicNode::Filter(FieldFilter {
        column: name,
        json,
        parsed,
    }))
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

/// Like `char_err`, but the unexpected character is quoted as a Haskell string char.
fn string_char_err<'a>(at: &At<'a>, expects: &[&str]) -> PErr {
    PErr {
        line: at.line,
        col: at.col,
        unexpected: at.peek().map(show_string_char).unwrap_or_default(),
        expects: expects.iter().map(|item| (*item).to_string()).collect(),
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

/// Build the JSON read statement for `query` in `schema`.
///
/// `column_types` is `(name, format_type)` in `attnum` order. Values, JSON
/// keys, and limit/offset integers are bound parameters. The `FROM` target
/// is `"schema"."relation"`.
///
/// # Errors
///
/// Returns [`UnsafeType`] when a filter's catalog type cannot be spliced.
#[cfg(test)]
pub(crate) fn build_read_sql(
    schema: &str,
    relation: &str,
    query: &ReadQuery,
    column_types: &[(&str, &str)],
    rels: &HashMap<String, Relationship>,
) -> Result<ReadSql, UnsafeType> {
    build_read_sql_as(
        schema,
        relation,
        query,
        column_types,
        rels,
        BodyHandler::Json,
    )
}

/// The builtin media handler that turns the result rows into a body
/// (`handlerF` in `SqlFragment.hs`, `initialMediaHandlers` in `SchemaCache.hs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyHandler {
    /// `asJsonF`: a JSON array of row objects.
    Json,
    /// `asCsvF`: the key row of the first result row, a newline, then each
    /// row's record text without its parentheses, joined by newlines.
    Csv,
    /// `asGeoJsonF`: a `FeatureCollection` whose features are
    /// `ST_AsGeoJSON(row)`. The function comes from PostGIS; without it, or
    /// without a geometry column, Postgres raises the error PostgREST
    /// forwards.
    GeoJson,
    /// `NoAgg`: the constant `''::text`. `HEAD` runs no handler upstream
    /// (`negotiateContent`, `handlerF`), so the source query is still run
    /// and counted but no body is built, whatever the negotiated type.
    NoAgg,
}

/// [`build_read_sql`] with the body built by `handler`.
///
/// # Errors
///
/// Returns [`UnsafeType`] when a filter's catalog type cannot be spliced.
pub(crate) fn build_read_sql_as(
    schema: &str,
    relation: &str,
    query: &ReadQuery,
    column_types: &[(&str, &str)],
    rels: &HashMap<String, Relationship>,
    handler: BodyHandler,
) -> Result<ReadSql, UnsafeType> {
    let mut params = Vec::new();
    let relation_ident = quote_ident(relation);
    let schema_ident = quote_ident(schema);
    let mut select_sql = String::new();
    // megabase:unit rest:query-param:select
    let select = expand_star(&query.select, column_types);
    push_select(&mut select_sql, &mut params, &relation_ident, &select);
    let mut inner = format!("SELECT {select_sql} FROM {schema_ident}.{relation_ident}");
    let mut exists = Vec::new();
    let mut null_embeds = Vec::new();
    if let SelectList::Mixed(entries) = &query.select {
        let mut select_sql = String::new();
        for (index, entry) in entries.iter().enumerate() {
            if index > 0 {
                select_sql.push_str(", ");
            }
            match entry {
                SelectEntry::Field(field) => {
                    push_select_field(&mut select_sql, &mut params, &relation_ident, field);
                }
                SelectEntry::Embed(embed) => {
                    let Some(rel) = rels.get(&embed.key()) else {
                        continue;
                    };
                    let rows = embed_rows(
                        &mut params,
                        &schema_ident,
                        &relation_ident,
                        embed,
                        rel,
                        true,
                    )?;
                    push_embed(&mut select_sql, &rows, embed, rel);
                    // `INNER JOIN LATERAL … ON agg IS NOT NULL`: the parent
                    // needs a row in the filtered, ordered and limited embed.
                    if embed.inner {
                        exists.push(format!("EXISTS ({rows})"));
                    }
                    null_embeds.push((embed.output_key().to_string(), rows));
                }
            }
        }
        inner = format!("SELECT {select_sql} FROM {schema_ident}.{relation_ident}");
    }
    if !query.filters.is_empty() || !query.logic.is_empty() || !exists.is_empty() {
        inner.push_str(" WHERE ");
        push_where(
            &mut inner,
            &mut params,
            relation,
            &query.filters,
            &query.logic,
            column_types,
            &null_embeds,
        )?;
        if !exists.is_empty() {
            if !query.filters.is_empty() || !query.logic.is_empty() {
                inner.push_str(" AND ");
            }
            inner.push_str(&exists.join(" AND "));
        }
    }
    push_group(&mut inner, &mut params, &relation_ident, &select);
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
    let sql = match handler {
        BodyHandler::Csv => format!(
            "WITH _megabase_src AS MATERIALIZED ({inner}) \
             SELECT (SELECT pg_catalog.count(*) FROM _megabase_src), \
             (SELECT coalesce(string_agg(a.k, ','), '') FROM \
             (SELECT json_object_keys(r)::text AS k FROM \
             (SELECT row_to_json(hh) AS r FROM _megabase_src AS hh LIMIT 1) s) a) \
             || E'\\n' || \
             coalesce((SELECT string_agg(x.line, E'\\n' ORDER BY x.rn) FROM \
             (SELECT substring(t::text, 2, length(t::text) - 2) AS line, \
             row_number() OVER () AS rn FROM _megabase_src AS t) x), '')"
        ),
        // `asGeoJsonF`. The row alias is a plain subquery record as upstream
        // (`FROM ( SELECT * FROM pgrst_source ) _postgrest_t`), so an absent
        // PostGIS raises the same `st_asgeojson(record)` error on both stacks.
        BodyHandler::GeoJson => format!(
            "WITH _megabase_src AS MATERIALIZED ({inner}) \
             SELECT (SELECT pg_catalog.count(*) FROM _megabase_src), \
             (SELECT json_build_object('type', 'FeatureCollection', 'features', \
             coalesce(json_agg(x.feature ORDER BY x.rn), '[]'))::text FROM \
             (SELECT ST_AsGeoJSON(_postgrest_t)::json AS feature, \
             row_number() OVER () AS rn FROM _megabase_src AS _postgrest_t) x)"
        ),
        BodyHandler::Json => format!(
            "WITH _megabase_src AS MATERIALIZED ({inner}) \
             SELECT (SELECT pg_catalog.count(*) FROM _megabase_src), \
             coalesce((SELECT json_agg(t.row ORDER BY t.rn) FROM \
             (SELECT to_json(_megabase_src) AS row, row_number() OVER () AS rn \
             FROM _megabase_src) t), '[]'::json)::text"
        ),
        // `mainRead` with `NoAgg`: the source CTE still runs (so `select`,
        // filters and paging are validated and counted); the body is constant.
        BodyHandler::NoAgg => format!(
            "WITH _megabase_src AS MATERIALIZED ({inner}) \
             SELECT (SELECT pg_catalog.count(*) FROM _megabase_src), ''::text"
        ),
    };
    let offset = query.page.offset.parse::<i64>().unwrap_or(0);
    let _ = (&query.columns, &query.on_conflict);
    Ok(ReadSql {
        sql,
        params,
        offset,
    })
}

/// Foreign-key link between the requested table and an embedded one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Relationship {
    /// Embedded table. The embed name may be a key column or constraint instead.
    pub table: String,
    /// `true` for a one-to-many link (an array result).
    pub to_many: bool,
    /// Columns on the requested table, in key order.
    pub root_columns: Vec<String>,
    /// Columns on the embedded table, in key order.
    pub target_columns: Vec<String>,
    /// Column names and types of the embedded table, for its filters. The
    /// relationship lookup leaves it empty; the caller loads the catalog.
    pub column_types: Vec<(String, String)>,
}

fn join_condition(rel: &Relationship, root: &str, target: &str) -> String {
    rel.root_columns
        .iter()
        .zip(&rel.target_columns)
        .map(|(r, t)| format!("{target}.{} = {root}.{}", quote_ident(t), quote_ident(r)))
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// The embedded rows of one parent row: the join condition, then the embed's
/// own filters and logic trees (`readPlanToQuery` for a child node).
///
/// `full` adds the select list, order and page. Without it the query is
/// `SELECT 1 …` as `readPlanToCountQuery` builds it, which ignores the
/// embed's order and range.
///
/// # Errors
///
/// Returns [`UnsafeType`] when a filter's catalog type cannot be spliced.
fn embed_rows(
    params: &mut Vec<String>,
    schema: &str,
    root: &str,
    embed: &Embed,
    rel: &Relationship,
    full: bool,
) -> Result<String, UnsafeType> {
    let target = quote_ident(&rel.table);
    let mut sql = String::from("SELECT ");
    if full {
        push_select(&mut sql, params, &target, &embed.select);
    } else {
        sql.push('1');
    }
    sql.push_str(&format!(
        " FROM {schema}.{target} WHERE {}",
        join_condition(rel, root, &target)
    ));
    let own = &embed.params;
    if !own.filters.is_empty() || !own.logic.is_empty() {
        let column_types: Vec<(&str, &str)> = rel
            .column_types
            .iter()
            .map(|(name, pg_type)| (name.as_str(), pg_type.as_str()))
            .collect();
        sql.push_str(" AND ");
        push_where(
            &mut sql,
            params,
            &rel.table,
            &own.filters,
            &own.logic,
            &column_types,
            &[],
        )?;
    }
    if full && !own.orders.is_empty() {
        sql.push(' ');
        push_order(&mut sql, params, &target, &own.orders);
    }
    if full && own.page.active {
        sql.push(' ');
        push_page(&mut sql, params, &own.page);
    }
    Ok(sql)
}

/// Correlated subquery for one embed: an array for a to-many link, an object or `null` otherwise.
fn push_embed(sql: &mut String, rows: &str, embed: &Embed, rel: &Relationship) {
    if rel.to_many {
        sql.push_str(&format!(
            "COALESCE((SELECT json_agg(_t) FROM ({rows}) _t), '[]'::json)"
        ));
    } else {
        sql.push_str(&format!("(SELECT to_json(_t) FROM ({rows}) _t)"));
    }
    sql.push_str(" AS ");
    sql.push_str(&quote_ident(embed.output_key()));
}

/// `WHERE` fragment for `Prefer: count`, without the `WHERE` keyword
/// (`readPlanToCountQuery`): the root filters, with a null embed filter as
/// `[NOT] EXISTS`, and an `EXISTS` per `!inner` embed. Embedded order and
/// range do not apply. `None` when nothing restricts the parent rows.
///
/// # Errors
///
/// Returns [`UnsafeType`] when a filter's catalog type cannot be spliced.
pub(crate) fn count_where(
    schema: &str,
    relation: &str,
    query: &ReadQuery,
    column_types: &[(&str, &str)],
    rels: &HashMap<String, Relationship>,
) -> Result<Option<ReadSql>, UnsafeType> {
    let schema_ident = quote_ident(schema);
    let relation_ident = quote_ident(relation);
    let mut params = Vec::new();
    let mut exists = Vec::new();
    let mut null_embeds = Vec::new();
    for embed in query.embeds() {
        let Some(rel) = rels.get(&embed.key()) else {
            continue;
        };
        let named = query.has_null_embed_filter(embed.output_key());
        if !embed.inner && !named {
            continue;
        }
        let rows = embed_rows(
            &mut params,
            &schema_ident,
            &relation_ident,
            embed,
            rel,
            false,
        )?;
        if embed.inner {
            exists.push(format!("EXISTS ({rows})"));
        }
        if named {
            null_embeds.push((embed.output_key().to_string(), rows));
        }
    }
    let filtered = !query.filters.is_empty() || !query.logic.is_empty();
    if !filtered && exists.is_empty() {
        return Ok(None);
    }
    let mut sql = String::new();
    if filtered {
        push_where(
            &mut sql,
            &mut params,
            relation,
            &query.filters,
            &query.logic,
            column_types,
            &null_embeds,
        )?;
    }
    if !exists.is_empty() {
        if filtered {
            sql.push_str(" AND ");
        }
        sql.push_str(&exists.join(" AND "));
    }
    Ok(Some(ReadSql {
        sql,
        params,
        offset: 0,
    }))
}

fn is_null_filter(parsed: &ParsedFilter) -> bool {
    matches!(
        parsed,
        ParsedFilter::Is {
            value: IsVal::Null,
            ..
        }
    )
}

fn logic_names_null(node: &LogicNode, key: &str) -> bool {
    match node {
        LogicNode::Expr { children, .. } => {
            children.iter().any(|child| logic_names_null(child, key))
        }
        LogicNode::Filter(filter) => {
            filter.json.is_empty() && filter.column == key && is_null_filter(&filter.parsed)
        }
    }
}

/// `CoercibleFilterNullEmbed` (`addNullEmbedFilters` in `Plan.hs`):
/// `<embed>=is.null` keeps the parent rows with no embedded row (an
/// anti-join); `<embed>=not.is.null` keeps the others. `rows` is the
/// embed's subquery; `None` when the filter is an ordinary column filter.
fn null_embed_predicate(null_embeds: &[(String, String)], filter: &FieldFilter) -> Option<String> {
    let ParsedFilter::Is {
        negated,
        value: IsVal::Null,
    } = &filter.parsed
    else {
        return None;
    };
    if !filter.json.is_empty() {
        return None;
    }
    let (_, rows) = null_embeds.iter().find(|(key, _)| *key == filter.column)?;
    Some(if *negated {
        format!("EXISTS ({rows})")
    } else {
        format!("NOT EXISTS ({rows})")
    })
}

/// `*` next to an aggregate becomes the table's columns, so `GROUP BY` can
/// name them (`Plan.hs` expands it the same way).
fn expand_star(select: &SelectList, column_types: &[(&str, &str)]) -> SelectList {
    let SelectList::Fields(fields) = select else {
        return select.clone();
    };
    let aggregated = fields.iter().any(|field| field.agg.is_some());
    if !aggregated || column_types.is_empty() {
        return select.clone();
    }
    let mut expanded = Vec::with_capacity(fields.len());
    for field in fields {
        if field.column == "*" && field.agg.is_none() {
            for (name, _) in column_types {
                expanded.push(SelectField {
                    column: (*name).to_string(),
                    alias: None,
                    json: Vec::new(),
                    cast: None,
                    agg: None,
                    agg_cast: None,
                });
            }
        } else {
            expanded.push(field.clone());
        }
    }
    SelectList::Fields(expanded)
}

/// `GROUP BY` over the non-aggregated fields (`SqlFragment.hs` `groupF`).
fn push_group(sql: &mut String, params: &mut Vec<String>, relation: &str, select: &SelectList) {
    let SelectList::Fields(fields) = select else {
        return;
    };
    if fields.iter().all(|field| field.agg.is_none()) {
        return;
    }
    let mut first = true;
    for field in fields.iter().filter(|field| field.agg.is_none()) {
        sql.push_str(if first { " GROUP BY " } else { ", " });
        first = false;
        if let Some(alias) = output_alias(field) {
            sql.push_str(&quote_ident(alias));
        } else {
            sql.push_str(relation);
            sql.push('.');
            sql.push_str(&quote_ident(&field.column));
            push_json(sql, params, &field.json);
        }
    }
}

fn push_select(sql: &mut String, params: &mut Vec<String>, relation: &str, select: &SelectList) {
    match select {
        // Embeds are rendered by `build_read_sql`.
        SelectList::Mixed(_) => {}
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
        expr = format!("CAST( {expr} AS {cast} )");
    }
    if let Some(agg) = field.agg {
        expr = format!("{}({expr})", agg.sql());
        if let Some(cast) = &field.agg_cast {
            expr = format!("CAST( {expr} AS {cast} )");
        }
    }
    sql.push_str(&expr);
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
    if field.agg.is_some() {
        return None;
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

/// `pgFmtJsonPath`: each key and index is a bound parameter.
pub(crate) fn push_json(sql: &mut String, params: &mut Vec<String>, steps: &[JsonStep]) {
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
        &[],
    )?;
    Ok(Some(ReadSql {
        sql,
        params,
        offset: 0,
    }))
}

/// `null_embeds` pairs an embed key with its rows subquery, for
/// [`null_embed_predicate`].
fn push_where(
    sql: &mut String,
    params: &mut Vec<String>,
    relation: &str,
    filters: &[FieldFilter],
    logic: &[LogicNode],
    column_types: &[(&str, &str)],
    null_embeds: &[(String, String)],
) -> Result<(), UnsafeType> {
    let mut wrote = false;
    for filter in filters {
        if wrote {
            sql.push_str(" AND ");
        }
        if let Some(predicate) = null_embed_predicate(null_embeds, filter) {
            sql.push_str(&predicate);
        } else {
            let bound = bound_filter(filter, column_types);
            append_predicate(sql, params, relation, &bound)?;
        }
        wrote = true;
    }
    for node in logic {
        if wrote {
            sql.push_str(" AND ");
        }
        // megabase:unit rest:query-param:and
        // megabase:unit rest:query-param:or
        push_logic(sql, params, relation, node, column_types, null_embeds)?;
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
    null_embeds: &[(String, String)],
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
                push_logic(sql, params, relation, child, column_types, null_embeds)?;
            }
            sql.push(')');
            Ok(())
        }
        LogicNode::Filter(filter) => {
            if let Some(predicate) = null_embed_predicate(null_embeds, filter) {
                sql.push_str(&predicate);
                return Ok(());
            }
            let bound = bound_filter(filter, column_types);
            append_predicate(sql, params, relation, &bound)
        }
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
                // megabase:unit rest:order-modifier:asc
                Direction::Asc => "ASC",
                // megabase:unit rest:order-modifier:desc
                Direction::Desc => "DESC",
            });
        }
        if let Some(nulls) = nulls {
            sql.push(' ');
            sql.push_str(match nulls {
                // megabase:unit rest:order-modifier:nullsfirst
                Nulls::First => "NULLS FIRST",
                // megabase:unit rest:order-modifier:nullslast
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

/// The filter with its catalog type (`resolveQueryInputField` in `Plan.hs`).
///
/// A JSON path applies to the column as is when it is `json` or `jsonb`, and
/// to `to_jsonb(column)` otherwise, even when the column is missing. The
/// value then takes the type of the path: `text` after `->>`, the column's
/// `json` or `jsonb` after `->`.
fn bound_filter(filter: &FieldFilter, column_types: &[(&str, &str)]) -> BoundFilter {
    let column_type = lookup_type(column_types, &filter.column);
    let is_json = matches!(column_type.as_deref(), Some("json" | "jsonb"));
    let pg_type = match filter.json.last() {
        None => column_type,
        Some(JsonStep::TwoArrow(_)) => Some("text".to_string()),
        Some(JsonStep::Arrow(_)) if column_type.as_deref() == Some("json") => {
            Some("json".to_string())
        }
        Some(JsonStep::Arrow(_)) => Some("jsonb".to_string()),
    };
    let (negated, body) = match &filter.parsed {
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
        column: filter.column.clone(),
        json: filter.json.clone(),
        to_json: !filter.json.is_empty() && !is_json,
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
    fn order_modifiers_render_sql() {
        for (input, expected) in [
            ("a.asc", "ORDER BY r.\"a\" ASC"),
            ("a.desc", "ORDER BY r.\"a\" DESC"),
            ("a.nullsfirst", "ORDER BY r.\"a\" NULLS FIRST"),
            ("a.nullslast", "ORDER BY r.\"a\" NULLS LAST"),
            (
                "a.desc.nullslast,b.asc.nullsfirst",
                "ORDER BY r.\"a\" DESC NULLS LAST, r.\"b\" ASC NULLS FIRST",
            ),
        ] {
            let query = parse_get_query(&format!("order={input}")).unwrap();
            let mut sql = String::new();
            push_order(&mut sql, &mut Vec::new(), "r", &query.orders);
            assert_eq!(sql, expected);
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
        let query = parse_get_query("select=id,notes(body)").unwrap();
        assert_eq!(query.embed_names(), vec!["notes"]);
        let query = parse_get_query("select=notes!inner(body)").unwrap();
        assert_eq!(query.embed_names(), vec!["notes"]);
        assert!(matches!(
            parse_get_query("select=...notes!fk(body)").unwrap_err(),
            QueryFail::Route
        ));
        for empty in [
            "select=id,notes()",
            "select=notes!inner()",
            "select=notes( )",
        ] {
            assert!(matches!(
                parse_get_query(empty).unwrap_err(),
                QueryFail::Route
            ));
        }
        assert!(matches!(
            parse_get_query("select=notes(a(b))").unwrap_err(),
            QueryFail::Route
        ));
        assert!(parse_get_query("select=id.count()")
            .unwrap()
            .has_aggregate());
        let err = parse_get_query("select=notes(body)&id=nope").unwrap_err();
        assert!(matches!(err, QueryFail::Parse { .. }));
    }

    #[test]
    fn embed_sql_joins_on_the_foreign_key() {
        let query = parse_get_query("select=name,notes!inner(body)").unwrap();
        let rels = HashMap::from([(
            "notes".to_string(),
            Relationship {
                table: "notes".into(),
                to_many: true,
                root_columns: vec!["id".into()],
                target_columns: vec!["user_id".into()],
                column_types: Vec::new(),
            },
        )]);
        let sql = build_read_sql("public", "profiles", &query, &[], &rels)
            .unwrap()
            .sql;
        assert!(sql.contains("json_agg(_t)"), "{sql}");
        assert!(
            sql.contains("\"notes\".\"user_id\" = \"profiles\".\"id\""),
            "{sql}"
        );
        assert!(
            sql.contains("AND EXISTS") || sql.contains("WHERE EXISTS"),
            "{sql}"
        );
    }

    #[test]
    fn embed_hints_parse_in_either_order() {
        for (select, hint) in [
            (
                "select=users!posts_author_id_fkey(name)",
                "posts_author_id_fkey",
            ),
            ("select=users!editor_id!inner(name)", "editor_id"),
            ("select=users!inner!editor_id(name)", "editor_id"),
            // `pEmbedParams` keeps the first hint.
            ("select=users!editor_id!author_id(name)", "editor_id"),
        ] {
            let query = parse_get_query(select).unwrap();
            assert_eq!(
                query.embed_targets(),
                vec![("users", Some(hint), format!("users!{hint}"))],
                "{select}"
            );
        }
        let query = parse_get_query("select=users!left!inner(name)").unwrap();
        let SelectList::Mixed(entries) = &query.select else {
            panic!("mixed");
        };
        let SelectEntry::Embed(embed) = &entries[0] else {
            panic!("embed");
        };
        // The first join type wins too.
        assert!(!embed.inner);
        assert_eq!(embed.hint, None);
    }

    #[test]
    fn two_hinted_embeds_of_one_table_use_their_own_keys() {
        let query = parse_get_query(
            "select=title,author:users!author_id(name),editor:users!editor_id(name)",
        )
        .unwrap();
        let rel = |column: &str| Relationship {
            table: "users".into(),
            to_many: false,
            root_columns: vec![column.into()],
            target_columns: vec!["id".into()],
            column_types: Vec::new(),
        };
        let rels = HashMap::from([
            ("users!author_id".to_string(), rel("author_id")),
            ("users!editor_id".to_string(), rel("editor_id")),
        ]);
        let sql = build_read_sql("public", "posts", &query, &[], &rels)
            .unwrap()
            .sql;
        assert!(
            sql.contains("WHERE \"users\".\"id\" = \"posts\".\"author_id\") _t) AS \"author\""),
            "{sql}"
        );
        assert!(
            sql.contains("WHERE \"users\".\"id\" = \"posts\".\"editor_id\") _t) AS \"editor\""),
            "{sql}"
        );
    }

    #[test]
    fn column_hint_embed_reads_the_referenced_table() {
        let query = parse_get_query("select=*,author:user_id!inner(name)").unwrap();
        assert_eq!(query.embed_names(), vec!["user_id"]);
        let rels = HashMap::from([(
            "user_id".to_string(),
            Relationship {
                table: "users".into(),
                to_many: false,
                root_columns: vec!["user_id".into()],
                target_columns: vec!["id".into()],
                column_types: Vec::new(),
            },
        )]);
        let sql = build_read_sql("public", "messages", &query, &[], &rels)
            .unwrap()
            .sql;
        assert!(
            sql.contains(
                "(SELECT to_json(_t) FROM (SELECT \"users\".\"name\" FROM \"public\".\"users\" \
                 WHERE \"users\".\"id\" = \"messages\".\"user_id\") _t) AS \"author\""
            ),
            "{sql}"
        );
        assert!(
            sql.contains("WHERE EXISTS (SELECT \"users\".\"name\" FROM \"public\".\"users\" WHERE"),
            "{sql}"
        );
        assert!(!sql.contains("\"user_id\" AS"), "{sql}");
    }

    #[test]
    fn limit_offset_pairing_and_last_wins() {
        let (page, _) = pages_from(
            &[
                ("limit".into(), "1".into()),
                ("limit".into(), "nope".into()),
            ],
            &[],
        );
        assert!(!page.active);
        let (page, _) = pages_from(
            &[("limit".into(), "1".into())],
            &[("offset".into(), "2".into())],
        );
        assert!(page.active);
        assert_eq!(page.limit.as_deref(), Some("1"));
        assert_eq!(page.offset, "2");
        let (page, _) = pages_from(&[], &[("offset".into(), "0".into())]);
        assert!(!page.active);
        let (page, embeds) = pages_from(
            &[
                ("items.limit".into(), "1".into()),
                ("tags.limit".into(), "3".into()),
            ],
            &[("items.offset".into(), "2".into())],
        );
        assert!(!page.active);
        // `items` was seen last, so it is placed (and fails) first.
        let paths: Vec<_> = embeds.iter().map(|(path, _)| path.join(".")).collect();
        assert_eq!(paths, ["tags", "items"]);
        assert_eq!(embeds[1].1.limit.as_deref(), Some("1"));
        assert_eq!(embeds[1].1.offset, "2");
        let err = parse_get_query("select=id&items.limit=1&tags.limit=2").unwrap_err();
        assert_eq!(
            err,
            QueryFail::NotEmbedded {
                resource: "tags".into()
            }
        );
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
    fn logic_nested_trees_bind_values_in_order() {
        let query =
            parse_get_query("or=(id.eq.1,and(done.eq.true,not.or(id.eq.2,id.eq.3)))").unwrap();
        let sql = build_read_sql("public", "todos", &query, &[], &HashMap::new()).unwrap();
        // No catalog types are passed, so leaves are bare `col = $n`.
        assert!(sql.sql.contains(concat!(
            r#" WHERE ("todos"."id" = $1 OR ("todos"."done" = $2 "#,
            r#"AND NOT ("todos"."id" = $3 OR "todos"."id" = $4)))"#,
        )));
        assert_eq!(sql.params, vec!["1", "true", "2", "3"]);
        assert!(matches!(
            parse_get_query("or=()").unwrap_err(),
            QueryFail::Parse { .. }
        ));
        assert!(matches!(
            parse_get_query("and=(id.eq.1").unwrap_err(),
            QueryFail::Parse { .. }
        ));
    }

    #[test]
    fn logic_leaf_not_and_negated_groups_render_sql() {
        let query = parse_get_query("or=(id.not.eq.1,and(priority.gt.1,done.is.null))").unwrap();
        let sql = build_read_sql("public", "todos", &query, &[], &HashMap::new()).unwrap();
        assert!(
            sql.sql.contains(concat!(
                r#" WHERE (NOT "todos"."id" = $1 OR ("todos"."priority" > $2 "#,
                r#"AND "todos"."done" IS NULL))"#,
            )),
            "{}",
            sql.sql
        );
        assert_eq!(sql.params, vec!["1", "1"]);

        let query = parse_get_query("not.and=(id.eq.1,id.eq.2)").unwrap();
        let sql = build_read_sql("public", "todos", &query, &[], &HashMap::new()).unwrap();
        assert!(sql.sql.contains(" WHERE NOT ("), "{}", sql.sql);
        assert_eq!(sql.params, vec!["1", "2"]);

        for bad in ["or=(id.eq.1,)", "not.or=()", "or=(id.nope.1)"] {
            assert!(
                matches!(parse_get_query(bad).unwrap_err(), QueryFail::Parse { .. }),
                "{bad}"
            );
        }
    }

    #[test]
    fn logic_parse_errors_match_postgrest_text() {
        let cases = [
            (
                "or=()",
                "\"failed to parse logic tree (())\" (line 1, column 4)",
                "unexpected \")\" expecting field name (* or [a..z0..9_$]), negation operator (not) or logic operator (and, or)",
            ),
            (
                "and=(id.eq.1",
                "\"failed to parse logic tree ((id.eq.1)\" (line 1, column 12)",
                "unexpected end of input expecting \",\" or \")\"",
            ),
        ];
        for (input, want_message, want_details) in cases {
            match parse_get_query(input).unwrap_err() {
                QueryFail::Parse { message, details } => {
                    assert_eq!(message, want_message, "{input}");
                    assert_eq!(details, want_details, "{input}");
                }
                other => panic!("{input}: {other:?}"),
            }
        }
    }

    #[test]
    fn sql_orders_limits_and_binds_filter_values() {
        let query = parse_get_query("select=id,title&order=id.desc&limit=1&offset=2").unwrap();
        let sql = build_read_sql("public", "todos", &query, &[], &HashMap::new()).unwrap();
        assert!(sql.sql.contains("\"todos\".\"id\", \"todos\".\"title\""));
        assert!(sql.sql.contains("ORDER BY \"todos\".\"id\" DESC"));
        assert!(sql.sql.contains("LIMIT ($1::bigint) OFFSET ($2::bigint)"));
        assert_eq!(sql.params, vec!["1", "2"]);
        assert_eq!(sql.offset, 2);

        let query = parse_get_query("or=(id.eq.1,id.eq.3)&order=id").unwrap();
        let sql = build_read_sql("public", "todos", &query, &[], &HashMap::new()).unwrap();
        assert!(sql.sql.contains(" OR "));
        assert_eq!(sql.params, vec!["1", "3"]);
        assert!(!sql.sql.contains("drop"));

        let query = parse_get_query("columns=id,title&order=id").unwrap();
        let sql = build_read_sql("public", "todos", &query, &[], &HashMap::new()).unwrap();
        assert!(sql.sql.contains("\"todos\".*"));
        assert!(!sql.sql.contains("\"todos\".\"id\","));

        let query = parse_get_query("body=eq.victim-secret') or true--&select=body").unwrap();
        let sql = build_read_sql(
            "public",
            "notes",
            &query,
            &[("body", "text")],
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(sql.params, vec!["victim-secret') or true--"]);
        assert!(!sql.sql.contains("victim-secret"));
    }

    #[test]
    fn csv_read_uses_key_row_and_record_text() {
        let query = parse_get_query("id=eq.1").unwrap();
        let sql = build_read_sql_as(
            "public",
            "todos",
            &query,
            &[("id", "integer")],
            &HashMap::new(),
            BodyHandler::Csv,
        )
        .unwrap();
        assert!(sql.sql.contains("json_object_keys(r)"));
        assert!(sql
            .sql
            .contains("substring(t::text, 2, length(t::text) - 2)"));
        assert!(!sql.sql.contains("json_agg"));
        assert_eq!(sql.params, vec!["1"]);
    }

    #[test]
    fn geo_json_read_wraps_st_asgeojson_in_a_feature_collection() {
        let query = parse_get_query("id=eq.1&order=id").unwrap();
        let sql = build_read_sql_as(
            "public",
            "todos",
            &query,
            &[("id", "integer")],
            &HashMap::new(),
            BodyHandler::GeoJson,
        )
        .unwrap();
        assert!(sql
            .sql
            .contains("json_build_object('type', 'FeatureCollection', 'features'"));
        assert!(sql
            .sql
            .contains("ST_AsGeoJSON(_postgrest_t)::json AS feature"));
        assert!(sql
            .sql
            .contains("coalesce(json_agg(x.feature ORDER BY x.rn), '[]')"));
        assert!(!sql.sql.contains("to_json(_megabase_src)"));
        assert_eq!(sql.params, vec!["1"]);
    }

    #[test]
    fn head_read_counts_the_source_without_a_body_aggregate() {
        let query = parse_get_query("select=id&id=eq.1&limit=5").unwrap();
        let sql = build_read_sql_as(
            "public",
            "todos",
            &query,
            &[("id", "integer")],
            &HashMap::new(),
            BodyHandler::NoAgg,
        )
        .unwrap();
        assert!(sql.sql.ends_with("''::text"), "{}", sql.sql);
        assert!(sql.sql.contains("pg_catalog.count(*) FROM _megabase_src"));
        assert!(sql.sql.contains("LIMIT"));
        assert!(!sql.sql.contains("json_agg"));
        assert!(!sql.sql.contains("ST_AsGeoJSON"));
        assert!(!sql.sql.contains("string_agg"));
        // The filter value, then the page's limit and offset, are bound.
        assert_eq!(sql.params, vec!["1", "5", "0"]);
    }

    #[test]
    fn default_select_is_a_lone_star_without_embeds() {
        assert!(parse_get_query("").unwrap().is_default_select());
        assert!(parse_get_query("select=*&id=eq.1")
            .unwrap()
            .is_default_select());
        assert!(!parse_get_query("select=id").unwrap().is_default_select());
        assert!(!parse_get_query("select=*,todos(*)")
            .unwrap()
            .is_default_select());
        assert!(!parse_get_query("select=count()")
            .unwrap()
            .is_default_select());
    }

    #[test]
    fn aggregates_build_functions_casts_and_group_by() {
        let query =
            parse_get_query("select=count(),n:amount.sum()::text,kind&amount=gt.1").unwrap();
        assert!(query.has_aggregate());
        let sql = build_read_sql(
            "public",
            "t",
            &query,
            &[("amount", "integer")],
            &HashMap::new(),
        )
        .unwrap();
        assert!(sql.sql.contains(
            "SELECT COUNT(\"t\".*), CAST( SUM(\"t\".\"amount\") AS text ) AS \"n\", \"t\".\"kind\" \
             FROM \"public\".\"t\" WHERE"
        ));
        assert!(sql.sql.contains(" GROUP BY \"t\".\"kind\""));

        let query =
            parse_get_query("select=a.avg(),b.max(),c.min(),k:d::int.sum()::bigint").unwrap();
        let sql = build_read_sql("public", "t", &query, &[], &HashMap::new()).unwrap();
        assert!(sql.sql.contains(
            "AVG(\"t\".\"a\"), MAX(\"t\".\"b\"), MIN(\"t\".\"c\"), \
             CAST( SUM(CAST( \"t\".\"d\" AS int )) AS bigint ) AS \"k\""
        ));
        assert!(!sql.sql.contains("GROUP BY"));

        let query = parse_get_query("select=*,id.count()").unwrap();
        let sql = build_read_sql(
            "public",
            "t",
            &query,
            &[("id", "integer"), ("v", "text")],
            &HashMap::new(),
        )
        .unwrap();
        assert!(sql.sql.contains("GROUP BY \"t\".\"id\", \"t\".\"v\""));
        assert!(matches!(
            parse_get_query("select=id.count()x").unwrap_err(),
            QueryFail::Parse { .. }
        ));
    }

    #[test]
    fn embedded_related_order_is_deferred() {
        let err = parse_get_query("select=id,notes(body)&notes.order=users(id)").unwrap_err();
        assert_eq!(err, QueryFail::Route);
    }

    fn books_rel() -> HashMap<String, Relationship> {
        HashMap::from([(
            "books".to_string(),
            Relationship {
                table: "books".into(),
                to_many: true,
                root_columns: vec!["id".into()],
                target_columns: vec!["author_id".into()],
                column_types: vec![
                    ("title".into(), "text".into()),
                    ("published".into(), "boolean".into()),
                ],
            },
        )])
    }

    #[test]
    fn embedded_filters_order_and_page_render_in_the_subquery() {
        let query = parse_get_query(
            "select=name,books(title)&books.published=eq.true&books.order=title.desc\
             &books.limit=1&books.offset=2&books.or=(title.eq.a,title.eq.b)&order=name",
        )
        .unwrap();
        let sql = build_read_sql("public", "authors", &query, &[], &books_rel()).unwrap();
        assert!(
            sql.sql.contains(
                "COALESCE((SELECT json_agg(_t) FROM (SELECT \"books\".\"title\" \
                 FROM \"public\".\"books\" WHERE \"books\".\"author_id\" = \"authors\".\"id\" \
                 AND \"books\".\"published\" = ($1::text)::boolean \
                 AND (\"books\".\"title\" = ($2::text)::text \
                 OR \"books\".\"title\" = ($3::text)::text) \
                 ORDER BY \"books\".\"title\" DESC LIMIT ($4::bigint) OFFSET ($5::bigint)) _t), \
                 '[]'::json) AS \"books\""
            ),
            "{}",
            sql.sql
        );
        // A left embed keeps every parent: no `WHERE` on `authors`.
        assert!(
            sql.sql.contains("FROM \"public\".\"authors\" ORDER BY"),
            "{}",
            sql.sql
        );
        assert_eq!(sql.params, ["true", "a", "b", "1", "2"]);
        // An embedded page is not the root page.
        assert!(!query.limits_rows());
    }

    #[test]
    fn inner_embed_filters_parents_with_the_same_subquery() {
        let query =
            parse_get_query("select=name,books!inner(title)&books.published=eq.true").unwrap();
        let sql = build_read_sql("public", "authors", &query, &[], &books_rel()).unwrap();
        let rows = "SELECT \"books\".\"title\" FROM \"public\".\"books\" \
                    WHERE \"books\".\"author_id\" = \"authors\".\"id\" \
                    AND \"books\".\"published\" = ($1::text)::boolean";
        assert!(
            sql.sql.contains(&format!("WHERE EXISTS ({rows})")),
            "{}",
            sql.sql
        );
        assert_eq!(sql.params, ["true"]);
        let count = count_where("public", "authors", &query, &[], &books_rel())
            .unwrap()
            .unwrap();
        assert_eq!(
            count.sql,
            "EXISTS (SELECT 1 FROM \"public\".\"books\" \
             WHERE \"books\".\"author_id\" = \"authors\".\"id\" \
             AND \"books\".\"published\" = ($1::text)::boolean)"
        );
        assert_eq!(count.params, ["true"]);
    }

    #[test]
    fn null_embed_filter_is_an_anti_join() {
        let query = parse_get_query("select=name,books(title)&books=is.null&name=eq.x").unwrap();
        let sql = build_read_sql("public", "authors", &query, &[], &books_rel()).unwrap();
        assert!(
            sql.sql.contains(
                "WHERE NOT EXISTS (SELECT \"books\".\"title\" FROM \"public\".\"books\" \
                 WHERE \"books\".\"author_id\" = \"authors\".\"id\") AND \"authors\".\"name\" = $1"
            ),
            "{}",
            sql.sql
        );
        let query =
            parse_get_query("select=name,b:books(title)&or=(b.not.is.null,name.eq.x)").unwrap();
        let rels = HashMap::from([("books".to_string(), books_rel().remove("books").unwrap())]);
        let sql = build_read_sql("public", "authors", &query, &[], &rels).unwrap();
        assert!(
            sql.sql
                .contains("WHERE (EXISTS (SELECT \"books\".\"title\""),
            "{}",
            sql.sql
        );
        let count = count_where("public", "authors", &query, &[], &rels)
            .unwrap()
            .unwrap();
        assert!(
            count
                .sql
                .starts_with("(EXISTS (SELECT 1 FROM \"public\".\"books\""),
            "{}",
            count.sql
        );
        // `is.null` on a column that is not an embed stays a column filter.
        let query = parse_get_query("select=name,books(title)&name=is.null").unwrap();
        let count = count_where("public", "authors", &query, &[], &books_rel())
            .unwrap()
            .unwrap();
        assert_eq!(count.sql, "\"authors\".\"name\" IS NULL");
    }

    fn hinted_books_rel() -> HashMap<String, Relationship> {
        HashMap::from([(
            "books!author_id".to_string(),
            books_rel().remove("books").unwrap(),
        )])
    }

    #[test]
    fn count_finds_a_hinted_inner_embed() {
        let query = parse_get_query("select=name,books!author_id!inner(title)").unwrap();
        let count = count_where("public", "authors", &query, &[], &hinted_books_rel())
            .unwrap()
            .unwrap();
        assert_eq!(
            count.sql,
            "EXISTS (SELECT 1 FROM \"public\".\"books\" \
             WHERE \"books\".\"author_id\" = \"authors\".\"id\")"
        );
    }

    #[test]
    fn count_finds_a_null_filter_on_a_hinted_embed() {
        let query = parse_get_query("select=name,books!author_id(title)&books=is.null").unwrap();
        let count = count_where("public", "authors", &query, &[], &hinted_books_rel())
            .unwrap()
            .unwrap();
        assert_eq!(
            count.sql,
            "NOT EXISTS (SELECT 1 FROM \"public\".\"books\" \
             WHERE \"books\".\"author_id\" = \"authors\".\"id\")"
        );
    }

    #[test]
    fn embedded_parameters_target_the_name_or_alias() {
        for query in [
            "select=b:books(title)&b.title=eq.x",
            "select=books:books(title)&books.title=eq.x",
        ] {
            assert!(parse_get_query(query).is_ok(), "{query}");
        }
        // The relation name of an aliased embed draws a `Warning` header upstream.
        assert_eq!(
            parse_get_query("select=b:books(title)&books.title=eq.x").unwrap_err(),
            QueryFail::Route
        );
        for (query, resource) in [
            ("select=id&books.title=eq.x", "books"),
            ("select=id,books(title)&books.tags.limit=1", "tags"),
            ("select=id,books(title)&authors.or=(id.eq.1)", "authors"),
            // Filters are placed before orders, right to left.
            ("select=id,books(title)&x.order=id&y.a=eq.1&z.a=eq.1", "z"),
        ] {
            assert_eq!(
                parse_get_query(query).unwrap_err(),
                QueryFail::NotEmbedded {
                    resource: resource.into()
                },
                "{query}"
            );
        }
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
        let sql = build_read_sql("public", "todos", &query, &[], &HashMap::new()).unwrap();
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

    #[test]
    fn json_filter_casts_the_value_to_the_path_type() {
        let query = parse_get_query(
            "select=id&payload->user->>name=eq.a&payload->user=eq.{}&title->>k=like.*x*\
             &or=(payload->tags->0.eq.1,meta->>n.is.null)",
        )
        .unwrap();
        let types = [
            ("id", "bigint"),
            ("payload", "jsonb"),
            ("title", "text"),
            ("meta", "json"),
        ];
        let sql = build_read_sql("public", "events", &query, &types, &HashMap::new()).unwrap();
        assert!(
            sql.sql.contains(
                "WHERE \"events\".\"payload\"->$1->>$2 = ($3::text)::text \
                 AND \"events\".\"payload\"->$4 = ($5::text)::jsonb \
                 AND to_jsonb(\"events\".\"title\")->>$6 like ($7::text)::text \
                 AND (\"events\".\"payload\"->$8->$9::int = ($10::text)::jsonb \
                 OR \"events\".\"meta\"->>$11 IS NULL)"
            ),
            "{}",
            sql.sql
        );
        assert_eq!(
            sql.params,
            vec!["user", "name", "a", "user", "{}", "k", "%x%", "tags", "+0", "1", "n"]
        );
    }

    #[test]
    fn json_filter_on_a_json_column_compares_json() {
        let query = parse_get_query("meta->k=eq.1&ghost->k=eq.1").unwrap();
        let types = [("meta", "json")];
        let sql = build_read_sql("public", "events", &query, &types, &HashMap::new()).unwrap();
        // A missing column is still wrapped, so PostgreSQL reports `42703`.
        assert!(
            sql.sql.contains(
                "\"events\".\"meta\"->$1 = ($2::text)::json \
                 AND to_jsonb(\"events\".\"ghost\")->$3 = ($4::text)::jsonb"
            ),
            "{}",
            sql.sql
        );
    }

    #[test]
    fn json_filter_keys_split_the_embed_path_first() {
        assert_eq!(
            filter_path("books.data->a->>0"),
            Some((
                vec!["books".to_string()],
                "data".to_string(),
                vec![
                    JsonStep::Arrow(JsonOperand::Key("a".to_string())),
                    JsonStep::TwoArrow(JsonOperand::Index("+0".to_string())),
                ],
            ))
        );
        // Text after the path, or an empty operand, is not split.
        assert_eq!(filter_path("data->a)"), None);
        assert_eq!(filter_path("data->"), None);
        assert_eq!(filter_path("->a"), None);
        // A JSON path never names an embed with `is.null`.
        let query = parse_get_query("data->a=is.null").unwrap();
        assert!(!query.has_null_embed_filter("data"));
        // `PUT` needs plain column `eq` filters.
        let query = parse_get_query("data->a=eq.1").unwrap();
        assert_eq!(query.eq_column_filters(), None);
    }
}
