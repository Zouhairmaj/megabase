//! Postgres Meta (TypeScript, fastify).

use std::collections::HashMap;

use anyhow::{Context, Result};
use regex::Regex;

use super::{first_segment, require, ts_routes, Ctx};
use crate::scan::{join_paths, normalize_path, Repo};

const C: &str = "meta";
const HOST: &str = "meta:8080";
const LEVEL: u8 = 4;

pub fn extract(ctx: &mut Ctx, repo: &Repo) -> Result<()> {
    let app_file = "src/server/app.ts";
    let app = repo.read(app_file)?;
    let mut count = 0;
    for route in ts_routes(&app, &["app"]) {
        let source = repo.source(app_file, &app, route.offset);
        ctx.gateway_route(C, "server", HOST, &route.method, &route.path, LEVEL, source);
        count += 1;
    }

    let index_file = "src/server/routes/index.ts";
    let index = repo.read(index_file)?;
    let import = Regex::new(r"import (\w+) from '\./([\w/-]+)\.js'").unwrap();
    let register =
        Regex::new(r"register\((\w+)(?:\([^)]*\))?,\s*\{\s*prefix:\s*['`]([^'`]*)['`]").unwrap();
    let imports: HashMap<String, String> = import
        .captures_iter(&index)
        .map(|c| (c[1].to_string(), c[2].to_string()))
        .collect();
    let mounts: Vec<_> = register.captures_iter(&index).collect();
    for mount in require(mounts, "postgres-meta route mounts")? {
        let module = imports
            .get(&mount[1])
            .with_context(|| format!("{} is not imported in routes/index.ts", &mount[1]))?;
        let file = format!("src/server/routes/{module}.ts");
        let text = repo.read(&file)?;
        let prefix = normalize_path(&mount[2]);
        let group = first_segment(&prefix, 0);
        for route in ts_routes(&text, &["fastify"]) {
            let path = join_paths(&[&prefix, &route.path]);
            let source = repo.source(&file, &text, route.offset);
            ctx.gateway_route(C, &group, HOST, &route.method, &path, LEVEL, source);
            count += 1;
        }
    }
    require(vec![(); count], "postgres-meta routes")?;
    Ok(())
}
