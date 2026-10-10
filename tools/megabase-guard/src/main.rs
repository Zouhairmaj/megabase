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
    parse_name_status(&git(&[
        "diff",
        "--name-status",
        "--no-renames",
        "-z",
        base,
        head,
    ])?)
}

fn parse_name_status(out: &str) -> Result<Vec<Change>> {
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
                before: None,
                after: None,
            })
        })
        .collect()
}

/// `(path, commit)` for every submodule gitlink under `vendor/` in `rev`.
fn gitlinks(rev: &str) -> Result<BTreeMap<String, String>> {
    Ok(parse_gitlinks(&git(&[
        "ls-tree", "-r", rev, "--", "vendor/",
    ])?))
}

fn parse_gitlinks(out: &str) -> BTreeMap<String, String> {
    out.lines()
        .filter_map(|line| {
            let (meta, path) = line.split_once('\t')?;
            let mut parts = meta.split_whitespace();
            (parts.next()? == "160000")
                .then(|| (path.to_string(), parts.nth(1).unwrap_or("").to_string()))
        })
        .collect()
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
    pin_errors_from(&text, &gitlinks(head)?)
}

fn pin_errors_from(text: &str, links: &BTreeMap<String, String>) -> Result<Vec<String>> {
    let pins: Pins = toml::from_str(text).context("parsing vendor.toml")?;
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
    let base_tip = base.context("--base is required")?;
    let base = git(&["merge-base", &base_tip, &head])?.trim().to_string();

    let base_is_pre_bootstrap = !git_ok(&["cat-file", "-e", &format!("{base}:vendor.toml")])
        && git(&["ls-tree", &base, "--", "judge"])?.trim().is_empty()
        && gitlinks(&base)?.is_empty();
    let human_log_size = git(&["cat-file", "-s", &format!("{head}:HUMAN_LOG.md")])
        .ok()
        .and_then(|s| s.trim().parse().ok());
    // The base branch tip, not the merge base. A pull request cut from
    // history older than the landing still has `main` as its base ref, and
    // that tip is what closes the exception.
    let base_policy = git(&[
        "show",
        &format!("{base_tip}:tools/megabase-guard/src/policy.rs"),
    ])
    .unwrap_or_default();
    let base_log = git(&["show", &format!("{base_tip}:HUMAN_LOG.md")]).unwrap_or_default();
    let decorative_landing_open = policy::decorative_landing_open(&base_policy, &base_log);
    let mut ctx = Context {
        head_ref,
        base_is_pre_bootstrap,
        human_log_size,
        pin_errors: Vec::new(),
        decorative_landing_open,
    };
    if policy::is_bootstrap(&ctx) {
        ctx.pin_errors = pin_errors(&head)?;
        eprintln!(
            "bootstrap exception active: base {base} predates the protected tree and the head branch is `{}`",
            policy::BOOTSTRAP_BRANCH
        );
    }
    let release_please = policy::is_release_please(&ctx);
    if release_please {
        eprintln!(
            "release-please exception active: branch `{}` may change CHANGELOG.md, .release-please-manifest.json, the [workspace.package] version in Cargo.toml, workspace package versions in Cargo.lock, and delete release-as from release-please-config.json",
            ctx.head_ref
        );
    }

    let mut changes = changes(&base, &head)?;
    for change in &mut changes {
        let version_file =
            release_please && (change.path == "Cargo.toml" || change.path == "Cargo.lock");
        if change.path == policy::RELEASE_PLEASE_CONFIG
            || change.path == "HUMAN_LOG.md"
            || version_file
        {
            change.before = git(&["show", &format!("{}:{}", base, change.path)]).ok();
            change.after = git(&["show", &format!("{}:{}", head, change.path)]).ok();
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_name_status_maps_added_deleted_modified() {
        let out = "A\0crates/a.rs\0D\0old.py\0M\0README.md\0";
        let changes = parse_name_status(out).unwrap();
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].status, Status::Added);
        assert_eq!(changes[0].path, "crates/a.rs");
        assert_eq!(changes[1].status, Status::Deleted);
        assert_eq!(changes[2].status, Status::Modified);
        assert!(parse_name_status("A").is_err());
    }

    #[test]
    fn parse_gitlinks_keeps_only_submodule_mode() {
        let out = "\
160000 commit abcdef vendor/auth\n\
100644 blob 123456 vendor.toml\n\
160000 commit deadbeef\tvendor/postgrest\n";
        let links = parse_gitlinks(out);
        assert_eq!(
            links.get("vendor/postgrest").map(String::as_str),
            Some("deadbeef")
        );
        assert!(!links.contains_key("vendor/auth"));
        assert!(!links.contains_key("vendor.toml"));
    }

    #[test]
    fn pin_errors_from_compares_toml_and_gitlinks() {
        let toml = r#"
[[pin]]
name = "auth"
path = "vendor/auth"
repo = "https://example/auth"
tag = "v1"
commit = "aaa"
license = "MIT"
[[pin]]
name = "postgrest"
path = "vendor/postgrest"
repo = "https://example/p"
tag = "v1"
commit = "bbb"
license = "MIT"
"#;
        let mut links = BTreeMap::new();
        links.insert("vendor/auth".into(), "aaa".into());
        links.insert("vendor/postgrest".into(), "ccc".into());
        links.insert("vendor/extra".into(), "ddd".into());
        let mut errors = pin_errors_from(toml, &links).unwrap();
        let mut expected = vec![
            "vendor/postgrest is at ccc, vendor.toml says bbb".to_string(),
            "vendor/extra is a submodule missing from vendor.toml".to_string(),
        ];
        errors.sort();
        expected.sort();
        assert_eq!(errors, expected);
        links.remove("vendor/extra");
        links.insert("vendor/postgrest".into(), "bbb".into());
        assert!(pin_errors_from(toml, &links).unwrap().is_empty());
        let mut missing = BTreeMap::new();
        missing.insert("vendor/auth".into(), "aaa".into());
        assert_eq!(
            pin_errors_from(toml, &missing).unwrap(),
            vec!["vendor/postgrest is in vendor.toml but is not a submodule".to_string()]
        );
    }
}
