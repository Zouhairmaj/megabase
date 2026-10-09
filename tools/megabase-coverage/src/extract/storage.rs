//! Supabase Storage (TypeScript, fastify).

use std::collections::HashMap;

use anyhow::{Context, Result};
use regex::Regex;

use super::{require, ts_routes, Ctx};
use crate::model::UnitSpec;
use crate::scan::{join_paths, normalize_path, string_consts, Repo};

const C: &str = "storage";
const HOST: &str = "storage:5000";
const LEVEL: u8 = 2;
const ROUTES_DIR: &str = "src/http/routes";

pub fn extract(ctx: &mut Ctx, repo: &Repo) -> Result<()> {
    let app = repo.read("src/app.ts")?;
    let index = repo.read(&format!("{ROUTES_DIR}/index.ts"))?;
    let register = Regex::new(r"register\(routes\.(\w+),\s*\{\s*prefix:\s*'([^']*)'").unwrap();
    let export = Regex::new(r"export \{ default as (\w+) \} from '\./([\w-]+)'").unwrap();
    let dirs: HashMap<String, String> = export
        .captures_iter(&index)
        .map(|c| (c[1].to_string(), c[2].to_string()))
        .collect();
    let mounts: Vec<_> = register.captures_iter(&app).collect();
    let mut count = 0;
    for mount in require(mounts, "storage route mounts")? {
        let name = &mount[1];
        let dir = dirs
            .get(name)
            .with_context(|| format!("routes.{name} is not exported from routes/index.ts"))?;
        let group = match name {
            "tus" => "resumable",
            "healthcheck" => "health",
            other => other,
        };
        count += mount_dir(ctx, repo, &format!("{ROUTES_DIR}/{dir}"), &mount[2], group)?;
    }
    require(vec![(); count], "storage routes")?;
    sql_objects(ctx, repo)
}

/// Registers every route defined under `dir`, honoring prefixes the
/// directory's `index.ts` applies to individual route files and prefixes a
/// file applies to its own route sets.
fn mount_dir(ctx: &mut Ctx, repo: &Repo, dir: &str, prefix: &str, group: &str) -> Result<usize> {
    let index = repo.read(&format!("{dir}/index.ts")).unwrap_or_default();
    let import = Regex::new(r"import (\w+) from '\./([\w-]+)'").unwrap();
    let register = Regex::new(r"register\((\w+),\s*\{\s*prefix:\s*'([^']*)'\s*\}").unwrap();
    let imports: HashMap<String, String> = import
        .captures_iter(&index)
        .map(|c| (c[1].to_string(), c[2].to_string()))
        .collect();
    let mut file_prefix: HashMap<String, String> = HashMap::new();
    for c in register.captures_iter(&index) {
        if let Some(stem) = imports.get(&c[1]) {
            file_prefix.insert(stem.clone(), c[2].to_string());
        }
    }

    let files = repo.files(dir, |f| f.ends_with(".ts") && !f.ends_with(".test.ts"))?;
    let mut dir_consts = Vec::new();
    for file in &files {
        dir_consts.extend(string_consts(&repo.read(file)?));
    }
    let const_prefix = Regex::new(r"\{\s*prefix:\s*([A-Z_][A-Z0-9_]*)\s*\}").unwrap();

    let mut count = 0;
    for file in &files {
        let text = repo.read(file)?;
        let stem = file.rsplit('/').next().unwrap().trim_end_matches(".ts");
        let sub = file_prefix.get(stem).map(String::as_str).unwrap_or("");
        let mut variants = vec![String::new()];
        for c in const_prefix.captures_iter(&text) {
            let value = dir_consts
                .iter()
                .find(|(k, _)| k == &c[1])
                .with_context(|| format!("unresolved prefix {} in {file}", &c[1]))?;
            if !variants.contains(&value.1) {
                variants.push(value.1.clone());
            }
        }
        for route in ts_routes(&text, &["fastify", "s3Router"]) {
            for variant in &variants {
                let path = normalize_path(&join_paths(&[prefix, sub, variant, &route.path]));
                let path = if route.path.ends_with('/') && route.path.len() > 1 {
                    format!("{path}/")
                } else {
                    path
                };
                let source = repo.source(file, &text, route.offset);
                ctx.gateway_route(C, group, HOST, &route.method, &path, LEVEL, source);
                count += 1;
            }
        }
    }
    Ok(count)
}

fn sql_objects(ctx: &mut Ctx, repo: &Repo) -> Result<()> {
    let function_re = Regex::new(
        r#"(?i)create\s+(?:or\s+replace\s+)?function\s+"?storage"?\."?([a-z_][a-z0-9_]*)"?"#,
    )
    .unwrap();
    let table_re = Regex::new(
        r#"(?i)create\s+table\s+(?:if\s+not\s+exists\s+)?"?storage"?\."?([a-z_][a-z0-9_]*)"?"#,
    )
    .unwrap();
    let mut count = 0;
    for file in repo.files("migrations/tenant", |f| f.ends_with(".sql"))? {
        let text = repo.read(&file)?;
        for (re, kind, suffix) in [
            (&function_re, "sql-function", "()"),
            (&table_re, "sql-table", ""),
        ] {
            for c in re.captures_iter(&text) {
                count += 1;
                ctx.out.item(UnitSpec {
                    component: C,
                    group: "database",
                    kind,
                    name: format!("storage.{}{suffix}", c[1].to_lowercase()),
                    level: LEVEL,
                    source: repo.source(&file, &text, c.get(0).unwrap().start()),
                });
            }
        }
    }
    require(vec![(); count], "storage SQL objects")?;
    Ok(())
}
