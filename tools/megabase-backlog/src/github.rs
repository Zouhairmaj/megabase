//! Talk to GitHub through `gh`. Writes are idempotent: existing issues are
//! matched by `<!-- megabase-id: … -->` and never recreated.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

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
    ("component:website", "303235", "Public website"),
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

/// Status values the generator may assign. Live columns (In progress, In
/// review, Blocked, Done) are left alone so board-sync can reflect reality.
const GENERATOR_STATUSES: &[&str] = &["Backlog", "Ready"];

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

fn graphql(query: &str, variables: &Value) -> Result<Value> {
    let payload = json!({ "query": query, "variables": variables });
    let mut child = Command::new("gh")
        .args(["api", "graphql", "--input", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn gh api graphql")?;
    child
        .stdin
        .take()
        .context("graphql stdin")?
        .write_all(payload.to_string().as_bytes())?;
    let out = child.wait_with_output().context("gh api graphql")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("graphql failed: {err}{stdout}");
    }
    let v: Value = serde_json::from_str(&stdout).context("parsing graphql json")?;
    if let Some(errors) = v.get("errors").and_then(|e| e.as_array()) {
        if !errors.is_empty() {
            bail!("graphql errors: {errors:?}");
        }
    }
    Ok(v["data"].clone())
}

fn graphql_ok(query: &str, variables: &Value) -> Result<Value> {
    match graphql(query, variables) {
        Ok(v) => Ok(v),
        Err(err) => {
            let msg = format!("{err:#}");
            if already_linked(&msg) {
                Ok(Value::Null)
            } else {
                Err(err)
            }
        }
    }
}

fn already_linked(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("already")
        || m.contains("duplicate")
        || m.contains("sub-issue") && m.contains("exist")
}

pub fn parse_megabase_id(body: &str) -> Option<String> {
    for line in body.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("<!-- megabase-id:") else {
            continue;
        };
        let Some(id) = rest.strip_suffix("-->") else {
            continue;
        };
        let id = id.trim();
        if !id.is_empty() {
            return Some(id.to_string());
        }
    }
    None
}

pub fn counts(items: &[Item]) -> (usize, usize, usize) {
    let epics = items.iter().filter(|i| i.parent.is_none()).count();
    let tasks = items.iter().filter(|i| i.parent.is_some()).count();
    (epics, tasks, items.len())
}

pub fn priority_option(item: &Item) -> &'static str {
    if item.status == "Ready" && item.priority >= 1000 {
        "P0"
    } else if item.status == "Ready" {
        "P1"
    } else if item.level == Some(2) {
        "P2"
    } else {
        "P3"
    }
}

pub fn level_option(item: &Item) -> &'static str {
    match item.level {
        Some(1) => "Level 1",
        Some(2) => "Level 2",
        Some(3) => "Level 3",
        Some(4) => "Level 4",
        Some(5) => "Level 5",
        _ => "No level",
    }
}

/// Whether the generator may write `desired` over `current` Status.
pub fn may_set_status(current: Option<&str>, desired: &str) -> bool {
    if !GENERATOR_STATUSES.contains(&desired) {
        return false;
    }
    match current {
        None => true,
        Some(s) => GENERATOR_STATUSES.iter().any(|g| eq_ignore_space(g, s)),
    }
}

fn eq_ignore_space(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b.trim())
}

pub struct Sync {
    pub dry_run: bool,
}

impl Sync {
    fn note(&self, msg: &str) {
        if self.dry_run {
            eprintln!("dry-run: {msg}");
        } else {
            eprintln!("{msg}");
        }
    }

    fn mutate(&self, desc: &str, query: &str, variables: &Value) -> Result<Value> {
        if self.dry_run {
            eprintln!("dry-run: {desc}");
            return Ok(Value::Null);
        }
        graphql_ok(query, variables)
    }
}

