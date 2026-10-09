//! PostgREST (Haskell).

use anyhow::Result;
use regex::Regex;

use super::{require, Ctx};
use crate::model::UnitSpec;
use crate::scan::{clean, Repo, HASKELL};

const C: &str = "rest";
const HOST: &str = "rest:3000";
const LIB: &str = "src/library/PostgREST";

pub fn extract(ctx: &mut Ctx, repo: &Repo) -> Result<()> {
    routes(ctx, repo)?;
    query_params(ctx, repo)?;
    grammar(ctx, repo)?;
    preferences(ctx, repo)?;
    media_types(ctx, repo)?;
    Ok(())
}

/// `getAction` maps (resource kind, HTTP method) pairs to actions.
fn routes(ctx: &mut Ctx, repo: &Repo) -> Result<()> {
    let file = format!("{LIB}/ApiRequest.hs");
    let text = repo.read(&file)?;
    let code = clean(&text, HASKELL).code;
    let re = Regex::new(r#"\(Resource(Routine|Relation|Schema)\b[^,()]*,\s*"([A-Z]+)"\)"#).unwrap();
    let found: Vec<_> = re.captures_iter(&code).collect();
    for c in require(found, "PostgREST resource actions")? {
        let path = match &c[1] {
            "Routine" => "/rpc/{function}",
            "Relation" => "/{relation}",
            _ => "/",
        };
        let group = if &c[1] == "Routine" {
            "rpc"
        } else {
            "resources"
        };
        let source = repo.source(&file, &text, c.get(0).unwrap().start());
        ctx.gateway_route(C, group, HOST, &c[2], path, 1, source);
    }
    Ok(())
}

fn query_params(ctx: &mut Ctx, repo: &Repo) -> Result<()> {
    let file = format!("{LIB}/ApiRequest/QueryParams.hs");
    let text = repo.read(&file)?;
    let code = clean(&text, HASKELL).code;
    let lookup = Regex::new(r#"lookupParam\s+"([a-z_]+)""#).unwrap();
    let ending = Regex::new(r"endingIn\s+\[([^\]]*)\]").unwrap();
    let literal = Regex::new(r#""([a-z_]+)""#).unwrap();
    let mut found = Vec::new();
    for c in lookup.captures_iter(&code) {
        found.push((c[1].to_string(), c.get(0).unwrap().start()));
    }
    for c in ending.captures_iter(&code) {
        for l in literal.captures_iter(&c[1]) {
            found.push((l[1].to_string(), c.get(0).unwrap().start()));
        }
    }
    for (name, offset) in require(found, "PostgREST query parameters")? {
        ctx.out.item(UnitSpec {
            component: C,
            group: "query-params",
            kind: "query-param",
            name,
            level: 1,
            source: repo.source(&file, &text, offset),
        });
    }
    Ok(())
}

/// Keywords of the query-string grammar, grouped by the parser that accepts
/// them (`string "eq"` inside `simpleOperator`, and so on).
fn grammar(ctx: &mut Ctx, repo: &Repo) -> Result<()> {
    let file = format!("{LIB}/ApiRequest/QueryParams.hs");
    let text = repo.read(&file)?;
    let code = clean(&text, HASKELL).code;
    let definition = Regex::new(r"(?m)^([a-z][A-Za-z0-9_']*)\s*::").unwrap();
    let keyword = Regex::new(r#"\bstring\s+"([a-z]+)""#).unwrap();
    let parsers: Vec<(usize, String)> = definition
        .captures_iter(&code)
        .map(|c| (c.get(0).unwrap().start(), c[1].to_string()))
        .collect();
    let mut count = 0;
    for c in keyword.captures_iter(&code) {
        let offset = c.get(0).unwrap().start();
        let parser = parsers
            .iter()
            .rev()
            .find(|(start, _)| *start < offset)
            .map(|(_, name)| name.as_str())
            .unwrap_or("toplevel");
        let (group, kind) = match parser {
            "simpleOperator" | "quantOperator" | "pOpExpr" => ("filtering", "filter-operator"),
            "pFieldSelect" => ("aggregates", "aggregate"),
            "pEmbedParams" => ("embedding", "embed-join"),
            "pLogicTree" => ("logic", "logic-operator"),
            p if p.starts_with("pOrder") => ("ordering", "order-modifier"),
            p => (p, p),
        };
        count += 1;
        ctx.out.item(UnitSpec {
            component: C,
            group,
            kind,
            name: c[1].to_string(),
            level: 1,
            source: repo.source(&file, &text, offset),
        });
    }
    require(vec![(); count], "PostgREST grammar keywords")?;
    Ok(())
}

fn preferences(ctx: &mut Ctx, repo: &Repo) -> Result<()> {
    let file = format!("{LIB}/ApiRequest/Preferences.hs");
    let text = repo.read(&file)?;
    let code = clean(&text, HASKELL).code;
    let re = Regex::new(r#"(?m)^\s*toHeaderValue\s[^=\n]*=\s*"([a-z-]+)=([a-z-]*)""#).unwrap();
    let found: Vec<_> = re.captures_iter(&code).collect();
    for c in require(found, "Prefer header values")? {
        let value = if c[2].is_empty() { "*" } else { &c[2] };
        ctx.out.item(UnitSpec {
            component: C,
            group: "prefer",
            kind: "prefer",
            name: format!("{}={value}", &c[1]),
            level: 1,
            source: repo.source(&file, &text, c.get(0).unwrap().start()),
        });
    }
    Ok(())
}

fn media_types(ctx: &mut Ctx, repo: &Repo) -> Result<()> {
    let file = format!("{LIB}/MediaType.hs");
    let text = repo.read(&file)?;
    let code = clean(&text, HASKELL).code;
    let re = Regex::new(r#"(?m)^toMime\s[^=\n]*=\s*"([^"]+)""#).unwrap();
    let found: Vec<_> = re.captures_iter(&code).collect();
    for c in require(found, "media types")? {
        let name = match c[1].strip_suffix('+') {
            Some(base) => format!("{base}+{{format}}"),
            None => c[1].to_string(),
        };
        ctx.out.item(UnitSpec {
            component: C,
            group: "media-types",
            kind: "media-type",
            name,
            level: 1,
            source: repo.source(&file, &text, c.get(0).unwrap().start()),
        });
    }
    Ok(())
}
