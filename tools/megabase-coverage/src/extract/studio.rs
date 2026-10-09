//! Supabase Studio (Next.js pages router): project pages and server-side API
//! routes. Pages outside `/project/[ref]` belong to the hosted platform
//! (organizations, billing, sign-in) and are excluded (MANIFESTO.md, scope).

use anyhow::Result;
use regex::Regex;

use super::{first_segment, require, Ctx};
use crate::scan::{normalize_path, Repo};

const C: &str = "studio";
const HOST: &str = "studio:3000";
const LEVEL: u8 = 5;
const PAGES: &str = "apps/studio/pages";

fn route_path(file: &str) -> String {
    let rel = file.trim_start_matches(PAGES);
    let rel = rel.rsplit_once('.').map_or(rel, |(stem, _)| stem);
    let rel = rel.strip_suffix("/index").unwrap_or(rel);
    let rel = if rel.is_empty() { "/" } else { rel };
    normalize_path(rel)
}

pub fn extract(ctx: &mut Ctx, repo: &Repo) -> Result<()> {
    let is_source = |f: &str| {
        let name = f.rsplit('/').next().unwrap_or(f);
        (f.ends_with(".tsx") || f.ends_with(".ts") || f.ends_with(".jsx"))
            && !name.starts_with('_')
            && !name.contains(".test.")
            && !f.ends_with(".d.ts")
    };
    let mut pages = 0;
    for file in repo.files(PAGES, is_source)? {
        if file.starts_with(&format!("{PAGES}/api/")) {
            continue;
        }
        let path = route_path(&file);
        let source = repo.source(&file, "", 0);
        if !path.starts_with("/project/{ref}") {
            ctx.out
                .exclude(C, format!("GET {path}"), "hosted-platform page", source);
            continue;
        }
        let group = match first_segment(&path, 2).as_str() {
            "root" => "project-home".to_string(),
            g => g.to_string(),
        };
        ctx.gateway_route(C, &group, HOST, "GET", &path, LEVEL, source);
        pages += 1;
    }
    require(vec![(); pages], "studio pages")?;

    let method = Regex::new(
        r#"case '(GET|POST|PUT|PATCH|DELETE|HEAD)'|method === '(GET|POST|PUT|PATCH|DELETE|HEAD)'"#,
    )
    .unwrap();
    let mut api = 0;
    for file in repo.files(&format!("{PAGES}/api"), is_source)? {
        let text = repo.read(&file)?;
        let path = route_path(&file);
        let group = format!("api-{}", first_segment(&path, 1));
        let mut methods: Vec<(String, usize)> = method
            .captures_iter(&text)
            .map(|c| {
                let m = c.get(1).or(c.get(2)).unwrap().as_str().to_string();
                (m, c.get(0).unwrap().start())
            })
            .collect();
        methods.sort();
        methods.dedup_by(|a, b| a.0 == b.0);
        if methods.is_empty() {
            methods.push(("*".into(), 0));
        }
        for (m, offset) in methods {
            let source = repo.source(&file, &text, offset);
            ctx.gateway_route(C, &group, HOST, &m, &path, LEVEL, source);
            api += 1;
        }
    }
    require(vec![(); api], "studio API routes")?;
    Ok(())
}
