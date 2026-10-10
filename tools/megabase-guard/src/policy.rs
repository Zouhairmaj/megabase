//! The protected-path policy, as a pure function of the change set.
//! Rationale, bootstrap exception and release-please exception:
//! `docs/adr/0003-protected-paths.md`.

/// Files only humans edit (GOAL.md section 11, MANIFESTO.md rule 2).
pub const HUMAN_OWNED: &[&str] = &["GOAL.md", "MANIFESTO.md", "HUMAN_LOG.md"];

/// The frozen specification: pinned upstream sources (GOAL.md section 2).
pub const FROZEN: &[&str] = &["vendor/", "vendor.toml", ".gitmodules"];

/// Paths that change only on a `review/*` branch, reviewed by the reviewer
/// agent (GOAL.md rule 2 and section 8). The guard and CI guard themselves.
pub const REVIEWED: &[&str] = &["judge/", ".github/", "tools/megabase-guard/", "CODEOWNERS"];

pub const REVIEW_BRANCH_PREFIX: &str = "review/";

/// release-please opens `release-please--branches--<target>--components--<name>`.
pub const RELEASE_PLEASE_BRANCH_PREFIX: &str = "release-please--branches--";

/// Version-bump files release-please (and the lockfile sync job) may change.
pub const RELEASE_PLEASE_ALLOWED: &[&str] = &[
    "CHANGELOG.md",
    ".release-please-manifest.json",
    "Cargo.toml",
    "Cargo.lock",
];

/// Release config may change on a release-please branch only by deleting
/// `release-as` (see `is_release_as_deletion_only`).
pub const RELEASE_PLEASE_CONFIG: &str = "release-please-config.json";

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
    /// Contents at the merge base, when loaded.
    pub before: Option<String>,
    /// Contents at head, when loaded.
    pub after: Option<String>,
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

pub fn is_release_please(ctx: &Context) -> bool {
    ctx.head_ref.starts_with(RELEASE_PLEASE_BRANCH_PREFIX)
}

/// True when `after` is `before` with only `release-as` keys removed.
pub fn is_release_as_deletion_only(before: &str, after: &str) -> bool {
    let Ok(mut old) = serde_json::from_str::<serde_json::Value>(before) else {
        return false;
    };
    let Ok(new) = serde_json::from_str::<serde_json::Value>(after) else {
        return false;
    };
    if !strip_release_as(&mut old) {
        return false;
    }
    old == new
}

fn strip_release_as(value: &mut serde_json::Value) -> bool {
    let Some(obj) = value.as_object_mut() else {
        return false;
    };
    let mut removed = obj.remove("release-as").is_some();
    if let Some(packages) = obj.get_mut("packages").and_then(|p| p.as_object_mut()) {
        for pkg in packages.values_mut() {
            if let Some(pkg) = pkg.as_object_mut() {
                removed |= pkg.remove("release-as").is_some();
            }
        }
    }
    removed
}

/// True when a `review/*` edit only appends or removes `## Pending` items
/// (prefix grow or shrink) and/or grows the Completed section.
pub fn is_human_log_review_ok(before: &str, after: &str) -> bool {
    const HEADING: &str = "## Pending";
    let Some(old_at) = before.find(HEADING) else {
        return false;
    };
    let Some(new_at) = after.find(HEADING) else {
        return false;
    };
    if before[..old_at] != after[..new_at] {
        return false;
    }
    let Some((old_pending, old_rest)) = split_pending_body(&before[old_at..]) else {
        return false;
    };
    let Some((new_pending, new_rest)) = split_pending_body(&after[new_at..]) else {
        return false;
    };
    let pending_ok = new_pending == old_pending
        || (new_pending.starts_with(old_pending)
            && pending_grow_starts_at_item_boundary(old_pending, new_pending))
        || (old_pending.starts_with(new_pending)
            && pending_shrink_ends_at_item_boundary(old_pending, new_pending));
    pending_ok && is_completed_section_ok(old_rest, new_rest) && after != before
}

fn pending_grow_starts_at_item_boundary(old_pending: &str, new_pending: &str) -> bool {
    new_pending
        .get(old_pending.len()..)
        .is_some_and(|added| added.trim_start_matches('\n').starts_with("- "))
}

