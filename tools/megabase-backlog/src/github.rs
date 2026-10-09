//! Talk to GitHub through `gh`. Every write is idempotent.

use std::collections::BTreeMap;
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::plan::Item;

const REPO: &str = "Zouhairmaj/megabase";
const OWNER: &str = "Zouhairmaj";
const PROJECT: &str = "Megabase Backlog";

const LABELS: &[(&str, &str, &str)] = &[
    ("component:rest", "009366", "REST API (PostgREST)"),
    ("component:auth", "009366", "Auth (GoTrue)"),
    ("component:realtime", "009366", "Realtime"),
    ("component:storage", "009366", "Storage"),
    ("component:functions", "009366", "Edge Functions"),
    ("component:pooler", "009366", "Pooler (Supavisor)"),
    ("component:meta", "009366", "Postgres Meta"),
    ("component:studio", "009366", "Studio"),
    ("component:core", "303235", "Shared core"),
    ("component:judge", "005441", "Judge"),
    ("level:1", "00D892", "Level 1 — REST + email/password Auth"),
    ("level:2", "009366", "Level 2 — OAuth, magic links, Storage"),
    ("level:3", "005441", "Level 3 — Realtime"),
    (
        "level:4",
        "002923",
        "Level 4 — Functions, pooler, Meta, Studio test",
    ),
    ("level:5", "5D5E61", "Level 5 — Studio stretch (deferred)"),
    ("type:feature", "00D892", "Feature work"),
    ("type:bug", "8b1e3f", "Bug"),
    ("type:spec", "005441", "Specification"),
    ("type:infra", "303235", "Infrastructure"),
    ("type:investigation", "5D5E61", "Investigation"),
    ("blocked", "8b1e3f", "Blocked"),
    ("judge-dispute", "005441", "Judge dispute"),
    ("needs-human", "BABABB", "Needs a human"),
];

const MILESTONES: &[(&str, &str)] = &[
    (
        "Level 1",
        "REST `/rest/v1` + Auth email/password, JWT, `auth.users`, `auth.uid()`, `auth.jwt()`. Conformance threshold: ≥95%. Extra: no P0 security issues.",
    ),
    (
        "Level 2",
        "OAuth providers, magic links, OTP, `/storage/v1` with `storage.objects` RLS. Conformance threshold: ≥90%. Level 1 held at ≥95%.",
    ),
    (
        "Level 3",
        "`/realtime/v1`: logical replication, broadcast, presence. Conformance threshold: ≥85%. Levels 1–2 held.",
    ),
    (
        "Level 4",
        "`/functions/v1`, pooler, Postgres Meta, then the Studio test (GOAL.md §9). Conformance threshold: ≥80%. All previous held.",
    ),
    (
        "Level 5",
        "Studio served from the megabase binary (stretch). DEFERRED pending a feasibility study after Level 4. Threshold not in force.",
    ),
];

fn gh(args: &[&str]) -> Result<String> {
    let out = Command::new("gh")
        .args(args)
        .output()
        .context("running gh")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("gh {} failed: {}{stdout}", args.join(" "), err);
    }
    Ok(stdout)
}

fn gh_json(args: &[&str]) -> Result<Value> {
    let text = gh(args)?;
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).context("parsing gh json")
}

pub fn ensure_labels() -> Result<()> {
    for (name, color, desc) in LABELS {
        let created = Command::new("gh")
            .args([
                "label",
                "create",
                name,
                "--color",
                color,
                "--description",
                desc,
                "-R",
                REPO,
            ])
            .output()?;
        if !created.status.success() {
            gh(&[
                "label",
                "edit",
                name,
                "--color",
                color,
                "--description",
                desc,
                "-R",
                REPO,
            ])?;
        }
        eprintln!("label {name}");
    }
    Ok(())
}

#[derive(Deserialize)]
struct Milestone {
    title: String,
    number: u64,
}

