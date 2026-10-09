//! Supabase Realtime (Elixir, Phoenix) plus the wire protocol constants of
//! realtime-js.

use anyhow::Result;
use regex::Regex;

use super::{first_segment, phoenix, require, Ctx};
use crate::model::UnitSpec;
use crate::scan::{clean, Repo, ELIXIR, TS};

const C: &str = "realtime";
const HOST: &str = "realtime-dev.supabase-realtime:4000";
const LEVEL: u8 = 3;

pub fn extract(ctx: &mut Ctx, repo: &Repo, client: &Repo) -> Result<()> {
    let file = "lib/realtime_web/router.ex";
    let text = repo.read(file)?;
    for r in require(phoenix::routes(&text), "realtime routes")? {
        let group = match first_segment(&r.path, 0).as_str() {
            "api" => "http-api".to_string(),
            other => other.to_string(),
        };
        let source = repo.source(file, &text, r.offset);
        ctx.gateway_route(C, &group, HOST, &r.method, &r.path, LEVEL, source);
    }

    // Phoenix serves a socket mounted at "/x" on "/x/websocket".
    let file = "lib/realtime_web/endpoint.ex";
    let text = repo.read(file)?;
    let socket = Regex::new(r#"(?m)^\s*socket\s+"([^"]+)",\s*([A-Za-z.]+)"#).unwrap();
    let sockets: Vec<_> = socket
        .captures_iter(&clean(&text, ELIXIR).code)
        .map(|c| (format!("{}/websocket", &c[1]), c.get(0).unwrap().start()))
        .collect();
    for (path, offset) in require(sockets, "realtime sockets")? {
        let source = repo.source(file, &text, offset);
        ctx.gateway_route(C, "websocket", HOST, "GET", &path, LEVEL, source);
    }

    messages(ctx, repo, client)
}

fn messages(ctx: &mut Ctx, repo: &Repo, client: &Repo) -> Result<()> {
    let file = "packages/core/realtime-js/src/lib/constants.ts";
    let text = client.read(file)?;
    let code = clean(&text, TS).code;
    let block = Regex::new(r"(?s)export const CHANNEL_EVENTS = \{(.*?)\}").unwrap();
    let entry = Regex::new(r"[a-z_]+:\s*'([a-z_]+)'").unwrap();
    let mut protocol = Vec::new();
    if let Some(b) = block.captures(&code) {
        let base = b.get(1).unwrap().start();
        for e in entry.captures_iter(&b[1]) {
            protocol.push((e[1].to_string(), base + e.get(0).unwrap().start()));
        }
    }
    for (name, offset) in require(protocol, "realtime-js channel events")? {
        ctx.out.item(UnitSpec {
            component: C,
            group: "protocol",
            kind: "protocol-event",
            name,
            level: LEVEL,
            source: client.source(file, &text, offset),
        });
    }

    let handle_in = Regex::new(r#"\bdef\s+handle_in\(\s*"([a-z_]+)""#).unwrap();
    let push = Regex::new(r#"\bpush\(\s*socket,\s*"([a-z_]+)""#).unwrap();
    let event = Regex::new(r#"\bevent:\s*"([a-z_]+)""#).unwrap();
    let mut client_events = 0;
    let mut server_events = 0;
    let keep = |f: &str| f.ends_with(".ex");
    let mut files = repo.files("lib/realtime_web/channels", keep)?;
    files.extend(repo.files("lib/extensions", keep)?);
    for file in files {
        let text = repo.read(&file)?;
        let code = clean(&text, ELIXIR).code;
        for c in handle_in.captures_iter(&code) {
            client_events += 1;
            ctx.out.item(UnitSpec {
                component: C,
                group: "client-events",
                kind: "client-event",
                name: c[1].to_string(),
                level: LEVEL,
                source: repo.source(&file, &text, c.get(0).unwrap().start()),
            });
        }
        for re in [&push, &event] {
            for c in re.captures_iter(&code) {
                server_events += 1;
                ctx.out.item(UnitSpec {
                    component: C,
                    group: "server-events",
                    kind: "server-event",
                    name: c[1].to_string(),
                    level: LEVEL,
                    source: repo.source(&file, &text, c.get(0).unwrap().start()),
                });
            }
        }
    }
    require(vec![(); client_events], "realtime client events")?;
    require(vec![(); server_events], "realtime server events")?;
    Ok(())
}
