//! Supabase Auth (Go, chi router).

use std::collections::{BTreeSet, HashMap};

use anyhow::{Context, Result};
use regex::Regex;

use super::{first_segment, require, Ctx};
use crate::model::UnitSpec;
use crate::scan::{block_end, clean, join_paths, string_consts, Repo, GO};

const C: &str = "auth";
const HOST: &str = "auth:9999";
const ROUTES: &str = "internal/api/api.go";

/// Level 1 is email/password and JWT; everything else waits for Level 2.
const LEVEL_2_GROUPS: &[&str] = &[
    "authorize",
    "callback",
    "factors",
    "identities",
    "magiclink",
    "oauth",
    "otp",
    "passkeys",
    "scim",
    "sso",
    "well-known",
    "custom-providers",
];

fn route_level(group: &str) -> u8 {
    if LEVEL_2_GROUPS.contains(&group) {
        2
    } else {
        1
    }
}

pub fn extract(ctx: &mut Ctx, repo: &Repo) -> Result<()> {
    routes(ctx, repo)?;
    grant_types(ctx, repo)?;
    providers(ctx, repo)?;
    verify_types(ctx, repo)?;
    sql_objects(ctx, repo)?;
    Ok(())
}

fn routes(ctx: &mut Ctx, repo: &Repo) -> Result<()> {
    let text = repo.read(ROUTES)?;
    let cleaned = clean(&text, GO);
    let mut consts: HashMap<String, String> = HashMap::new();
    for (k, v) in string_consts(&repo.read("internal/api/scim/server.go")?) {
        consts.insert(format!("scim.{k}"), v);
    }

    let open_re = Regex::new(
        r#"\.Route\(\s*(?:"([^"]*)"|([A-Za-z_][A-Za-z0-9_.]*))\s*,\s*func\s*\(\s*r\s+\*router\s*\)\s*\{"#,
    )
    .unwrap();
    let handler_re =
        Regex::new(r#"\.\s*(Get|Post|Put|Patch|Delete|Head|Options)\(\s*"([^"]*)""#).unwrap();

    let mut opens: HashMap<usize, String> = HashMap::new();
    for c in open_re.captures_iter(&cleaned.code) {
        let prefix = match (c.get(1), c.get(2)) {
            (Some(lit), _) => lit.as_str().to_string(),
            (None, Some(name)) => consts
                .get(name.as_str())
                .with_context(|| format!("unresolved route prefix {}", name.as_str()))?
                .clone(),
            _ => unreachable!(),
        };
        opens.insert(c.get(0).unwrap().end() - 1, prefix);
    }
    let handlers: HashMap<usize, (String, String)> = handler_re
        .captures_iter(&cleaned.code)
        .map(|c| {
            (
                c.get(0).unwrap().start(),
                (c[1].to_uppercase(), c[2].to_string()),
            )
        })
        .collect();

    let mut stack: Vec<(String, usize)> = Vec::new();
    let mut depth = 0usize;
    let mut found = Vec::new();
    for (i, b) in cleaned.skeleton.bytes().enumerate() {
        if let Some((method, path)) = handlers.get(&i) {
            let mut parts: Vec<&str> = stack.iter().map(|(p, _)| p.as_str()).collect();
            parts.push(path);
            found.push((method.clone(), join_paths(&parts), i));
        }
        match b {
            b'{' => {
                depth += 1;
                if let Some(prefix) = opens.get(&i) {
                    stack.push((prefix.clone(), depth));
                }
            }
            b'}' => {
                if stack.last().is_some_and(|(_, d)| *d == depth) {
                    stack.pop();
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    for (method, path, offset) in require(found, "auth routes")? {
        let group = first_segment(&path, 0);
        let group = if group == "root" {
            "health".into()
        } else {
            group
        };
        let source = repo.source(ROUTES, &text, offset);
        ctx.gateway_route(C, &group, HOST, &method, &path, route_level(&group), source);
    }
    Ok(())
}

fn function_body<'a>(text: &'a str, signature: &str) -> Result<(usize, &'a str)> {
    let start = text
        .find(signature)
        .with_context(|| format!("`{signature}` not found"))?;
    let cleaned = clean(text, GO);
    let open = start
        + cleaned.skeleton[start..]
            .find('{')
            .context("function body")?;
    let end = block_end(&cleaned.skeleton, open);
    Ok((open, &text[open..end]))
}

fn grant_types(ctx: &mut Ctx, repo: &Repo) -> Result<()> {
    let file = "internal/api/token.go";
    let text = repo.read(file)?;
    let (start, body) = function_body(&text, "func (a *API) Token(")?;
    let case_re = Regex::new(r#"case\s+"([a-z_0-9]+)"\s*:"#).unwrap();
    let cases: Vec<_> = case_re.captures_iter(body).collect();
    for c in require(cases, "grant types")? {
        let grant = c[1].to_string();
        let level = if matches!(grant.as_str(), "password" | "refresh_token") {
            1
        } else {
            2
        };
        ctx.out.item(UnitSpec {
            component: C,
            group: "token",
            kind: "grant-type",
            name: grant,
            level,
            source: repo.source(file, &text, start + c.get(0).unwrap().start()),
        });
    }
    Ok(())
}

fn providers(ctx: &mut Ctx, repo: &Repo) -> Result<()> {
    let file = "internal/api/external.go";
    let text = repo.read(file)?;
    let consts: HashMap<String, String> =
        string_consts(&repo.read("internal/api/provider_constants.go")?)
            .into_iter()
            .collect();
    let (start, body) = function_body(&text, "func (a *API) Provider(")?;
    let case_re = Regex::new(r"case\s+([A-Za-z]+Provider)\s*:").unwrap();
    let mut found = Vec::new();
    for c in case_re.captures_iter(body) {
        let name = consts
            .get(&c[1])
            .with_context(|| format!("unresolved provider constant {}", &c[1]))?;
        found.push((name.clone(), start + c.get(0).unwrap().start()));
    }
    let prefix_re = Regex::new(r#"HasPrefix\(name,\s*"([a-z]+):"\)"#).unwrap();
    for c in prefix_re.captures_iter(body) {
        found.push((
            format!("{}:{{identifier}}", &c[1]),
            start + c.get(0).unwrap().start(),
        ));
    }
    for (name, offset) in require(found, "OAuth providers")? {
        ctx.out.item(UnitSpec {
            component: C,
            group: "oauth-providers",
            kind: "provider",
            name,
            level: 2,
            source: repo.source(file, &text, offset),
        });
    }
    Ok(())
}

fn verify_types(ctx: &mut Ctx, repo: &Repo) -> Result<()> {
    let file = "internal/api/verify.go";
    let text = repo.read(file)?;
    let cleaned = clean(&text, GO);
    let mut consts: HashMap<String, String> = string_consts(&text).into_iter().collect();
    for (k, v) in string_consts(&repo.read("internal/mailer/mailer.go")?) {
        consts.insert(format!("mail.{k}"), v);
    }
    let switch_re = Regex::new(r"switch\s+params\.Type\s*\{").unwrap();
    let case_re = Regex::new(r"case\s+([^:\n]+):").unwrap();
    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    for m in switch_re.find_iter(&cleaned.code) {
        let end = block_end(&cleaned.skeleton, m.end() - 1);
        for c in case_re.captures_iter(&cleaned.code[m.end()..end]) {
            for ident in c[1].split(',').map(str::trim) {
                if let Some(value) = consts.get(ident) {
                    if seen.insert(value.clone()) {
                        found.push((value.clone(), m.end() + c.get(0).unwrap().start()));
                    }
                }
            }
        }
    }
    for (name, offset) in require(found, "verification types")? {
        let level = if matches!(
            name.as_str(),
            "signup" | "recovery" | "invite" | "email_change"
        ) {
            1
        } else {
            2
        };
        ctx.out.item(UnitSpec {
            component: C,
            group: "verify",
            kind: "verify-type",
            name,
            level,
            source: repo.source(file, &text, offset),
        });
    }
    Ok(())
}

fn sql_objects(ctx: &mut Ctx, repo: &Repo) -> Result<()> {
    let function_re = Regex::new(
        r"(?i)create\s+(?:or\s+replace\s+)?function\s+\{\{[^}]*\}\}\.([a-z_][a-z0-9_]*)",
    )
    .unwrap();
    let table_re = Regex::new(
        r"(?i)create\s+table\s+(?:if\s+not\s+exists\s+)?\{\{[^}]*\}\}\.([a-z_][a-z0-9_]*)",
    )
    .unwrap();
    let mut count = 0;
    for file in repo.files("migrations", |f| f.ends_with(".up.sql"))? {
        let text = repo.read(&file)?;
        for (re, kind, fmt) in [
            (&function_re, "sql-function", "()"),
            (&table_re, "sql-table", ""),
        ] {
            for c in re.captures_iter(&text) {
                count += 1;
                ctx.out.item(UnitSpec {
                    component: C,
                    group: "database",
                    kind,
                    name: format!("auth.{}{fmt}", c[1].to_lowercase()),
                    level: 1,
                    source: repo.source(&file, &text, c.get(0).unwrap().start()),
                });
            }
        }
    }
    require(vec![(); count], "auth SQL objects")?;
    Ok(())
}
