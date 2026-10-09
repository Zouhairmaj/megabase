//! Turn `coverage/units.json` into a PM-quality backlog under 150 issues.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Deserialize;

const CHUNK: usize = 10;

#[derive(Debug, Deserialize)]
pub struct UnitsFile {
    pub units: Vec<Unit>,
    #[serde(default)]
    pub vendor: Vec<Pin>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Pin {
    pub name: String,
    pub commit: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Unit {
    pub id: String,
    pub component: String,
    pub group: String,
    pub name: String,
    pub level: u8,
    pub source: Source,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Source {
    pub repo: String,
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub key: String,
    pub title: String,
    pub body: String,
    pub labels: Vec<&'static str>,
    pub milestone: Option<&'static str>,
    pub component: &'static str,
    pub level: Option<u8>,
    pub effort: &'static str,
    pub status: &'static str,
    pub parent: Option<String>,
    /// megabase-id keys of issues that block this one.
    pub blocked_by: Vec<String>,
    pub priority: i32,
}

fn blocked_by_keys(component: &str, group: &str) -> Vec<String> {
    match (component, group) {
        ("rest", "filtering") | ("rest", "query-params") | ("rest", "prefer") | ("rest", "rpc") => {
            vec!["epic:rest:resources:1".into()]
        }
        ("auth", "endpoints") | ("auth", "token") | ("auth", "user") => {
            vec!["epic:auth:database:1".into()]
        }
        ("auth", "database") | ("rest", "resources") => vec!["epic:core:jwt:1".into()],
        _ => vec![],
    }
}

pub fn component_label(c: &str) -> &'static str {
    match c {
        "rest" => "REST",
        "auth" => "Auth",
        "realtime" => "Realtime",
        "storage" => "Storage",
        "functions" => "Functions",
        "pooler" => "Pooler",
        "meta" => "Meta",
        "studio" => "Studio",
        "core" => "Core",
        "judge" => "Judge",
        "website" => "Website",
        _ => "Megabase",
    }
}

fn component_static(c: &str) -> &'static str {
    match c {
        "rest" => "rest",
        "auth" => "auth",
        "realtime" => "realtime",
        "storage" => "storage",
        "functions" => "functions",
        "pooler" => "pooler",
        "meta" => "meta",
        "studio" => "studio",
        "core" => "core",
        "judge" => "judge",
        "website" => "website",
        _ => "core",
    }
}

fn milestone(level: u8) -> Option<&'static str> {
    match level {
        1 => Some("Level 1"),
        2 => Some("Level 2"),
        3 => Some("Level 3"),
        4 => Some("Level 4"),
        5 => Some("Level 5"),
        _ => None,
    }
}

/// How often real apps hit this group, 0–100. Used only to order Ready.
fn usage(component: &str, group: &str) -> i32 {
    match (component, group) {
        ("rest", "resources") => 100,
        ("rest", "filtering") => 98,
        ("rest", "query-params") => 96,
        ("auth", "token") => 95,
        ("auth", "endpoints") => 94,
        ("auth", "user") => 92,
        ("auth", "database") => 90,
        ("rest", "rpc") => 88,
        ("rest", "ordering") => 80,
        ("rest", "prefer") => 72,
        ("rest", "logic") => 64,
        ("rest", "embedding") => 58,
        ("rest", "aggregates") => 52,
        ("auth", "verify") => 48,
        ("auth", "admin") => 40,
        ("rest", "media-types") => 36,
        _ => 10,
    }
}

fn effort(n: usize) -> &'static str {
    match n {
        0..=4 => "S",
        5..=10 => "M",
        11..=20 => "L",
        _ => "XL",
    }
}

fn repo_url(repo: &str) -> &'static str {
    match repo {
        "auth" => "https://github.com/supabase/auth",
        "postgrest" => "https://github.com/PostgREST/postgrest",
        "realtime" => "https://github.com/supabase/realtime",
        "storage" => "https://github.com/supabase/storage",
        "supavisor" => "https://github.com/supabase/supavisor",
        "postgres-meta" => "https://github.com/supabase/postgres-meta",
        "edge-runtime" => "https://github.com/supabase/edge-runtime",
        "supabase" => "https://github.com/supabase/supabase",
        "supabase-js" => "https://github.com/supabase/supabase-js",
        _ => "",
    }
}

