//! `megabase-guard`: rejects changes to protected paths.
//!
//!   megabase-guard --base <rev> --head <rev> [--head-ref <branch>]
//!
//! `--head-ref` is the branch the change comes from (the pull request head,
//! or the branch of the pull request a pushed commit belongs to). Leave it
//! empty for a direct push. Policy: `docs/adr/0003-protected-paths.md`.

mod policy;

use std::collections::BTreeMap;
use std::process::{Command, ExitCode};

use anyhow::{bail, Context as _, Result};
use serde::Deserialize;

use policy::{Change, Context, Status};

fn git(args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .output()
        .context("running git")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn git_ok(args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

fn changes(base: &str, head: &str) -> Result<Vec<Change>> {
    let out = git(&["diff", "--name-status", "--no-renames", "-z", base, head])?;
    let fields: Vec<&str> = out.split('\0').filter(|s| !s.is_empty()).collect();
    fields
        .chunks(2)
        .map(|pair| {
            let [status, path] = pair else {
                bail!("unexpected git diff output")
            };
            let status = match status.chars().next() {
                Some('A') => Status::Added,
                Some('D') => Status::Deleted,
                _ => Status::Modified,
            };
            Ok(Change {
                status,
                path: path.to_string(),
            })
        })
        .collect()
}

/// `(path, commit)` for every submodule gitlink under `vendor/` in `rev`.
fn gitlinks(rev: &str) -> Result<BTreeMap<String, String>> {
    let out = git(&["ls-tree", "-r", rev, "--", "vendor/"])?;
    Ok(out
        .lines()
        .filter_map(|line| {
            let (meta, path) = line.split_once('\t')?;
            let mut parts = meta.split_whitespace();
            (parts.next()? == "160000")
                .then(|| (path.to_string(), parts.nth(1).unwrap_or("").to_string()))
        })
        .collect())
}

#[derive(Deserialize)]
struct Pins {
    pin: Vec<Pin>,
}

#[derive(Deserialize)]
struct Pin {
    path: String,
    commit: String,
}

fn pin_errors(head: &str) -> Result<Vec<String>> {
    let Ok(text) = git(&["show", &format!("{head}:vendor.toml")]) else {
        return Ok(vec!["vendor.toml is missing".into()]);
    };
    let pins: Pins = toml::from_str(&text).context("parsing vendor.toml")?;
    let links = gitlinks(head)?;
    let mut errors = Vec::new();
    for pin in &pins.pin {
        match links.get(&pin.path) {
            Some(commit) if commit == &pin.commit => {}
            Some(commit) => errors.push(format!(
                "{} is at {commit}, vendor.toml says {}",
                pin.path, pin.commit
            )),
            None => errors.push(format!(
                "{} is in vendor.toml but is not a submodule",
                pin.path
            )),
        }
    }
    for path in links.keys() {
        if !pins.pin.iter().any(|p| &p.path == path) {
            errors.push(format!("{path} is a submodule missing from vendor.toml"));
        }
    }
    Ok(errors)
}

fn run() -> Result<bool> {
    let mut base = None;
    let mut head = "HEAD".to_string();
    let mut head_ref = String::new();
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .with_context(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--base" => base = Some(value),
            "--head" => head = value,
            "--head-ref" => head_ref = value,
            other => bail!("unknown argument `{other}`"),
        }
    }
    let base = base.context("--base is required")?;
    let base = git(&["merge-base", &base, &head])?.trim().to_string();

    let base_is_pre_bootstrap = !git_ok(&["cat-file", "-e", &format!("{base}:vendor.toml")])
        && git(&["ls-tree", &base, "--", "judge"])?.trim().is_empty()
        && gitlinks(&base)?.is_empty();
    let human_log_size = git(&["cat-file", "-s", &format!("{head}:HUMAN_LOG.md")])
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let mut ctx = Context {
        head_ref,
        base_is_pre_bootstrap,
        human_log_size,
        pin_errors: Vec::new(),
    };
    if policy::is_bootstrap(&ctx) {
        ctx.pin_errors = pin_errors(&head)?;
        eprintln!(
            "bootstrap exception active: base {base} predates the protected tree and the head branch is `{}`",
            policy::BOOTSTRAP_BRANCH
        );
    }

    let changes = changes(&base, &head)?;
    let violations = policy::evaluate(&changes, &ctx);
    eprintln!(
        "checked {} changed paths against base {base}",
        changes.len()
    );
    for v in &violations {
        eprintln!("violation: {v}");
    }
    Ok(violations.is_empty())
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}