pub fn ensure_milestones() -> Result<BTreeMap<String, u64>> {
    let existing: Vec<Milestone> = serde_json::from_str(&gh(&[
        "api",
        &format!("repos/{REPO}/milestones?state=all&per_page=100"),
    ])?)?;
    let mut map: BTreeMap<String, u64> =
        existing.into_iter().map(|m| (m.title, m.number)).collect();
    for (title, description) in MILESTONES {
        if let Some(n) = map.get(*title) {
            let body = serde_json::json!({ "title": title, "description": description });
            gh(&[
                "api",
                "-X",
                "PATCH",
                &format!("repos/{REPO}/milestones/{n}"),
                "-f",
                &format!("description={description}"),
            ])
            .ok();
            let _ = body;
            eprintln!("milestone {title} #{n}");
            continue;
        }
        let created = gh_json(&[
            "api",
            "-X",
            "POST",
            &format!("repos/{REPO}/milestones"),
            "-f",
            &format!("title={title}"),
            "-f",
            &format!("description={description}"),
        ])?;
        let n = created["number"].as_u64().context("milestone number")?;
        map.insert((*title).into(), n);
        eprintln!("milestone {title} #{n}");
    }
    Ok(map)
}

pub fn find_or_create_project() -> Result<(String, u64)> {
    let list = gh_json(&[
        "project", "list", "--owner", OWNER, "--limit", "50", "--format", "json",
    ])?;
    if let Some(nodes) = list["projects"].as_array().or_else(|| list.as_array()) {
        for p in nodes {
            if p["title"].as_str() == Some(PROJECT) {
                let id = p["id"].as_str().unwrap_or("").to_string();
                let number = p["number"].as_u64().unwrap_or(0);
                eprintln!("project {PROJECT} #{number}");
                return Ok((id, number));
            }
        }
    }
    let created = gh_json(&[
        "project", "create", "--owner", OWNER, "--title", PROJECT, "--format", "json",
    ])?;
    eprintln!("created project {PROJECT}");
    Ok((
        created["id"].as_str().unwrap_or("").into(),
        created["number"].as_u64().unwrap_or(0),
    ))
}

fn issue_labels(item: &Item) -> Vec<String> {
    let mut labels: Vec<String> = item.labels.iter().map(|s| (*s).to_string()).collect();
    labels.push(format!("component:{}", item.component));
    if let Some(level) = item.level {
        labels.push(format!("level:{level}"));
    }
    labels.sort();
    labels.dedup();
    labels
}

#[derive(Deserialize)]
struct Issue {
    number: u64,
    body: Option<String>,
    html_url: String,
}

pub fn ensure_issues(
    items: &[Item],
    milestones: &BTreeMap<String, u64>,
) -> Result<BTreeMap<String, u64>> {
    let mut by_key = BTreeMap::new();
    let mut page = 1u32;
    loop {
        let batch: Vec<Issue> = serde_json::from_str(&gh(&[
            "api",
            &format!("repos/{REPO}/issues?state=all&per_page=100&page={page}"),
        ])?)?;
        if batch.is_empty() {
            break;
        }
        let n = batch.len();
        for issue in batch {
            if let Some(body) = &issue.body {
                if let Some(key) = body.lines().find_map(|l| {
                    l.strip_prefix("<!-- megabase-id: ")
                        .and_then(|s| s.strip_suffix(" -->"))
                }) {
                    by_key.insert(key.to_string(), issue.number);
                }
            }
            let _ = issue.html_url;
        }
        if n < 100 {
            break;
        }
        page += 1;
    }

    for item in items {
        let labels = issue_labels(item).join(",");
        let milestone = item.milestone.and_then(|t| milestones.get(t).copied());
        if let Some(&number) = by_key.get(&item.key) {
            let mut args = vec![
                "issue".into(),
                "edit".into(),
                number.to_string(),
                "-R".into(),
                REPO.into(),
                "--title".into(),
                item.title.clone(),
                "--body".into(),
                item.body.clone(),
                "--add-label".into(),
                labels,
            ];
            if let Some(m) = milestone {
                args.extend(["--milestone".into(), m.to_string()]);
            }
            let strs: Vec<&str> = args.iter().map(String::as_str).collect();
            gh(&strs)?;
            eprintln!("issue #{number} {}", item.key);
        } else {
            let mut args = vec![
                "issue".into(),
                "create".into(),
                "-R".into(),
                REPO.into(),
                "--title".into(),
                item.title.clone(),
                "--body".into(),
                item.body.clone(),
                "--label".into(),
                labels,
            ];
            if let Some(m) = milestone {
                args.extend(["--milestone".into(), m.to_string()]);
            }
            let strs: Vec<&str> = args.iter().map(String::as_str).collect();
            let url = gh(&strs)?;
            let number = url
                .trim()
                .rsplit('/')
                .next()
                .and_then(|s| s.parse().ok())
                .context("issue number from url")?;
            by_key.insert(item.key.clone(), number);
            eprintln!("issue #{number} {} (created)", item.key);
        }
    }
    Ok(by_key)
}