fn source_link(u: &Unit, pins: &BTreeMap<String, String>) -> String {
    let base = repo_url(&u.source.repo);
    if base.is_empty() {
        format!("`{}:{}`", u.source.file, u.source.line)
    } else {
        let rev = pins
            .get(&u.source.repo)
            .map(String::as_str)
            .unwrap_or("HEAD");
        format!(
            "[`{}:{}`]({base}/blob/{rev}/{}#L{})",
            u.source.file, u.source.line, u.source.file, u.source.line
        )
    }
}

fn chunk<T>(items: &[T], size: usize) -> Vec<&[T]> {
    items.chunks(size.max(1)).collect()
}

fn unit_rows(units: &[Unit], pins: &BTreeMap<String, String>) -> String {
    let mut s = String::from("| Unit id | Name | Upstream |\n|---|---|---|\n");
    for u in units {
        let _ = writeln!(s, "| `{}` | {} | {} |", u.id, u.name, source_link(u, pins));
    }
    s
}

fn level_label(level: u8) -> &'static str {
    match level {
        1 => "level:1",
        2 => "level:2",
        3 => "level:3",
        4 => "level:4",
        5 => "level:5",
        _ => "level:1",
    }
}

fn component_label_tag(c: &str) -> String {
    format!("component:{c}")
}

pub fn build(file: &UnitsFile) -> Vec<Item> {
    let pins: BTreeMap<String, String> = file
        .vendor
        .iter()
        .map(|p| (p.name.clone(), p.commit.clone()))
        .collect();
    let mut groups: BTreeMap<(u8, String, String), Vec<Unit>> = BTreeMap::new();
    for u in &file.units {
        groups
            .entry((u.level, u.component.clone(), u.group.clone()))
            .or_default()
            .push(u.clone());
    }

    let mut items = Vec::new();
    for ((level, component, group), members) in &groups {
        let label = component_label(component);
        let comp = component_static(component);
        let epic_key = format!("epic:{component}:{group}:{level}");
        let chunks = chunk(members, CHUNK);
        let mut body = String::new();
        let _ = writeln!(body, "<!-- megabase-id: {epic_key} -->");
        let _ = writeln!(
            body,
            "\nPort **{label} / {group}** (Level {level}) so Megabase matches the pinned upstream for these {} units.\n",
            members.len()
        );
        let _ = writeln!(body, "## Context\n");
        let _ = writeln!(
            body,
            "Read the files linked below in `vendor/`. Write `specs/{component}/{group}.md` if it does not exist. One PR per child issue (or one PR for this epic if there are no children).\n"
        );
        let _ = writeln!(body, "## Scope\n");
        let _ = writeln!(body, "**In:** the units in the table. **Out:** other groups, hosted-platform behaviour, pin bumps.\n");
        if *level == 1 {
            let _ = writeln!(body, "## Child issues\n");
            for (i, _) in chunks.iter().enumerate() {
                let _ = writeln!(body, "- [ ] {label}: {group} ({}/{})", i + 1, chunks.len());
            }
            body.push('\n');
        } else {
            let _ = writeln!(body, "## Tasks (one checkbox ≈ one PR)\n");
            for (i, part) in chunks.iter().enumerate() {
                let _ = writeln!(
                    body,
                    "- [ ] PR {}: `{}` … `{}` ({} units)",
                    i + 1,
                    part.first().unwrap().id,
                    part.last().unwrap().id,
                    part.len()
                );
            }
            body.push('\n');
        }
        let _ = writeln!(body, "## Units\n\n{}", unit_rows(members, &pins));
        let _ = writeln!(body, "## Acceptance\n");
        let _ = writeln!(
            body,
            "- Each unit has `// megabase:unit <id>` on the serving code.\n\
             - A judge case in `judge/cases/` lists the id and passes against the reference stack.\n\
             - Coverage and conformance do not drop.\n\
             - Upstream file and license named in the source header.\n"
        );
        let _ = writeln!(body, "## Dependencies\n");
        match (component.as_str(), group.as_str()) {
            ("rest", "filtering")
            | ("rest", "query-params")
            | ("rest", "prefer")
            | ("rest", "rpc") => {
                let _ = writeln!(
                    body,
                    "Blocked by **REST / resources** (table CRUD must exist first)."
                );
            }
            ("auth", "endpoints") | ("auth", "token") | ("auth", "user") => {
                let _ = writeln!(
                    body,
                    "Blocked by **Auth / database** (`auth.users`, `auth.uid()`, `auth.jwt()`)."
                );
            }
            ("auth", "database") => {
                let _ = writeln!(
                    body,
                    "Blocked by **Core: JWT and config**. Blocks Auth endpoints, token and user."
                );
            }
            ("rest", "resources") => {
                let _ = writeln!(body, "Blocked by **Core: JWT and config** (RLS uses the JWT). Blocks the rest of REST.");
            }
            _ => {
                let _ = writeln!(body, "Respect the current level gate in `PROGRESS.md`. Do not start this epic until the previous level is at threshold.");
            }
        }
        let _ = writeln!(
            body,
            "\n**Effort:** {}. **Milestone:** {}. **Labels:** `{}`, `{}`, `type:feature`.",
            effort(members.len()),
            milestone(*level).unwrap_or("—"),
            component_label_tag(component),
            level_label(*level)
        );

        let status = if *level == 1 { "Ready" } else { "Backlog" };
        let blocked_by = blocked_by_keys(component, group);
        items.push(Item {
            key: epic_key.clone(),
            title: format!("{label}: {group}"),
            body,
            labels: vec!["type:feature"],
            milestone: milestone(*level),
            component: comp,
            level: Some(*level),
            effort: effort(members.len()),
            status,
            parent: None,
            blocked_by: blocked_by.clone(),
            priority: usage(component, group) * 10,
        });

        if *level == 1 {
            for (i, part) in chunks.iter().enumerate() {
                let key = format!("task:{component}:{group}:{level}:{}", i + 1);
                let mut b = String::new();
                let _ = writeln!(b, "<!-- megabase-id: {key} -->");
                let _ = writeln!(b, "\nOne PR. Parent epic: **{label}: {group}**.\n");
                let _ = writeln!(
                    b,
                    "## Scope\n\n**In:** the units below. **Out:** everything else in the epic.\n"
                );
                let _ = writeln!(b, "## Units\n\n{}", unit_rows(part, &pins));
                let _ = writeln!(
                    b,
                    "## Acceptance\n\n- Spec in `specs/{component}/` if missing.\n\
                     - `// megabase:unit <id>` on serving code.\n\
                     - Judge cases covering these ids pass.\n\
                     - Commit message: `[{component}] {group}: … (coverage X→Y, conformance A→B)`.\n"
                );
                items.push(Item {
                    key,
                    title: format!("{label}: {group} ({}/{})", i + 1, chunks.len()),
                    body: b,
                    labels: vec!["type:feature"],
                    milestone: milestone(*level),
                    component: comp,
                    level: Some(*level),
                    effort: effort(part.len()),
                    status: "Ready",
                    parent: Some(epic_key.clone()),
                    blocked_by: blocked_by.clone(),
                    priority: usage(component, group) * 10 - i as i32,
                });
            }
        }
    }

    items.extend(infra());
    items.extend(website());
    items
}

