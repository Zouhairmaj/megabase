//! Generate a PM-quality backlog from `coverage/units.json` and optionally
//! sync it to GitHub (milestones, labels, Project "Megabase Backlog").
//!
//!   cargo run -p megabase-backlog -- plan
//!   cargo run -p megabase-backlog -- sync
//!   cargo run -p megabase-backlog -- sync --dry-run

mod github;
mod plan;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};

use github::Sync;
use plan::{build, render_markdown, UnitsFile};

fn repo_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join("GOAL.md").exists() && dir.join("coverage/units.json").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            bail!("run from the Megabase repository");
        }
    }
}

fn load_units(root: &Path) -> Result<UnitsFile> {
    let text = std::fs::read_to_string(root.join("coverage/units.json"))?;
    serde_json::from_str(&text).context("parsing coverage/units.json")
}

fn write_plan(root: &Path, markdown: &str) -> Result<()> {
    let dir = root.join("docs/backlog");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("PLAN.md"), markdown)?;
    eprintln!("wrote docs/backlog/PLAN.md");
    Ok(())
}

fn sync(root: &Path, dry_run: bool) -> Result<()> {
    let units = load_units(root)?;
    let items = build(&units);
    write_plan(root, &render_markdown(&items))?;
    let (epics, tasks, total) = github::counts(&items);
    eprintln!("{epics} epics, {tasks} sub-issues, {total} total (cap 150)");
    if total >= 150 {
        bail!("backlog has {total} items; split fewer child issues (cap 150)");
    }

    let sync = Sync { dry_run };
    if dry_run {
        eprintln!("dry-run: no GitHub writes");
    }

    github::ensure_labels(&sync).context("labels")?;
    let milestones = github::ensure_milestones(&sync).context("milestones")?;
    let mut existing = github::load_issues_by_id().context("list issues")?;
    github::ensure_issues(&sync, &items, &mut existing, &milestones).context("issues")?;
    match github::find_project() {
        Ok((project_id, project_number)) => {
            github::apply_project(&sync, &project_id, &items, &existing)
                .context("project fields")?;
            eprintln!("synced {total} items to GitHub project {project_number}");
        }
        Err(err) => {
            eprintln!("project fields skipped: {err:#}");
            eprintln!(
                "issues were still created/updated; set PROJECT_TOKEN (classic PAT, project scope) to fill the board"
            );
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let cmd = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(String::as_str)
        .unwrap_or("plan");
    let result = repo_root().and_then(|root| match cmd {
        "plan" => {
            let units = load_units(&root)?;
            let items = build(&units);
            let (epics, tasks, total) = github::counts(&items);
            eprintln!("{epics} epics, {tasks} sub-issues, {total} total");
            write_plan(&root, &render_markdown(&items))
        }
        "sync" => sync(&root, dry_run),
        other => bail!("unknown command `{other}` (plan | sync [--dry-run])"),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            eprintln!(
                "docs/backlog/PLAN.md is still the in-repo backlog if GitHub writes are denied."
            );
            ExitCode::FAILURE
        }
    }
}