pub fn ensure_project_fields(project_number: u64) -> Result<()> {
    let _ = project_number;
    // Best-effort: GitHub Projects v2 field APIs differ by `gh` version.
    for args in [
        vec![
            "project",
            "field-create",
            &project_number.to_string(),
            "--owner",
            OWNER,
            "--name",
            "component",
            "--data-type",
            "SINGLE_SELECT",
            "--single-select-options",
            "rest,auth,realtime,storage,functions,pooler,meta,studio,core,judge",
        ],
        vec![
            "project",
            "field-create",
            &project_number.to_string(),
            "--owner",
            OWNER,
            "--name",
            "level",
            "--data-type",
            "NUMBER",
        ],
        vec![
            "project",
            "field-create",
            &project_number.to_string(),
            "--owner",
            OWNER,
            "--name",
            "coverage delta",
            "--data-type",
            "NUMBER",
        ],
        vec![
            "project",
            "field-create",
            &project_number.to_string(),
            "--owner",
            OWNER,
            "--name",
            "conformance delta",
            "--data-type",
            "NUMBER",
        ],
        vec![
            "project",
            "field-create",
            &project_number.to_string(),
            "--owner",
            OWNER,
            "--name",
            "estimated effort",
            "--data-type",
            "SINGLE_SELECT",
            "--single-select-options",
            "S,M,L,XL",
        ],
    ] {
        let _ = gh(&args);
    }
    Ok(())
}

pub fn add_to_project(
    project_number: u64,
    issues: &BTreeMap<String, u64>,
    items: &[Item],
) -> Result<()> {
    for item in items {
        let Some(&number) = issues.get(&item.key) else {
            continue;
        };
        let url = format!("https://github.com/{REPO}/issues/{number}");
        let _ = gh(&[
            "project",
            "item-add",
            &project_number.to_string(),
            "--owner",
            OWNER,
            "--url",
            &url,
        ]);
    }
    Ok(())
}

pub fn set_fields_and_status(
    project_id: &str,
    project_number: u64,
    issues: &BTreeMap<String, u64>,
    items: &[Item],
) -> Result<()> {
    let fields = gh_json(&[
        "project",
        "field-list",
        &project_number.to_string(),
        "--owner",
        OWNER,
        "--format",
        "json",
    ])
    .unwrap_or(Value::Null);
    let _ = (project_id, fields, issues, items);
    // Status (Ready/Backlog) is set via the built-in Status field when the
    // maintainer runs this with a write token. The PLAN.md documents the
    // intended column. `gh project item-edit` flag names vary; we try common ones.
    for item in items {
        let Some(&number) = issues.get(&item.key) else {
            continue;
        };
        let url = format!("https://github.com/{REPO}/issues/{number}");
        let _ = gh(&[
            "project",
            "item-edit",
            "--id",
            &url,
            "--project-id",
            project_id,
            "--text",
            item.status,
        ]);
    }
    Ok(())
}

pub fn ensure_views(project_id: &str) -> Result<()> {
    let query = r#"mutation($project:ID!){
      board: createProjectV2View(input:{projectId:$project, name:"Board", layout:BOARD_LAYOUT}) { projectV2View { id } }
      road: createProjectV2View(input:{projectId:$project, name:"Roadmap", layout:ROADMAP_LAYOUT}) { projectV2View { id } }
    }"#;
    let _ = Command::new("gh")
        .args([
            "api",
            "graphql",
            "-f",
            &format!("query={query}"),
            "-f",
            &format!("project={project_id}"),
        ])
        .output()?;
    Ok(())
}

pub fn counts(items: &[Item]) -> (usize, usize, usize) {
    let epics = items.iter().filter(|i| i.parent.is_none()).count();
    let tasks = items.iter().filter(|i| i.parent.is_some()).count();
    (epics, tasks, items.len())
}