fn infra() -> Vec<Item> {
    vec![
        Item {
            key: "epic:core:jwt:1".into(),
            title: "Core: JWT validation and config".into(),
            body: concat!(
                "<!-- megabase-id: epic:core:jwt:1 -->\n\n",
                "Shared `megabase-core` types so Auth and REST can enforce the same JWT and role claims as GoTrue / PostgREST.\n\n",
                "## Scope\n\n**In:** parse and verify the HS256 tokens the demo `JWT_SECRET` issues; `role`, `sub`, `exp`; config from env. **Out:** asymmetric keys (later), JWKS rotation.\n\n",
                "## Acceptance\n\n- Tokens the reference Auth issues are accepted.\n- Forged / expired / `alg=none` tokens are rejected the same way.\n- Judge cases `auth.user.get` and REST RLS cases pass once those units exist.\n- Blocks Auth/database and REST/resources.\n\n",
                "**Effort:** M. **Milestone:** Level 1. **Labels:** `component:core`, `level:1`, `type:infra`.\n"
            )
            .into(),
            labels: vec!["type:infra"],
            milestone: Some("Level 1"),
            component: "core",
            level: Some(1),
            effort: "M",
            status: "Ready",
            parent: None,
            blocked_by: vec![],
            priority: 1100,
        },
        Item {
            key: "epic:judge:side-effects:1".into(),
            title: "Judge: database side-effect checks".into(),
            body: concat!(
                "<!-- megabase-id: epic:judge:side-effects:1 -->\n\n",
                "After mutating cases, snapshot `auth.users` and the public fixtures on both databases and compare. HTTP equality is not enough for sign-up.\n\n",
                "## Scope\n\n**In:** harness support + Auth sign-up / REST insert cases. **Out:** Storage objects (Level 2).\n\n",
                "On a `review/*` branch only ([ADR 0003](docs/adr/0003-protected-paths.md)).\n\n",
                "**Effort:** L. **Milestone:** Level 1. **Labels:** `component:judge`, `level:1`, `type:infra`.\n"
            )
            .into(),
            labels: vec!["type:infra"],
            milestone: Some("Level 1"),
            component: "judge",
            level: Some(1),
            effort: "L",
            status: "Backlog",
            parent: None,
            blocked_by: vec![],
            priority: 0,
        },
        Item {
            key: "epic:judge:hidden:2".into(),
            title: "Judge: hidden / held-out suite".into(),
            body: concat!(
                "<!-- megabase-id: epic:judge:hidden:2 -->\n\n",
                "Private cases the agents cannot read. Weekly run; publish pass/fail counts only. Design in `judge/README.md`.\n\n",
                "On a `review/*` branch. Needs a human-held secret store (human-only action).\n\n",
                "**Effort:** XL. **Labels:** `component:judge`, `type:infra`.\n"
            )
            .into(),
            labels: vec!["type:infra"],
            milestone: None,
            component: "judge",
            level: None,
            effort: "XL",
            status: "Backlog",
            parent: None,
            blocked_by: vec![],
            priority: 0,
        },
        Item {
            key: "epic:judge:studio:4".into(),
            title: "Judge: Studio test harness".into(),
            body: concat!(
                "<!-- megabase-id: epic:judge:studio:4 -->\n\n",
                "GOAL.md section 9: official Studio twice, scripted sessions, compare network, UI errors and DB state.\n\n",
                "**In:** a Rust browser-testing harness (fantoccini or chromiumoxide driving Chromium) in `judge/studio/`. **Out:** rewriting Studio; no non-Rust runner.\n\n",
                "On a `review/*` branch. Blocked by Level 4 Meta/Auth admin/Storage.\n\n",
                "**Effort:** XL. **Milestone:** Level 4. **Labels:** `component:judge`, `level:4`, `type:infra`.\n"
            )
            .into(),
            labels: vec!["type:infra"],
            milestone: Some("Level 4"),
            component: "judge",
            level: Some(4),
            effort: "XL",
            status: "Backlog",
            parent: None,
            blocked_by: vec![],
            priority: 0,
        },
        Item {
            key: "epic:spec:templates:1".into(),
            title: "Specs: per-unit template and Level 1 stubs".into(),
            body: concat!(
                "<!-- megabase-id: epic:spec:templates:1 -->\n\n",
                "GOAL.md section 5: a spec file per unit before implementation. Add `specs/_template.md` and stubs for Ready Level 1 issues.\n\n",
                "**Effort:** S. **Milestone:** Level 1. **Labels:** `type:spec`, `level:1`.\n"
            )
            .into(),
            labels: vec!["type:spec"],
            milestone: Some("Level 1"),
            component: "core",
            level: Some(1),
            effort: "S",
            status: "Ready",
            parent: None,
            blocked_by: vec![],
            priority: 1080,
        },
        Item {
            key: "epic:judge:adversarial:1".into(),
            title: "Judge: adversarial Auth and RLS cases".into(),
            body: concat!(
                "<!-- megabase-id: epic:judge:adversarial:1 -->\n\n",
                "Forged JWTs, `alg=none`, expired tokens, RLS filter injection. Visible cases in `judge/cases/`.\n\n",
                "On a `review/*` branch. Blocks calling Level 1 'done' (no P0 security issues).\n\n",
                "**Effort:** M. **Milestone:** Level 1. **Labels:** `component:judge`, `level:1`, `type:infra`.\n"
            )
            .into(),
            labels: vec!["type:infra"],
            milestone: Some("Level 1"),
            component: "judge",
            level: Some(1),
            effort: "M",
            status: "Backlog",
            parent: None,
            blocked_by: vec![],
            priority: 0,
        },
    ]
}