fn pending_shrink_ends_at_item_boundary(old_pending: &str, new_pending: &str) -> bool {
    old_pending
        .get(new_pending.len()..)
        .is_some_and(|removed| removed.trim_start_matches('\n').starts_with("- **Date**"))
}

fn is_completed_section_ok(old_rest: &str, new_rest: &str) -> bool {
    if old_rest == new_rest {
        return true;
    }
    let Some(old_body) = completed_body(old_rest) else {
        return false;
    };
    let Some(new_body) = completed_body(new_rest) else {
        return false;
    };
    if new_body == old_body {
        return true;
    }
    if is_empty_completed(old_body) {
        return new_body.starts_with("## Completed");
    }
    let old_trim = old_body.trim_end();
    new_body.starts_with(old_trim) && new_body.len() > old_trim.len()
}

fn completed_body(rest: &str) -> Option<&str> {
    rest.strip_prefix("\n---")
        .map(|body| body.trim_start_matches('\n'))
}

fn is_empty_completed(body: &str) -> bool {
    let trimmed = body.trim();
    trimmed.is_empty() || trimmed == "*No completed human interventions recorded yet.*"
}

/// Suffix added to the Completed section, when that section grew.
fn completed_suffix<'a>(before: &str, after: &'a str) -> Option<&'a str> {
    let old_at = before.find("## Pending")?;
    let new_at = after.find("## Pending")?;
    let (_, old_rest) = split_pending_body(&before[old_at..])?;
    let (_, new_rest) = split_pending_body(&after[new_at..])?;
    let old_body = completed_body(old_rest)?;
    let new_body = completed_body(new_rest)?;
    if old_body == new_body {
        return None;
    }
    if is_empty_completed(old_body) {
        return Some(new_body);
    }
    let old_trim = old_body.trim_end();
    let suffix = new_body.strip_prefix(old_trim)?;
    (!suffix.is_empty()).then_some(suffix)
}

/// A `review/*` mission edit is logged when Completed grew and names `path`.
fn mission_edit_is_logged(path: &str, changes: &[Change]) -> bool {
    let Some(log) = changes.iter().find(|change| change.path == "HUMAN_LOG.md") else {
        return false;
    };
    if log.status != Status::Modified {
        return false;
    }
    let Some((before, after)) = log.before.as_deref().zip(log.after.as_deref()) else {
        return false;
    };
    is_human_log_review_ok(before, after)
        && completed_suffix(before, after).is_some_and(|added| added.contains(path))
}

fn split_pending_body(from_heading: &str) -> Option<(&str, &str)> {
    let nl = from_heading.find('\n')?;
    let after_heading = &from_heading[nl + 1..];
    let end = after_heading.find("\n---")?;
    Some((&after_heading[..end], &after_heading[end..]))
}

fn release_please_config_ok(change: &Change) -> bool {
    change.status == Status::Modified
        && change
            .before
            .as_deref()
            .zip(change.after.as_deref())
            .is_some_and(|(before, after)| is_release_as_deletion_only(before, after))
}

