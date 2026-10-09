//! The protected-path policy, as a pure function of the change set.
//! Rationale and the bootstrap exception: `docs/adr/0003-protected-paths.md`.

/// Files only humans edit (GOAL.md section 11, MANIFESTO.md rule 2).
pub const HUMAN_OWNED: &[&str] = &["GOAL.md", "MANIFESTO.md", "HUMAN_LOG.md"];

/// The frozen specification: pinned upstream sources (GOAL.md section 2).
pub const FROZEN: &[&str] = &["vendor/", "vendor.toml", ".gitmodules"];

/// Paths that change only on a `review/*` branch, reviewed by the reviewer
/// agent (GOAL.md rule 2 and section 8). The guard and CI guard themselves.
pub const REVIEWED: &[&str] = &["judge/", ".github/", "tools/megabase-guard/", "CODEOWNERS"];

pub const REVIEW_BRANCH_PREFIX: &str = "review/";

/// The one branch allowed to create the protected tree, and only while the
/// base branch does not have it yet.
pub const BOOTSTRAP_BRANCH: &str = "cursor/phase-0-bootstrap-121c";

/// Source extensions of languages other than Rust (GOAL.md rule 1). SQL and
/// configuration formats are allowed.
pub const NON_RUST_CODE: &[&str] = &[
    "py", "js", "mjs", "cjs", "ts", "tsx", "jsx", "go", "ex", "exs", "hs", "rb", "lua", "php",
    "java", "kt", "swift", "c", "cc", "cpp", "h", "hpp", "sh", "bash", "zsh", "ps1", "pl",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone)]
pub struct Change {
    pub status: Status,
    pub path: String,
}

#[derive(Debug)]
pub struct Context {
    /// Branch the change comes from; empty for a direct push.
    pub head_ref: String,
    /// The base tree has none of `vendor.toml`, `judge/` or vendor gitlinks.
    pub base_is_pre_bootstrap: bool,
    /// Size of `HUMAN_LOG.md` in the head tree, if present.
    pub human_log_size: Option<u64>,
    /// Errors from comparing vendor gitlinks with `vendor.toml` in the head tree.
    pub pin_errors: Vec<String>,
}

fn matches(path: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| match p.strip_suffix('/') {
        Some(dir) => path == dir || path.starts_with(p),
        None => path == *p,
    })
}

pub fn is_bootstrap(ctx: &Context) -> bool {
    ctx.base_is_pre_bootstrap && ctx.head_ref == BOOTSTRAP_BRANCH
}

pub fn evaluate(changes: &[Change], ctx: &Context) -> Vec<String> {
    let bootstrap = is_bootstrap(ctx);
    let review = ctx.head_ref.starts_with(REVIEW_BRANCH_PREFIX);
    let mut violations = Vec::new();
    for change in changes {
        let path = change.path.as_str();
        if matches(path, HUMAN_OWNED) {
            let creating_empty_log = bootstrap
                && path == "HUMAN_LOG.md"
                && change.status == Status::Added
                && ctx.human_log_size == Some(0);
            if !creating_empty_log {
                violations.push(format!(
                    "{path}: human-owned file; only maintainers edit it"
                ));
            }
        } else if matches(path, FROZEN) {
            if !bootstrap {
                violations.push(format!(
                    "{path}: frozen specification; vendor pins never change"
                ));
            }
        } else if matches(path, REVIEWED) && !bootstrap && !review {
            violations.push(format!(
                "{path}: reviewed path; change it on a `{REVIEW_BRANCH_PREFIX}*` branch for the reviewer agent"
            ));
        }
        if change.status != Status::Deleted && !path.starts_with("vendor/") {
            if let Some(ext) = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()) {
                if !path.ends_with('/') && NON_RUST_CODE.contains(&ext.as_str()) {
                    violations.push(format!("{path}: .{ext} source; Megabase code is Rust only"));
                }
            }
        }
    }
    if bootstrap {
        violations.extend(
            ctx.pin_errors
                .iter()
                .map(|e| format!("bootstrap pins: {e}")),
        );
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(path: &str) -> Change {
        Change {
            status: Status::Added,
            path: path.into(),
        }
    }

    fn ctx(head_ref: &str, pre_bootstrap: bool) -> Context {
        Context {
            head_ref: head_ref.into(),
            base_is_pre_bootstrap: pre_bootstrap,
            human_log_size: Some(0),
            pin_errors: Vec::new(),
        }
    }

    #[test]
    fn bootstrap_branch_may_create_protected_tree_once() {
        let changes = [
            add("vendor/auth"),
            add("vendor.toml"),
            add("judge/cases/a.toml"),
            add("HUMAN_LOG.md"),
        ];
        assert!(evaluate(&changes, &ctx(BOOTSTRAP_BRANCH, true)).is_empty());
        // Same branch name after bootstrap landed: no exception.
        assert_eq!(evaluate(&changes, &ctx(BOOTSTRAP_BRANCH, false)).len(), 4);
        // Another branch before bootstrap landed: no exception.
        assert_eq!(evaluate(&changes, &ctx("cursor/other", true)).len(), 4);
    }

    #[test]
    fn bootstrap_never_touches_goal_or_a_non_empty_log() {
        let mut c = ctx(BOOTSTRAP_BRANCH, true);
        assert_eq!(evaluate(&[add("GOAL.md")], &c).len(), 1);
        c.human_log_size = Some(12);
        assert_eq!(evaluate(&[add("HUMAN_LOG.md")], &c).len(), 1);
    }

    #[test]
    fn bootstrap_requires_matching_pins() {
        let mut c = ctx(BOOTSTRAP_BRANCH, true);
        c.pin_errors
            .push("vendor/auth at abc, vendor.toml says def".into());
        assert_eq!(evaluate(&[add("vendor/auth")], &c).len(), 1);
    }

    #[test]
    fn review_branches_may_change_judge_but_not_vendor() {
        let c = ctx("review/judge-fix", false);
        assert!(evaluate(
            &[add("judge/cases/x.toml"), add(".github/workflows/ci.yml")],
            &c
        )
        .is_empty());
        assert_eq!(evaluate(&[add("vendor/auth")], &c).len(), 1);
    }

    #[test]
    fn regular_branches_and_direct_pushes() {
        for head in ["issue-12-rest-filters", ""] {
            let c = ctx(head, false);
            assert!(evaluate(
                &[
                    add("crates/megabase-rest/src/lib.rs"),
                    add("specs/rest/eq.md")
                ],
                &c
            )
            .is_empty());
            assert_eq!(evaluate(&[add("judge/harness/src/main.rs")], &c).len(), 1);
            assert_eq!(evaluate(&[add("MANIFESTO.md")], &c).len(), 1);
        }
    }

    #[test]
    fn rust_only_outside_vendor() {
        let c = ctx("issue-1-x", false);
        assert_eq!(
            evaluate(&[add("scripts/extract.py"), add("tools/x/run.sh")], &c).len(),
            2
        );
        assert!(evaluate(
            &[add("sql/auth.sql"), add("config/x.toml"), add("docs/a.md")],
            &c
        )
        .is_empty());
        let deleted = Change {
            status: Status::Deleted,
            path: "scripts/old.py".into(),
        };
        assert!(evaluate(&[deleted], &c).is_empty());
    }
}
