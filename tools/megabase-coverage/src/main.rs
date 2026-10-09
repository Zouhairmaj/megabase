//! `megabase-coverage`: the coverage denominator, unit states, treemaps,
//! badges and generated documentation blocks.
//!
//! ```text
//! cargo run -p megabase-coverage -- update   # regenerate everything
//! cargo run -p megabase-coverage -- check    # fail if anything committed is stale
//! cargo run -p megabase-coverage -- verify-pins
//! ```

mod banner;
mod extract;
mod glyphs;
mod kong;
mod model;
mod render;
mod scan;
mod status;
mod treemap;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{bail, Context, Result};

fn repo_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join("vendor.toml").exists() && dir.join("GOAL.md").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            bail!("run inside the Megabase repository (no vendor.toml found)");
        }
    }
}

/// Every submodule must be checked out at exactly the commit `vendor.toml`
/// records, both in the index (gitlink) and on disk.
fn verify_pins(root: &Path) -> Result<()> {
    let pins = extract::load_pins(root)?;
    let tree = Command::new("git")
        .args(["ls-tree", "HEAD", "vendor/"])
        .current_dir(root)
        .output()
        .context("running git ls-tree")?;
    let tree = String::from_utf8_lossy(&tree.stdout).to_string();
    let mut problems = Vec::new();
    let mut gitlinks = 0;
    for line in tree.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() == 4 && fields[1] == "commit" {
            gitlinks += 1;
            match pins.iter().find(|p| p.path == fields[3]) {
                Some(p) if p.commit == fields[2] => {}
                Some(p) => problems.push(format!(
                    "{} is pinned to {} in git but {} in vendor.toml",
                    p.path, fields[2], p.commit
                )),
                None => problems.push(format!("{} is not listed in vendor.toml", fields[3])),
            }
        }
    }
    if gitlinks != pins.len() {
        problems.push(format!(
            "vendor.toml lists {} pins but git has {gitlinks} submodules",
            pins.len()
        ));
    }
    for pin in &pins {
        let head = Command::new("git")
            .args(["-C", &pin.path, "rev-parse", "HEAD"])
            .current_dir(root)
            .output();
        if let Ok(out) = head {
            let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if out.status.success() && !head.is_empty() && head != pin.commit {
                problems.push(format!(
                    "{} is checked out at {head}, expected {}",
                    pin.path, pin.commit
                ));
            }
        }
    }
    if !problems.is_empty() {
        bail!(
            "vendor pins do not match vendor.toml:\n  {}",
            problems.join("\n  ")
        );
    }
    Ok(())
}

fn build(root: &Path) -> Result<render::Outputs> {
    verify_pins(root)?;
    let units = extract::extract(root)?;
    let status = status::compute(root, &units)?;
    let summary = status::summarize(&units, &status);
    render::render(&units, &status, &summary)
}

fn run() -> Result<bool> {
    let root = repo_root()?;
    let command = std::env::args().nth(1).unwrap_or_default();
    match command.as_str() {
        "update" => {
            let outputs = build(&root)?;
            let changed = render::apply(&root, &outputs, true)?;
            for path in &changed {
                println!("updated {path}");
            }
            let units = &outputs.files[Path::new("coverage/units.json")];
            let parsed: model::UnitsFile = serde_json::from_str(units)?;
            println!("{} units, {} excluded", parsed.total, parsed.excluded.len());
            Ok(true)
        }
        "check" => {
            let outputs = build(&root)?;
            let drift = render::apply(&root, &outputs, false)?;
            if drift.is_empty() {
                println!("coverage/, README status, docs and epics are up to date");
                Ok(true)
            } else {
                eprintln!("These generated files are stale:");
                for path in drift {
                    eprintln!("  {path}");
                }
                eprintln!("Run `cargo run -p megabase-coverage -- update` and commit the result.");
                Ok(false)
            }
        }
        "verify-pins" => {
            verify_pins(&root)?;
            println!("vendor/ matches vendor.toml");
            Ok(true)
        }
        _ => {
            eprintln!("usage: megabase-coverage <update|check|verify-pins>");
            Ok(false)
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}
