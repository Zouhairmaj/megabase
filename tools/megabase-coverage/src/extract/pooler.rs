//! Supavisor (Elixir). The pooler is not behind the gateway: its management
//! API and Postgres listeners are served on their own ports, so unit paths
//! are the upstream ones.

use anyhow::Result;
use regex::Regex;

use super::{first_segment, phoenix, require, Ctx};
use crate::model::UnitSpec;
use crate::scan::{clean, Repo, ELIXIR};

const C: &str = "pooler";
const LEVEL: u8 = 4;

pub fn extract(ctx: &mut Ctx, repo: &Repo) -> Result<()> {
    let file = "lib/supavisor_web/router.ex";
    let text = repo.read(file)?;
    for r in require(phoenix::routes(&text), "supavisor routes")? {
        let source = repo.source(file, &text, r.offset);
        if r.path == "/api/openapi" || r.path.starts_with("/swaggerui") {
            ctx.out.exclude(
                C,
                format!("{} {}", r.method, r.path),
                "API documentation UI",
                source,
            );
            continue;
        }
        let group = match first_segment(&r.path, 0).as_str() {
            "api" => first_segment(&r.path, 1),
            other => other.to_string(),
        };
        ctx.out.route(C, &group, &r.method, &r.path, LEVEL, source);
    }

    let file = "lib/supavisor/tenants/user.ex";
    let text = repo.read(file)?;
    let code = clean(&text, ELIXIR).code;
    let modes = Regex::new(r"field\(:mode_type,\s*Ecto\.Enum,\s*values:\s*\[([^\]]*)\]").unwrap();
    let mut found = Vec::new();
    if let Some(c) = modes.captures(&code) {
        for mode in c[1].split(',') {
            found.push((
                mode.trim().trim_start_matches(':').to_string(),
                c.get(0).unwrap().start(),
            ));
        }
    }
    for (mode, offset) in require(found, "pool modes")? {
        ctx.out.item(UnitSpec {
            component: C,
            group: "pool-modes",
            kind: "pool-mode",
            name: mode,
            level: LEVEL,
            source: repo.source(file, &text, offset),
        });
    }
    Ok(())
}