pub fn ensure_labels(sync: &Sync) -> Result<()> {
    for (name, color, desc) in LABELS {
        if sync.dry_run {
            sync.note(&format!("ensure label {name}"));
            continue;
        }
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

pub fn ensure_milestones(sync: &Sync) -> Result<BTreeMap<String, u64>> {
    let existing: Vec<Milestone> = serde_json::from_str(&gh(&[
        "api",
        &format!("repos/{REPO}/milestones?state=all&per_page=100"),
    ])?)?;
    let mut map: BTreeMap<String, u64> =
        existing.into_iter().map(|m| (m.title, m.number)).collect();
    for (title, description) in MILESTONES {
        if let Some(n) = map.get(*title) {
            sync.note(&format!("milestone {title} #{n}"));
            continue;
        }
        if sync.dry_run {
            sync.note(&format!("would create milestone {title}"));
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

pub fn find_project() -> Result<(String, u64)> {
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
    bail!("GitHub Project `{PROJECT}` not found for {OWNER}")
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

fn managed_label(name: &str) -> bool {
    name.starts_with("component:") || name.starts_with("level:") || name.starts_with("type:")
}

fn reconcile_labels(existing: &BTreeSet<String>, wanted: &[String]) -> Vec<String> {
    let mut labels: BTreeSet<String> = existing
        .iter()
        .filter(|name| !managed_label(name))
        .cloned()
        .collect();
    labels.extend(wanted.iter().cloned());
    let mut out: Vec<String> = labels.into_iter().collect();
    out.sort();
    out
}

#[derive(Debug, Clone)]
pub struct ExistingIssue {
    pub number: u64,
    pub node_id: String,
    body: String,
    labels: BTreeSet<String>,
    milestone: Option<String>,
    state: String,
}

#[derive(Deserialize)]
struct LabelName {
    name: String,
}

#[derive(Deserialize)]
struct MilestoneRef {
    title: String,
}

#[derive(Deserialize)]
struct IssueRow {
    number: u64,
    node_id: String,
    body: Option<String>,
    pull_request: Option<Value>,
    #[serde(default)]
    labels: Vec<LabelName>,
    milestone: Option<MilestoneRef>,
    state: String,
}

pub fn load_issues_by_id() -> Result<BTreeMap<String, ExistingIssue>> {
    let mut by_key = BTreeMap::new();
    let mut page = 1u32;
    loop {
        let batch: Vec<IssueRow> = serde_json::from_str(&gh(&[
            "api",
            &format!("repos/{REPO}/issues?state=all&per_page=100&page={page}"),
        ])?)?;
        if batch.is_empty() {
            break;
        }
        let n = batch.len();
        for issue in batch {
            if issue.pull_request.is_some() {
                continue;
            }
            if let Some(body) = &issue.body {
                if let Some(key) = parse_megabase_id(body) {
                    by_key.insert(
                        key,
                        ExistingIssue {
                            number: issue.number,
                            node_id: issue.node_id,
                            body: body.clone(),
                            labels: issue.labels.into_iter().map(|l| l.name).collect(),
                            milestone: issue.milestone.map(|m| m.title),
                            state: issue.state,
                        },
                    );
                }
            }
        }
        if n < 100 {
            break;
        }
        page += 1;
    }
    eprintln!("matched {} existing issues by megabase-id", by_key.len());
    Ok(by_key)
}

fn patch_issue(number: u64, body: &str, labels: &[String], milestone: Option<u64>) -> Result<()> {
    let payload = json!({
        "body": body,
        "labels": labels,
        "milestone": milestone,
    });
    let mut child = Command::new("gh")
        .args([
            "api",
            "-X",
            "PATCH",
            &format!("repos/{REPO}/issues/{number}"),
            "--input",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn gh issue patch")?;
    child
        .stdin
        .take()
        .context("patch stdin")?
        .write_all(payload.to_string().as_bytes())?;
    let out = child.wait_with_output().context("gh issue patch")?;
    if !out.status.success() {
        bail!(
            "PATCH issues/{number} failed: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
    }
    Ok(())
}

/// Create missing issues, update bodies/labels/milestones on matches, and
/// close generated issues whose megabase-id is no longer in the plan.
pub fn ensure_issues(
    sync: &Sync,
    items: &[Item],
    existing: &mut BTreeMap<String, ExistingIssue>,
    milestones: &BTreeMap<String, u64>,
) -> Result<()> {
    let wanted: BTreeSet<&str> = items.iter().map(|i| i.key.as_str()).collect();
    let mut created = 0usize;
    let mut updated = 0usize;
    for item in items {
        let labels = issue_labels(item);
        let milestone_title = item.milestone.map(str::to_string);
        let milestone = item.milestone.and_then(|t| milestones.get(t).copied());
        if let Some(ex) = existing.get(&item.key) {
            let labels = reconcile_labels(&ex.labels, &labels);
            let label_set: BTreeSet<String> = labels.iter().cloned().collect();
            if ex.body == item.body && ex.labels == label_set && ex.milestone == milestone_title {
                continue;
            }
            if sync.dry_run {
                sync.note(&format!("would update #{} {}", ex.number, item.key));
                updated += 1;
                continue;
            }
            patch_issue(ex.number, &item.body, &labels, milestone)?;
            eprintln!("issue #{} {} (updated)", ex.number, item.key);
            updated += 1;
            continue;
        }
        if sync.dry_run {
            sync.note(&format!("would create {} ({})", item.title, item.key));
            created += 1;
            continue;
        }
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
            labels.join(","),
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
        let row: Value =
            serde_json::from_str(&gh(&["api", &format!("repos/{REPO}/issues/{number}")])?)?;
        let node_id = row["node_id"]
            .as_str()
            .context("issue node_id")?
            .to_string();
        existing.insert(
            item.key.clone(),
            ExistingIssue {
                number,
                node_id,
                body: item.body.clone(),
                labels: labels.iter().cloned().collect(),
                milestone: milestone_title,
                state: "open".to_string(),
            },
        );
        created += 1;
        eprintln!("issue #{number} {} (created)", item.key);
    }
    let mut closed = 0usize;
    for (key, ex) in existing.iter() {
        if wanted.contains(key.as_str()) {
            continue;
        }
        if !(key.starts_with("epic:") || key.starts_with("task:")) {
            continue;
        }
        if ex.state != "open" {
            continue;
        }
        if sync.dry_run {
            sync.note(&format!(
                "would close #{} {key} (no longer in plan)",
                ex.number
            ));
            closed += 1;
            continue;
        }
        gh(&[
            "issue",
            "close",
            &ex.number.to_string(),
            "-R",
            REPO,
            "--reason",
            "not_planned",
            "--comment",
            "Superseded: this megabase-id is no longer in the generated plan (units moved between levels). See docs/backlog/PLAN.md.",
        ])?;
        eprintln!("issue #{} {key} (closed)", ex.number);
        closed += 1;
    }
    eprintln!("{created} created, {updated} updated, {closed} closed");
    Ok(())
}

struct SelectField {
    id: String,
    options: BTreeMap<String, String>,
}

impl SelectField {
    fn option(&self, name: &str) -> Result<&str> {
        self.options
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
            .with_context(|| {
                format!(
                    "option `{name}` missing (have {})",
                    self.options.keys().cloned().collect::<Vec<_>>().join(", ")
                )
            })
    }
}

struct Schema {
    status: SelectField,
    level: SelectField,
    component: SelectField,
    size: SelectField,
    priority: SelectField,
}

fn as_select(node: &Value) -> Option<SelectField> {
    let id = node["id"].as_str()?.to_string();
    let mut options = BTreeMap::new();
    for opt in node["options"].as_array()? {
        let name = opt["name"].as_str()?.to_string();
        let oid = opt["id"].as_str()?.to_string();
        options.insert(name.to_ascii_lowercase(), oid);
    }
    Some(SelectField { id, options })
}

fn load_schema(project_id: &str) -> Result<Schema> {
    let data = graphql(
        r#"query($id: ID!) {
          node(id: $id) {
            ... on ProjectV2 {
              fields(first: 40) {
                nodes {
                  ... on ProjectV2SingleSelectField {
                    id
                    name
                    options { id name }
                  }
                }
              }
            }
          }
        }"#,
        &json!({ "id": project_id }),
    )?;
    let mut status = None;
    let mut level = None;
    let mut component = None;
    let mut size = None;
    let mut priority = None;
    for node in data["node"]["fields"]["nodes"]
        .as_array()
        .context("project fields")?
    {
        let name = node["name"].as_str().unwrap_or("");
        let Some(field) = as_select(node) else {
            continue;
        };
        match name {
            "Status" => status = Some(field),
            "Level" => level = Some(field),
            "Component" => component = Some(field),
            "Size" => size = Some(field),
            "Priority" => priority = Some(field),
            _ => {}
        }
    }
    let status = status.context("Status field")?;
    status
        .option("Blocked")
        .context("Status must include Blocked")?;
    for required in ["Backlog", "Ready", "In progress", "In review", "Done"] {
        status.option(required)?;
    }
    Ok(Schema {
        status,
        level: level.context("Level field")?,
        component: component.context("Component field")?,
        size: size.context("Size field")?,
        priority: priority.context("Priority field")?,
    })
}

struct ProjectItem {
    id: String,
    status: Option<String>,
}

fn load_project_items(project_id: &str) -> Result<BTreeMap<u64, ProjectItem>> {
    let mut map = BTreeMap::new();
    let mut cursor: Option<String> = None;
    loop {
        let data = graphql(
            r#"query($id: ID!, $cursor: String) {
              node(id: $id) {
                ... on ProjectV2 {
                  items(first: 50, after: $cursor) {
                    pageInfo { hasNextPage endCursor }
                    nodes {
                      id
                      fieldValueByName(name: "Status") {
                        ... on ProjectV2ItemFieldSingleSelectValue { name }
                      }
                      content { ... on Issue { number } }
                    }
                  }
                }
              }
            }"#,
            &json!({ "id": project_id, "cursor": cursor }),
        )?;
        let conn = &data["node"]["items"];
        for node in conn["nodes"].as_array().context("project items")? {
            let Some(number) = node["content"]["number"].as_u64() else {
                continue;
            };
            map.insert(
                number,
                ProjectItem {
                    id: node["id"].as_str().unwrap_or("").to_string(),
                    status: node["fieldValueByName"]["name"]
                        .as_str()
                        .map(str::to_string),
                },
            );
        }
        if conn["pageInfo"]["hasNextPage"].as_bool() != Some(true) {
            break;
        }
        cursor = conn["pageInfo"]["endCursor"].as_str().map(str::to_string);
    }
    Ok(map)
}

const SET_SELECT: &str = r#"
mutation($projectId: ID!, $itemId: ID!, $fieldId: ID!, $optionId: String!) {
  updateProjectV2ItemFieldValue(input: {
    projectId: $projectId
    itemId: $itemId
    fieldId: $fieldId
    value: { singleSelectOptionId: $optionId }
  }) { projectV2Item { id } }
}"#;

const ADD_ITEM: &str = r#"
mutation($projectId: ID!, $contentId: ID!) {
  addProjectV2ItemById(input: { projectId: $projectId, contentId: $contentId }) {
    item { id }
  }
}"#;

const ADD_SUB: &str = r#"
mutation($issueId: ID!, $subIssueId: ID!) {
  addSubIssue(input: { issueId: $issueId, subIssueId: $subIssueId }) {
    subIssue { number }
  }
}"#;

const ADD_BLOCKED: &str = r#"
mutation($issueId: ID!, $blockingIssueId: ID!) {
  addBlockedBy(input: { issueId: $issueId, blockingIssueId: $blockingIssueId }) {
    issue { number }
  }
}"#;

fn set_select(
    sync: &Sync,
    project_id: &str,
    item_id: &str,
    field: &SelectField,
    option: &str,
    desc: &str,
) -> Result<()> {
    let option_id = field.option(option)?;
    sync.mutate(
        desc,
        SET_SELECT,
        &json!({
            "projectId": project_id,
            "itemId": item_id,
            "fieldId": field.id,
            "optionId": option_id,
        }),
    )?;
    Ok(())
}

pub fn apply_project(
    sync: &Sync,
    project_id: &str,
    items: &[Item],
    existing: &BTreeMap<String, ExistingIssue>,
) -> Result<()> {
    let schema = load_schema(project_id)?;
    let mut on_board = if sync.dry_run {
        load_project_items(project_id).unwrap_or_default()
    } else {
        load_project_items(project_id)?
    };

    for item in items {
        let Some(issue) = existing.get(&item.key) else {
            continue;
        };
        if let std::collections::btree_map::Entry::Vacant(slot) = on_board.entry(issue.number) {
            let data = sync.mutate(
                &format!("add #{} to project", issue.number),
                ADD_ITEM,
                &json!({ "projectId": project_id, "contentId": issue.node_id }),
            )?;
            if let Some(id) = data["addProjectV2ItemById"]["item"]["id"].as_str() {
                slot.insert(ProjectItem {
                    id: id.to_string(),
                    status: None,
                });
            }
        }
    }
    if !sync.dry_run {
        on_board = load_project_items(project_id)?;
    }

    for item in items {
        let Some(issue) = existing.get(&item.key) else {
            continue;
        };
        let Some(row) = on_board.get(&issue.number) else {
            if sync.dry_run {
                sync.note(&format!(
                    "would set fields on #{} ({})",
                    issue.number, item.key
                ));
            }
            continue;
        };
        let current = row.status.as_deref();
        if may_set_status(current, item.status) {
            set_select(
                sync,
                project_id,
                &row.id,
                &schema.status,
                item.status,
                &format!("#{} Status → {}", issue.number, item.status),
            )?;
        } else {
            sync.note(&format!(
                "#{} Status {} left unchanged (not Backlog/Ready)",
                issue.number,
                current.unwrap_or("(none)")
            ));
        }
        set_select(
            sync,
            project_id,
            &row.id,
            &schema.level,
            level_option(item),
            &format!("#{} Level", issue.number),
        )?;
        set_select(
            sync,
            project_id,
            &row.id,
            &schema.component,
            item.component,
            &format!("#{} Component", issue.number),
        )?;
        set_select(
            sync,
            project_id,
            &row.id,
            &schema.size,
            item.effort,
            &format!("#{} Size", issue.number),
        )?;
        set_select(
            sync,
            project_id,
            &row.id,
            &schema.priority,
            priority_option(item),
            &format!("#{} Priority", issue.number),
        )?;
    }

    let mut linked = BTreeSet::new();
    for item in items {
        let Some(parent_key) = &item.parent else {
            continue;
        };
        let Some(child) = existing.get(&item.key) else {
            continue;
        };
        let Some(parent) = existing.get(parent_key) else {
            continue;
        };
        let pair = (parent.number, child.number);
        if !linked.insert(pair) {
            continue;
        }
        if let Err(err) = sync.mutate(
            &format!("sub-issue #{} ← #{}", parent.number, child.number),
            ADD_SUB,
            &json!({
                "issueId": parent.node_id,
                "subIssueId": child.node_id,
            }),
        ) {
            eprintln!("sub-issue #{} ← #{}: {err:#}", parent.number, child.number);
        }
    }

    for item in items {
        let Some(blocked) = existing.get(&item.key) else {
            continue;
        };
        for blocker_key in &item.blocked_by {
            let Some(blocker) = existing.get(blocker_key) else {
                continue;
            };
            if let Err(err) = sync.mutate(
                &format!("#{} blocked-by #{}", blocked.number, blocker.number),
                ADD_BLOCKED,
                &json!({
                    "issueId": blocked.node_id,
                    "blockingIssueId": blocker.node_id,
                }),
            ) {
                eprintln!(
                    "blocked-by #{} ← #{}: {err:#}",
                    blocked.number, blocker.number
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn megabase_id_from_html_comment() {
        let body = "<!-- megabase-id: epic:auth:admin:1 -->\n\nPort **Auth**.";
        assert_eq!(
            parse_megabase_id(body).as_deref(),
            Some("epic:auth:admin:1")
        );
    }

    #[test]
    fn megabase_id_tolerates_spaces() {
        let body = "  <!-- megabase-id: task:rest:resources:1:1 -->  ";
        assert_eq!(
            parse_megabase_id(body).as_deref(),
            Some("task:rest:resources:1:1")
        );
    }

    #[test]
    fn megabase_id_skips_leading_text() {
        let body = "Title line\n\n<!-- megabase-id: epic:website:site:later -->\n";
        assert_eq!(
            parse_megabase_id(body).as_deref(),
            Some("epic:website:site:later")
        );
    }

    #[test]
    fn generator_does_not_clobber_live_status() {
        assert!(may_set_status(None, "Ready"));
        assert!(may_set_status(Some("Backlog"), "Ready"));
        assert!(may_set_status(Some("Ready"), "Backlog"));
        assert!(!may_set_status(Some("In progress"), "Ready"));
        assert!(!may_set_status(Some("Blocked"), "Backlog"));
        assert!(!may_set_status(Some("Done"), "Ready"));
        assert!(!may_set_status(Some("In review"), "Ready"));
        assert!(may_set_status(Some(" ready "), "Ready"));
        assert!(!may_set_status(None, "In progress"));
    }

    #[test]
    fn megabase_id_skips_malformed_and_empty() {
        assert!(parse_megabase_id("no comment").is_none());
        assert!(parse_megabase_id("<!-- megabase-id: -->").is_none());
        assert!(parse_megabase_id("<!-- megabase-id: x").is_none());
    }

    #[test]
    fn counts_and_project_options() {
        let epic = Item {
            key: "epic:rest:resources:1".into(),
            title: "t".into(),
            body: "b".into(),
            labels: vec![],
            milestone: None,
            component: "rest",
            level: Some(1),
            effort: "M",
            status: "Ready",
            parent: None,
            blocked_by: vec![],
            priority: 1000,
        };
        let task = Item {
            parent: Some("epic:rest:resources:1".into()),
            status: "Backlog",
            level: Some(2),
            priority: 1,
            ..epic.clone()
        };
        assert_eq!(counts(&[epic.clone(), task.clone()]), (1, 1, 2));
        assert_eq!(priority_option(&epic), "P0");
        let p1 = Item {
            priority: 10,
            ..epic.clone()
        };
        assert_eq!(priority_option(&p1), "P1");
        assert_eq!(priority_option(&task), "P2");
        let p3 = Item {
            status: "Backlog",
            level: Some(3),
            ..epic.clone()
        };
        assert_eq!(priority_option(&p3), "P3");
        assert_eq!(level_option(&epic), "Level 1");
        assert_eq!(level_option(&task), "Level 2");
        for (n, label) in [(3, "Level 3"), (4, "Level 4"), (5, "Level 5")] {
            let item = Item {
                level: Some(n),
                ..epic.clone()
            };
            assert_eq!(level_option(&item), label);
        }
        let none = Item {
            level: None,
            ..epic
        };
        assert_eq!(level_option(&none), "No level");
    }

    #[test]
    fn already_linked_matches_idempotent_errors() {
        assert!(already_linked("already exists"));
        assert!(already_linked("Duplicate sub-issue"));
        assert!(already_linked("sub-issue does exist"));
        assert!(!already_linked("permission denied"));
    }
}