pub fn evaluate(changes: &[Change], ctx: &Context) -> Vec<String> {
    let bootstrap = is_bootstrap(ctx);
    let review = ctx.head_ref.starts_with(REVIEW_BRANCH_PREFIX);
    let release_please = is_release_please(ctx);
    let mut violations = Vec::new();
    for change in changes {
        let path = change.path.as_str();
        if release_please {
            if matches(path, RELEASE_PLEASE_ALLOWED)
                || (path == RELEASE_PLEASE_CONFIG && release_please_config_ok(change))
            {
                continue;
            }
            violations.push(format!(
                "{path}: release-please branches may only change CHANGELOG.md, .release-please-manifest.json, Cargo.toml, Cargo.lock, and delete release-as from {RELEASE_PLEASE_CONFIG}"
            ));
            continue;
        }
        if matches(path, HUMAN_OWNED) {
            let creating_empty_log = bootstrap
                && path == "HUMAN_LOG.md"
                && change.status == Status::Added
                && ctx.human_log_size == Some(0);
            // Phase 0 only: the lead approved landing the Design (Kite)
            // and Documentation sections in GOAL.md on the bootstrap branch.
            let bootstrap_goal = bootstrap && path == "GOAL.md";
            let human_log_review = review
                && path == "HUMAN_LOG.md"
                && change.status == Status::Modified
                && change
                    .before
                    .as_deref()
                    .zip(change.after.as_deref())
                    .is_some_and(|(before, after)| is_human_log_review_ok(before, after));
            let mission_logged = review
                && change.status == Status::Modified
                && (path == "GOAL.md" || path == "MANIFESTO.md")
                && mission_edit_is_logged(path, changes);
            if !creating_empty_log && !bootstrap_goal && !human_log_review && !mission_logged {
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
            before: None,
            after: None,
        }
    }

    fn modified(path: &str, before: &str, after: &str) -> Change {
        Change {
            status: Status::Modified,
            path: path.into(),
            before: Some(before.into()),
            after: Some(after.into()),
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

    fn release_ctx() -> Context {
        ctx(
            "release-please--branches--main--components--megabase",
            false,
        )
    }

    const RELEASE_CONFIG_WITH_AS: &str = r#"{
  "packages": {
    ".": {
      "release-type": "simple",
      "release-as": "0.1.0"
    }
  }
}"#;

    const RELEASE_CONFIG_WITHOUT_AS: &str = r#"{
  "packages": {
    ".": {
      "release-type": "simple"
    }
  }
}"#;

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
    fn bootstrap_never_touches_manifesto_or_a_non_empty_log() {
        let mut c = ctx(BOOTSTRAP_BRANCH, true);
        assert!(evaluate(&[add("GOAL.md")], &c).is_empty());
        assert_eq!(evaluate(&[add("MANIFESTO.md")], &c).len(), 1);
        c.human_log_size = Some(12);
        assert_eq!(evaluate(&[add("HUMAN_LOG.md")], &c).len(), 1);
        assert_eq!(
            evaluate(&[add("GOAL.md")], &ctx(BOOTSTRAP_BRANCH, false)).len(),
            1
        );
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
    fn release_please_branches_may_only_bump_version_files() {
        let c = release_ctx();
        assert!(evaluate(
            &[
                add("CHANGELOG.md"),
                add(".release-please-manifest.json"),
                add("Cargo.toml"),
                add("Cargo.lock"),
            ],
            &c
        )
        .is_empty());
        assert_eq!(evaluate(&[add("crates/megabase/src/main.rs")], &c).len(), 1);
        assert_eq!(evaluate(&[add("vendor/auth")], &c).len(), 1);
        assert_eq!(evaluate(&[add(".github/workflows/ci.yml")], &c).len(), 1);
        assert!(evaluate(&[add("GOAL.md")], &c)[0].contains("release-please"));
    }

    #[test]
    fn release_please_may_delete_release_as_only() {
        let c = release_ctx();
        assert!(evaluate(
            &[modified(
                RELEASE_PLEASE_CONFIG,
                RELEASE_CONFIG_WITH_AS,
                RELEASE_CONFIG_WITHOUT_AS
            )],
            &c
        )
        .is_empty());
        assert!(is_release_as_deletion_only(
            RELEASE_CONFIG_WITH_AS,
            RELEASE_CONFIG_WITHOUT_AS
        ));
    }

    #[test]
    fn release_please_rejects_other_release_please_config_edits() {
        let c = release_ctx();
        let changed_type = r#"{
  "packages": {
    ".": {
      "release-type": "rust"
    }
  }
}"#;
        assert_eq!(
            evaluate(
                &[modified(
                    RELEASE_PLEASE_CONFIG,
                    RELEASE_CONFIG_WITH_AS,
                    changed_type
                )],
                &c
            )
            .len(),
            1
        );
        let added_key = r#"{
  "packages": {
    ".": {
      "release-type": "simple",
      "release-as": "0.1.0",
      "extra": true
    }
  }
}"#;
        assert_eq!(
            evaluate(
                &[modified(
                    RELEASE_PLEASE_CONFIG,
                    RELEASE_CONFIG_WITH_AS,
                    added_key
                )],
                &c
            )
            .len(),
            1
        );
        assert_eq!(evaluate(&[add(RELEASE_PLEASE_CONFIG)], &c).len(), 1);
        assert_eq!(
            evaluate(
                &[modified(
                    RELEASE_PLEASE_CONFIG,
                    RELEASE_CONFIG_WITHOUT_AS,
                    RELEASE_CONFIG_WITHOUT_AS
                )],
                &c
            )
            .len(),
            1
        );
    }

    #[test]
    fn review_branch_may_append_human_log_pending_only() {
        let before = "# Human Intervention Log\n\n## Pending\n\n- old\n\n---\n\n*No completed human interventions recorded yet.*\n";
        let after =
            "# Human Intervention Log\n\n## Pending\n\n- old\n\n- new pending\n\n---\n\n*No completed human interventions recorded yet.*\n";
        let c = ctx("review/release-ci", false);
        assert!(evaluate(&[modified("HUMAN_LOG.md", before, after)], &c).is_empty());
        let rewritten =
            "# Human Intervention Log\n\n## Pending\n\n- rewritten\n\n---\n\n*No completed human interventions recorded yet.*\n";
        assert_eq!(
            evaluate(&[modified("HUMAN_LOG.md", before, rewritten)], &c).len(),
            1
        );
        assert_eq!(
            evaluate(
                &[modified("HUMAN_LOG.md", before, after)],
                &ctx("issue-1-x", false)
            )
            .len(),
            1
        );
        let continuation =
            "# Human Intervention Log\n\n## Pending\n\n- old\n  continuation\n\n---\n\n*No completed human interventions recorded yet.*\n";
        assert_eq!(
            evaluate(&[modified("HUMAN_LOG.md", before, continuation)], &c).len(),
            1
        );
        assert!(pending_grow_starts_at_item_boundary(
            "\n- old\n",
            "\n- old\n\n- new pending\n"
        ));
        assert!(!pending_grow_starts_at_item_boundary(
            "\n- old\n",
            "\n- old\n  continuation\n"
        ));
    }

    #[test]
    fn review_branch_may_complete_human_log_pending() {
        let before = "# Human Intervention Log\n\n## Pending\n\n- **Date**: 2026-10-09\n- **Action**: keep\n\n- **Date**: 2026-10-10\n- **Action**: done\n\n---\n\n*No completed human interventions recorded yet.*\n";
        let after =
            "# Human Intervention Log\n\n## Pending\n\n- **Date**: 2026-10-09\n- **Action**: keep\n\n---\n\n## Completed\n\n- done\n";
        let c = ctx("review/release-ci", false);
        assert!(evaluate(&[modified("HUMAN_LOG.md", before, after)], &c).is_empty());
        let before_placeholder = "# Human Intervention Log\n\n## Pending\n\n- **Date**: 2026-10-09\n- **Action**: keep\n\n---\n\n*No completed human interventions recorded yet.*\n";
        assert!(evaluate(&[modified("HUMAN_LOG.md", before_placeholder, after)], &c).is_empty());
        let rewritten_completed =
            "# Human Intervention Log\n\n## Pending\n\n- **Date**: 2026-10-09\n- **Action**: keep\n\n---\n\n## Completed\n\n- other\n";
        assert_eq!(
            evaluate(&[modified("HUMAN_LOG.md", after, rewritten_completed)], &c).len(),
            1
        );
        let appended_completed =
            "# Human Intervention Log\n\n## Pending\n\n- **Date**: 2026-10-09\n- **Action**: keep\n\n---\n\n## Completed\n\n- done\n\n- later\n";
        assert!(evaluate(&[modified("HUMAN_LOG.md", after, appended_completed)], &c).is_empty());
        assert_eq!(
            evaluate(
                &[modified("HUMAN_LOG.md", before, after)],
                &ctx("issue-1-x", false)
            )
            .len(),
            1
        );
    }

    #[test]
    fn review_branch_pending_shrink_must_end_at_item_boundary() {
        let header = "# Human Intervention Log\n\n## Pending\n\n";
        let rest = "\n---\n\n*No completed human interventions recorded yet.*\n";
        let keep = "- **Date**: 2026-10-09\n- **Action**: keep\n- **Reason**: still open\n";
        let drop = "- **Date**: 2026-10-10\n- **Action**: drop\n- **Reason**: done elsewhere\n";
        let before = format!("{header}{keep}\n{drop}{rest}");
        let shrink = format!("{header}{keep}{rest}");
        let c = ctx("review/release-ci", false);
        assert!(evaluate(&[modified("HUMAN_LOG.md", &before, &shrink)], &c).is_empty());
        assert!(pending_shrink_ends_at_item_boundary(
            &format!("\n{keep}\n{drop}"),
            &format!("\n{keep}")
        ));
        let mid_item = format!("{header}- **Date**: 2026-10-09\n- **Action**: keep\n- **Re{rest}");
        assert_eq!(
            evaluate(&[modified("HUMAN_LOG.md", &before, &mid_item)], &c).len(),
            1
        );
        assert!(!pending_shrink_ends_at_item_boundary(
            keep,
            "- **Date**: 2026-10-09\n- **Action**: keep\n- **Re"
        ));
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
            before: None,
            after: None,
        };
        assert!(evaluate(&[deleted], &c).is_empty());
        assert!(evaluate(
            &[add("vendor/scripts/extract.py")],
            &ctx(BOOTSTRAP_BRANCH, true)
        )
        .is_empty());
        assert_eq!(evaluate(&[add("CODEOWNERS")], &c).len(), 1);
        assert_eq!(
            evaluate(&[add("tools/megabase-guard/src/main.rs")], &c).len(),
            1
        );
        assert!(evaluate(
            &[add("tools/megabase-guard/src/main.rs")],
            &ctx("review/ok", false)
        )
        .is_empty());
    }

    #[test]
    fn release_as_deletion_rejects_invalid_json_and_no_key() {
        assert!(!is_release_as_deletion_only("not json", "{}"));
        assert!(!is_release_as_deletion_only("{}", "not json"));
        assert!(!is_release_as_deletion_only("[]", "[]"));
        assert!(!is_release_as_deletion_only(
            "{\"packages\":{}}",
            "{\"packages\":{}}"
        ));
        assert!(is_release_as_deletion_only(
            r#"{"release-as":"0.1.0"}"#,
            "{}"
        ));
    }

    #[test]
    fn human_log_review_rejects_missing_heading_and_identity() {
        assert!(!is_human_log_review_ok(
            "no heading",
            "## Pending\n\n- x\n\n---\n"
        ));
        assert!(!is_human_log_review_ok(
            "## Pending\n\n- x\n\n---\n",
            "no heading"
        ));
        let same =
            "# H\n\n## Pending\n\n- x\n\n---\n\n*No completed human interventions recorded yet.*\n";
        assert!(!is_human_log_review_ok(same, same));
        let prefix_changed = "# other\n\n## Pending\n\n- x\n\n---\n\n*No completed human interventions recorded yet.*\n";
        assert!(!is_human_log_review_ok(same, prefix_changed));
    }

    fn log_with_completed(completed: &str) -> String {
        format!(
            "# Human Intervention Log\n\n## Pending\n\n- **Date**: 2026-10-09\n- **Action**: keep\n\n---\n\n## Completed\n\n{completed}"
        )
    }

    #[test]
    fn review_branch_may_edit_mission_when_human_log_names_the_file() {
        let before = log_with_completed("- **Date**: 2026-10-09\n- **Action**: earlier\n");
        let after = log_with_completed(
            "- **Date**: 2026-10-09\n- **Action**: earlier\n\n- **Date**: 2026-10-10\n- **Action**: Owner decision. `MANIFESTO.md` and `GOAL.md`.\n",
        );
        let log = modified("HUMAN_LOG.md", &before, &after);
        let c = ctx("review/remove-manifesto-cost", false);
        assert!(evaluate(
            &[
                log.clone(),
                modified("MANIFESTO.md", "old rule", "new rule"),
                modified("GOAL.md", "old goal", "new goal"),
            ],
            &c
        )
        .is_empty());
        assert_eq!(
            evaluate(
                &[
                    log.clone(),
                    modified("MANIFESTO.md", "old rule", "new rule"),
                    modified("GOAL.md", "old goal", "new goal"),
                ],
                &ctx("issue-1-x", false)
            )
            .len(),
            3
        );
        let unnamed = log_with_completed(
            "- **Date**: 2026-10-09\n- **Action**: earlier\n\n- **Date**: 2026-10-10\n- **Action**: Owner decision, no path.\n",
        );
        assert_eq!(
            evaluate(
                &[
                    modified("HUMAN_LOG.md", &before, &unnamed),
                    modified("MANIFESTO.md", "old rule", "new rule"),
                ],
                &c
            )
            .len(),
            1
        );
        let deleted = Change {
            status: Status::Deleted,
            path: "MANIFESTO.md".into(),
            before: None,
            after: None,
        };
        assert_eq!(evaluate(&[log, deleted], &c).len(), 1);
        assert!(completed_suffix(&before, &after).is_some_and(|s| s.contains("MANIFESTO.md")));
        assert!(completed_suffix(&before, &before).is_none());
    }
}
