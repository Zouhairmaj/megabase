//! Extraction of the coverage denominator from the pinned upstream sources.
//!
//! Each component module reads its upstream repository in `vendor/` and
//! records units (routes, operators, flows, message types, pages...) with the
//! file and line they came from. Nothing here is a hand-maintained list: if
//! an upstream file moves or a pattern stops matching, extraction fails.

mod auth;
mod functions;
mod meta;
mod phoenix;
mod pooler;
mod realtime;
mod rest;
mod storage;
mod studio;

use std::path::Path;

use anyhow::{bail, Context, Result};
use regex::Regex;

use crate::kong::{Exposure, Kong};
use crate::model::{Collector, Pin, PinsFile, Source, UnitsFile};
use crate::scan::{normalize_path, Repo, TS};

pub struct Ctx {
    pub kong: Kong,
    pub out: Collector,
}

impl Ctx {
    /// Records an HTTP route served by `host` upstream, translated through
    /// the gateway; routes the gateway blocks or does not expose are recorded
    /// as exclusions instead.
    #[allow(clippy::too_many_arguments)]
    pub fn gateway_route(
        &mut self,
        component: &str,
        group: &str,
        host: &str,
        method: &str,
        upstream_path: &str,
        level: u8,
        source: Source,
    ) {
        let name = format!("{method} {upstream_path}");
        match self.kong.expose(host, upstream_path) {
            Exposure::Gateway(path) => self
                .out
                .route(component, group, method, &path, level, source),
            Exposure::Blocked(service) => self.out.exclude(
                component,
                name,
                &format!("blocked by the gateway (kong.yml service `{service}`)"),
                source,
            ),
            Exposure::NotExposed => self.out.exclude(
                component,
                name,
                "not exposed through the gateway (no kong.yml service)",
                source,
            ),
        }
    }
}

pub fn load_pins(root: &Path) -> Result<Vec<Pin>> {
    let text = std::fs::read_to_string(root.join("vendor.toml")).context("reading vendor.toml")?;
    let pins: PinsFile = toml::from_str(&text).context("parsing vendor.toml")?;
    Ok(pins.pin)
}

pub fn extract(root: &Path) -> Result<UnitsFile> {
    let pins = load_pins(root)?;
    let repo = |name: &str| -> Result<Repo> {
        let pin = pins
            .iter()
            .find(|p| p.name == name)
            .with_context(|| format!("vendor.toml has no pin named {name}"))?;
        let dir = root.join(&pin.path);
        if !dir.join(".git").exists()
            && std::fs::read_dir(&dir).is_ok_and(|mut d| d.next().is_none())
        {
            bail!(
                "{} is empty; run `git submodule update --init --depth 1 {}`",
                pin.path,
                pin.path
            );
        }
        Ok(Repo {
            name: name.to_string(),
            root: dir,
        })
    };

    let supabase = repo("supabase")?;
    let mut ctx = Ctx {
        kong: Kong::load(&supabase)?,
        out: Collector::default(),
    };
    rest::extract(&mut ctx, &repo("postgrest")?)?;
    auth::extract(&mut ctx, &repo("auth")?)?;
    realtime::extract(&mut ctx, &repo("realtime")?, &repo("supabase-js")?)?;
    storage::extract(&mut ctx, &repo("storage")?)?;
    functions::extract(&mut ctx, &repo("edge-runtime")?, &supabase)?;
    pooler::extract(&mut ctx, &repo("supavisor")?)?;
    meta::extract(&mut ctx, &repo("postgres-meta")?)?;
    studio::extract(&mut ctx, &supabase)?;
    for component in ["auth", "meta", "pooler", "studio"] {
        ctx.out.merge_small_route_groups(component, 3);
    }
    Ok(ctx.out.finish(pins))
}

/// Fails when a pattern that must match upstream source finds nothing, which
/// means the upstream layout changed and the extractor needs updating.
pub fn require<T>(items: Vec<T>, what: &str) -> Result<Vec<T>> {
    if items.is_empty() {
        bail!("extractor found no {what}; the upstream layout changed");
    }
    Ok(items)
}

pub struct TsRoute {
    pub method: String,
    pub path: String,
    pub offset: usize,
}

/// Finds `<receiver>.<method>(<path>, ...)` registrations in TypeScript
/// (fastify and storage's S3 router), skipping generic type arguments and
/// resolving a path given as a same-file `const`.
pub fn ts_routes(text: &str, receivers: &[&str]) -> Vec<TsRoute> {
    let cleaned = crate::scan::clean(text, TS);
    let code = &cleaned.code;
    let pattern = format!(
        r"\b(?:{})\.(get|post|put|patch|delete|head|options)\s*",
        receivers.join("|")
    );
    let call = Regex::new(&pattern).unwrap();
    let literal = Regex::new(r#"^\(\s*(['"`])([^'"`]*)['"`]"#).unwrap();
    let ident = Regex::new(r"^\(\s*([A-Za-z_][A-Za-z0-9_]*)\s*,").unwrap();
    let consts = crate::scan::string_consts(code);
    let mut routes = Vec::new();
    for m in call.captures_iter(code) {
        let whole = m.get(0).unwrap();
        let mut i = whole.end();
        let bytes = code.as_bytes();
        if bytes.get(i) == Some(&b'<') {
            let mut depth = 0i32;
            while i < bytes.len() {
                match bytes[i] {
                    b'<' => depth += 1,
                    b'>' => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
        }
        let rest = &code[i..];
        let path = if let Some(c) = literal.captures(rest) {
            c[2].to_string()
        } else if let Some(c) = ident.captures(rest) {
            match consts.iter().rev().find(|(k, _)| k == &c[1]) {
                Some((_, v)) => v.clone(),
                None => continue,
            }
        } else {
            continue;
        };
        routes.push(TsRoute {
            method: m[1].to_uppercase(),
            path: normalize_path(&path),
            offset: whole.start(),
        });
    }
    routes
}

/// The `skip`-th segment of `path`, used as a feature-group name.
pub fn first_segment(path: &str, skip: usize) -> String {
    path.split('/')
        .filter(|s| !s.is_empty())
        .nth(skip)
        .map(|s| {
            s.trim_start_matches('{')
                .trim_end_matches('}')
                .trim_start_matches('.')
                .to_string()
        })
        .unwrap_or_else(|| "root".into())
}