fn website() -> Vec<Item> {
    let parent = "epic:website:site:later".to_string();
    let mut items = vec![Item {
        key: parent.clone(),
        title: "Website".into(),
        body: concat!(
            "<!-- megabase-id: epic:website:site:later -->\n\n",
            "Public website for the experiment. **Not a compatibility level.** Status embeds `coverage/treemap.svg` / `coverage/treemap-light.svg` (same files as the README). Follows the design-first rule:\n\n",
            "1. Design in [Kite: Megabase identity](https://kite.new/p/megabase-identity).\n",
            "2. LLM committee review (several external models).\n",
            "3. Apply corrections in Kite.\n",
            "4. Implement in this repository so assets match Kite.\n\n",
            "## Scope\n\n**In:** marketing/docs site, social images, diagrams. **Out:** Studio rewrite, product API.\n\n",
            "Child issues below must be done **in order**. Do not implement before the committee signs off.\n\n",
            "**Effort:** XL. **Labels:** `type:feature`. No level milestone.\n"
        )
        .into(),
        labels: vec!["type:feature"],
        milestone: None,
        component: "website",
        level: None,
        effort: "XL",
        status: "Backlog",
        parent: None,
        blocked_by: vec![],
        priority: 0,
    }];
    let steps = [
        ("task:website:design:later", "Website: design in Kite", "Add website frames to https://kite.new/p/megabase-identity (home, status, docs). Palette and logo from the identity file. Stop when a reviewer can click through the Kite file.", Vec::<String>::new()),
        ("task:website:committee:later", "Website: LLM committee review", "Send the Kite frames to several external LLMs. Collect written review. Blocked by design.", vec!["task:website:design:later".into()]),
        ("task:website:revisions:later", "Website: apply committee revisions in Kite", "Update the Kite file. Do not start implementation. Blocked by committee review.", vec!["task:website:committee:later".into()]),
        ("task:website:implement:later", "Website: implement matching Kite", "Implement only what is in Kite after revisions. Repo assets must match. The Status page embeds `coverage/treemap.svg` / `coverage/treemap-light.svg`. Blocked by revisions.", vec!["task:website:revisions:later".into()]),
    ];
    for (i, (key, title, scope, blocked_by)) in steps.iter().enumerate() {
        items.push(Item {
            key: (*key).into(),
            title: (*title).into(),
            body: format!(
                "<!-- megabase-id: {key} -->\n\n{scope}\n\nParent: **Website**. Step {} of 4. Design-first: AGENTS.md.\n\n**Effort:** M. **Labels:** `type:feature`.\n",
                i + 1
            ),
            labels: vec!["type:feature"],
            milestone: None,
            component: "website",
            level: None,
            effort: "M",
            status: "Backlog",
            parent: Some(parent.clone()),
            blocked_by: blocked_by.clone(),
            priority: 0,
        });
    }
    items
}

