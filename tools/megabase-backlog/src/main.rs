//! Generate a PM-quality backlog from `coverage/units.json` and optionally
//! sync it to GitHub (milestones, labels, Project "Megabase Backlog").
//!
//!   cargo run -p megabase-backlog -- plan    # write docs/backlog/PLAN.md
//!   cargo run -p megabase-backlog -- sync    # plan + GitHub (needs write)

mod github;
mod plan;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};

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

fn sync(root: &Path) -> Result<()> {
    let units = load_units(root)?;
    let items = build(&units.units);
    write_plan(root, &render_markdown(&items))?;
    let (epics, tasks, total) = github::counts(&items);
    eprintln!("{epics} epics, {tasks} sub-issues, {total} total (cap 150)");
    if total >= 150 {
        bail!("backlog has {total} items; split fewer child issues (cap 150)");
    }

    github::ensure_labels().context("labels")?;
    let milestones = github::ensure_milestones().context("milestones")?;
    let (project_id, project_number) = github::find_or_create_project().context("project")?;
    github::ensure_project_fields(project_number).ok();
    github::ensure_views(&project_id).ok();
    let issues = github::ensure_issues(&items, &milestones).context("issues")?;
    github::add_to_project(project_number, &issues, &items).ok();
    github::set_fields_and_status(&project_id, project_number, &issues, &items).ok();
    eprintln!("synced {total} items to GitHub project {project_number}");
    Ok(())
}

fn main() -> ExitCode {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "plan".into());
    let result = repo_root().and_then(|root| match cmd.as_str() {
        "plan" => {
            let units = load_units(&root)?;
            let items = build(&units.units);
            let (epics, tasks, total) = github::counts(&items);
            eprintln!("{epics} epics, {tasks} sub-issues, {total} total");
            write_plan(&root, &render_markdown(&items))
        }
        "sync" => sync(&root),
        other => bail!("unknown command `{other}` (plan | sync)"),
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
