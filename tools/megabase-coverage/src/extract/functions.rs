//! Supabase Edge Runtime (Rust + Deno): the invocation route the gateway
//! exposes, and the runtime ops (`#[op2]`) its extensions give to user code.

use anyhow::{Context, Result};
use regex::Regex;

use super::{require, Ctx};
use crate::kong::KONG_FILE;
use crate::model::{Source, UnitSpec};
use crate::scan::Repo;

const C: &str = "functions";
const LEVEL: u8 = 4;

pub fn extract(ctx: &mut Ctx, repo: &Repo, supabase: &Repo) -> Result<()> {
    let service = ctx
        .kong
        .service("functions-v1")
        .context("kong.yml has no functions-v1 service")?;
    let route = service.routes[0].trim_end_matches('/').to_string();
    let line = service.line;
    ctx.out.route(
        C,
        "invoke",
        "*",
        &format!("{route}/{{function_name}}"),
        LEVEL,
        Source {
            repo: supabase.name.clone(),
            file: KONG_FILE.into(),
            line,
        },
    );

    let op = Regex::new(r"#\[op2[^\]]*\][\s\S]*?\bfn\s+(op_[A-Za-z0-9_]+)").unwrap();
    let mut count = 0;
    for file in repo.files("ext", |f| f.ends_with(".rs"))? {
        let text = repo.read(&file)?;
        let group = file.split('/').nth(1).unwrap_or("ext").to_string();
        for c in op.captures_iter(&text) {
            count += 1;
            ctx.out.item(UnitSpec {
                component: C,
                group: &group,
                kind: "op",
                name: c[1].to_string(),
                level: LEVEL,
                source: repo.source(&file, &text, c.get(0).unwrap().start()),
            });
        }
    }
    require(vec![(); count], "edge runtime ops")?;
    Ok(())
}