pub fn render_markdown(items: &[Item]) -> String {
    let mut s = String::from("# Backlog plan\n\nGenerated by `cargo run -p megabase-backlog -- plan` from `coverage/units.json`. Sync to GitHub with `-- sync` (needs issues + project write).\n\n");
    let epics = items.iter().filter(|i| i.parent.is_none()).count();
    let tasks = items.iter().filter(|i| i.parent.is_some()).count();
    let ready = items.iter().filter(|i| i.status == "Ready").count();
    let _ = writeln!(
        s,
        "| Epics | Sub-issues | Ready | Total | Cap |\n|---:|---:|---:|---:|---:|\n| {epics} | {tasks} | {ready} | {} | 150 |\n",
        items.len()
    );
    let mut ready_items: Vec<&Item> = items
        .iter()
        .filter(|i| i.status == "Ready" && i.parent.is_some())
        .collect();
    ready_items.sort_by_key(|i| std::cmp::Reverse(i.priority));
    let _ = writeln!(s, "## Ready (Level 1, usage order)\n");
    for i in &ready_items {
        let _ = writeln!(
            s,
            "1. **{}** — `{}` · effort {} · priority {}",
            i.title, i.key, i.effort, i.priority
        );
    }
    let _ = writeln!(s, "\n## All items\n");
    for i in items {
        let kind = if i.parent.is_some() { "task" } else { "epic" };
        let _ = writeln!(
            s,
            "### {} ({kind})\n\n- Status: {} · Milestone: {} · Effort: {} · Component: {} · Level: {:?}\n",
            i.title,
            i.status,
            i.milestone.unwrap_or("—"),
            i.effort,
            i.component,
            i.level
        );
        s.push_str(&i.body);
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_under_150() {
        let units: Vec<Unit> = (0..800)
            .map(|i| Unit {
                id: format!("rest:x:{i}"),
                component: if i < 176 { "rest" } else { "studio" }.into(),
                group: format!("g{}", i / 20),
                name: "n".into(),
                level: if i < 176 { 1 } else { 5 },
                source: Source {
                    repo: "postgrest".into(),
                    file: "a".into(),
                    line: 1,
                },
            })
            .collect();
        let items = build(&UnitsFile {
            units,
            vendor: vec![],
        });
        assert!(items.len() < 150, "{}", items.len());
        assert!(items.iter().any(|i| i.key == "epic:website:site:later"));
        assert!(items.iter().any(|i| i.title == "Website: design in Kite"));
        let studio = items
            .iter()
            .find(|i| i.key == "epic:judge:studio:4")
            .expect("studio harness");
        assert!(
            studio.body.contains("fantoccini") && studio.body.contains("chromiumoxide"),
            "Studio tests must be Rust browser automation"
        );
        assert!(
            !studio.body.contains("Playwright") && !studio.body.contains("TypeScript"),
            "Studio tests must not specify Playwright or TypeScript"
        );
        let website = items
            .iter()
            .find(|i| i.key == "epic:website:site:later")
            .expect("website epic");
        assert_eq!(website.component, "website");
        assert!(website
            .body
            .contains("Status embeds `coverage/treemap.svg`"));
    }

    #[test]
    fn subissues_and_blocked_by_keys() {
        let mk = |id: &str, group: &str| Unit {
            id: id.into(),
            component: "rest".into(),
            group: group.into(),
            name: "n".into(),
            level: 1,
            source: Source {
                repo: "postgrest".into(),
                file: "a".into(),
                line: 1,
            },
        };
        let items = build(&UnitsFile {
            units: vec![mk("rest:r", "resources"), mk("rest:f", "filtering")],
            vendor: vec![],
        });
        let filtering = items
            .iter()
            .find(|i| i.key == "epic:rest:filtering:1")
            .expect("filtering epic");
        assert_eq!(filtering.blocked_by, ["epic:rest:resources:1"]);
        let child = items
            .iter()
            .find(|i| i.parent.as_deref() == Some("epic:rest:filtering:1"))
            .expect("filtering child");
        assert_eq!(child.blocked_by, ["epic:rest:resources:1"]);
        let committee = items
            .iter()
            .find(|i| i.key == "task:website:committee:later")
            .expect("committee task");
        assert_eq!(committee.parent.as_deref(), Some("epic:website:site:later"));
        assert_eq!(committee.blocked_by, ["task:website:design:later"]);
    }

    #[test]
    fn source_links_use_the_pin_commit() {
        let items = build(&UnitsFile {
            units: vec![Unit {
                id: "auth:route:GET /auth/v1/signup".into(),
                component: "auth".into(),
                group: "endpoints".into(),
                name: "GET /auth/v1/signup".into(),
                level: 1,
                source: Source {
                    repo: "auth".into(),
                    file: "internal/api/api.go".into(),
                    line: 42,
                },
            }],
            vendor: vec![Pin {
                name: "auth".into(),
                commit: "4eee58f296d9698a1c2c0ae14d7a0b379c7622d3".into(),
            }],
        });
        let epic = items
            .iter()
            .find(|i| i.key == "epic:auth:endpoints:1")
            .expect("auth endpoints epic");
        assert!(epic
            .body
            .contains("/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L42"));
        assert!(!epic.body.contains("/blob/HEAD/"));
    }

    #[test]
    fn component_label_and_markdown_plan() {
        assert_eq!(component_label("rest"), "REST");
        assert_eq!(component_label("website"), "Website");
        assert_eq!(component_label("other"), "Megabase");
        let items = build(&UnitsFile {
            units: vec![Unit {
                id: "rest:route:GET /rest/v1/{relation}".into(),
                component: "rest".into(),
                group: "resources".into(),
                name: "GET /rest/v1/{relation}".into(),
                level: 1,
                source: Source {
                    repo: "postgrest".into(),
                    file: "a.hs".into(),
                    line: 1,
                },
            }],
            vendor: vec![],
        });
        let md = render_markdown(&items);
        assert!(md.contains("# Backlog plan"));
        assert!(md.contains("epic:rest:resources:1") || md.contains("REST"));
    }
}
