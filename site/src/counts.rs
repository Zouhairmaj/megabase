//! Build-time public counters for the home stats band.
//!
//! Commit and pull-request totals are read when the site is generated.
//! A shallow clone, a failed `git` call, or a missing `GITHUB_TOKEN`
//! yields `None`, which the page renders as an em dash. The generator
//! never invents a count.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Commit and pull-request totals for the home stats band.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublicCounts {
    pub commits: Option<u64>,
    pub pull_requests: Option<u64>,
}

impl PublicCounts {
    /// Counts that must render as an em dash. Used by unit tests.
    #[cfg(test)]
    pub fn unavailable() -> Self {
        Self {
            commits: None,
            pull_requests: None,
        }
    }

    /// Loads both counters from `repo_root` and, for pull requests, GitHub.
    pub fn load(repo_root: &Path) -> Self {
        Self {
            commits: commit_count(repo_root),
            pull_requests: pull_request_count(),
        }
    }
}

fn commit_count(repo_root: &Path) -> Option<u64> {
    let root = repo_root.to_str()?;
    if shallow_or_unknown(root) {
        return None;
    }
    let output = Command::new("git")
        .args(["-C", root, "rev-list", "--count", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    text.trim().parse().ok()
}

/// A shallow checkout's `rev-list --count` is the shallow depth, not the
/// history on `main`. Refuse that number, and refuse when git cannot say.
fn shallow_or_unknown(root: &str) -> bool {
    let Ok(output) = Command::new("git")
        .args(["-C", root, "rev-parse", "--is-shallow-repository"])
        .output()
    else {
        return true;
    };
    if !output.status.success() {
        return true;
    }
    String::from_utf8(output.stdout)
        .ok()
        .is_none_or(|text| text.trim() != "false")
}

/// Pull-request total from the GitHub search API.
///
/// Runs only outside `cargo test`, so the generator's many fixture builds
/// do not call the network. `GITHUB_TOKEN` is sent as a header and is not
/// logged.
#[cfg(not(test))]
fn pull_request_count() -> Option<u64> {
    let token = std::env::var("GITHUB_TOKEN")
        .ok()
        .filter(|token| !token.is_empty())?;
    let auth = format!("Authorization: Bearer {token}\n");
    let mut child = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "--max-time",
            "15",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "User-Agent: megabase-site",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
            "-H",
            "@-",
            "https://api.github.com/search/issues?q=repo%3AZouhairmaj%2Fmegabase+is%3Apr",
        ])
        .env_remove("GITHUB_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let written = child
        .stdin
        .take()
        .and_then(|mut stdin| stdin.write_all(auth.as_bytes()).ok());
    if written.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    parse_search_total(&text)
}

#[cfg(test)]
fn pull_request_count() -> Option<u64> {
    None
}

/// Reads `total_count` from a GitHub search response.
///
/// Returns `None` when GitHub marks the results incomplete, because the
/// total may then be understated.
pub fn parse_search_total(body: &str) -> Option<u64> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    if value.get("incomplete_results").and_then(|v| v.as_bool()) == Some(true) {
        return None;
    }
    value.get("total_count")?.as_u64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Megabase Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Megabase Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("megabase-site-counts-{tag}-{stamp}"))
    }

    #[test]
    fn search_total_parses_and_rejects_junk() {
        assert_eq!(
            parse_search_total(r#"{"total_count": 106, "items": []}"#),
            Some(106)
        );
        assert_eq!(
            parse_search_total(r#"{"total_count": 106, "incomplete_results": true}"#),
            None
        );
        assert_eq!(parse_search_total("{}"), None);
        assert_eq!(parse_search_total("not json"), None);
    }

    #[test]
    fn full_clone_counts_commits_and_shallow_clone_does_not() {
        let origin = temp_dir("origin");
        fs::create_dir_all(&origin).expect("origin");
        git(&origin, &["init", "-b", "main"]);
        fs::write(origin.join("README"), "a").expect("readme");
        git(&origin, &["add", "README"]);
        git(&origin, &["commit", "-m", "init"]);
        fs::write(origin.join("README"), "b").expect("readme");
        git(&origin, &["add", "README"]);
        git(&origin, &["commit", "-m", "second"]);
        assert_eq!(commit_count(&origin), Some(2));

        let shallow = temp_dir("shallow");
        let url = format!("file://{}", origin.display());
        let status = Command::new("git")
            .args(["clone", "--depth", "1", &url, shallow.to_str().unwrap()])
            .status()
            .expect("clone");
        assert!(status.success(), "shallow clone failed");
        assert_eq!(commit_count(&shallow), None);

        let missing = temp_dir("missing");
        fs::create_dir_all(&missing).expect("missing");
        assert_eq!(commit_count(&missing), None);

        let _ = fs::remove_dir_all(&origin);
        let _ = fs::remove_dir_all(&shallow);
        let _ = fs::remove_dir_all(&missing);
    }
}
