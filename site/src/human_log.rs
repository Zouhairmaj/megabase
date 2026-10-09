//! Parse `HUMAN_LOG.md` into a count and rendered entries.
//!
//! Completed interventions are the result. The "Pending" section is shown
//! separately and does not increment the public count.

use crate::html::esc;
use crate::markdown;

#[derive(Clone, Debug, Default)]
pub struct HumanLog {
    pub completed: usize,
    pub html: String,
}

pub fn load(md: &str) -> HumanLog {
    let entries = parse(md);
    let completed = entries
        .iter()
        .filter(|e| e.kind == EntryKind::Completed)
        .count();
    HumanLog {
        completed,
        html: render(&entries, completed),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryKind {
    Completed,
    Pending,
}

struct Entry {
    kind: EntryKind,
    date: String,
    action: String,
    reason: String,
    files: String,
}

fn parse(md: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut section = "";
    let mut current: Option<Entry> = None;

    let flush = |current: &mut Option<Entry>, entries: &mut Vec<Entry>| {
        if let Some(entry) = current.take() {
            if !entry.action.is_empty() || !entry.date.is_empty() {
                entries.push(entry);
            }
        }
    };

    for line in md.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") {
            flush(&mut current, &mut entries);
            section = trimmed.trim_start_matches('#').trim();
            continue;
        }
        if trimmed.starts_with("### ") {
            flush(&mut current, &mut entries);
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("- **Date**:") {
            flush(&mut current, &mut entries);
            let kind = if section.eq_ignore_ascii_case("pending") {
                EntryKind::Pending
            } else {
                EntryKind::Completed
            };
            current = Some(Entry {
                kind,
                date: rest.trim().to_string(),
                action: String::new(),
                reason: String::new(),
                files: String::new(),
            });
            continue;
        }
        if let Some(entry) = current.as_mut() {
            if let Some(rest) = trimmed.strip_prefix("- **Action**:") {
                entry.action = rest.trim().to_string();
            } else if let Some(rest) = trimmed.strip_prefix("- **Reason**:") {
                entry.reason = rest.trim().to_string();
            } else if let Some(rest) = trimmed.strip_prefix("- **Files affected**:") {
                entry.files = rest.trim().to_string();
            }
        }
    }
    flush(&mut current, &mut entries);
    entries
        .into_iter()
        .filter(|e| {
            !e.action.is_empty()
                && !e.action.eq_ignore_ascii_case("(pending)")
                && !e.date.eq_ignore_ascii_case("YYYY-MM-DD")
        })
        .collect()
}

fn render(entries: &[Entry], completed: usize) -> String {
    let mut out = String::new();
    let done: Vec<_> = entries
        .iter()
        .filter(|e| e.kind == EntryKind::Completed)
        .collect();
    let pending: Vec<_> = entries
        .iter()
        .filter(|e| e.kind == EntryKind::Pending)
        .collect();

    if done.is_empty() {
        out.push_str(
            r#"<div class="empty-panel">
  <p class="empty-title">No interventions logged.</p>
  <p class="empty-body">An intervention is any action a human takes on the repository or the running agents: a fix, a nudge, a reverted commit, a changed prompt. The count is part of the result.</p>
</div>"#,
        );
    } else {
        out.push_str(r#"<ol class="log-list">"#);
        for entry in done {
            out.push_str(&entry_html(entry));
        }
        out.push_str("</ol>");
    }

    if !pending.is_empty() {
        out.push_str(r#"<h2 class="section-title">Pending</h2><ol class="log-list">"#);
        for entry in pending {
            out.push_str(&entry_html(entry));
        }
        out.push_str("</ol>");
    }

    let _ = completed;
    out
}

fn entry_html(entry: &Entry) -> String {
    let action = if entry.action.is_empty() {
        markdown::inline(&entry.reason)
    } else {
        markdown::inline(&entry.action)
    };
    let reason = if entry.reason.is_empty() {
        String::new()
    } else {
        format!(
            r#"<p class="log-reason">{}</p>"#,
            markdown::inline(&entry.reason)
        )
    };
    let files = if entry.files.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="log-files">Files: {}</p>"#, esc(&entry.files))
    };
    format!(
        r#"<li class="log-entry"><p class="log-meta">{}</p><p class="log-action">{action}</p>{reason}{files}</li>"#,
        esc(&entry.date)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_does_not_count() {
        let md = r#"
## Pending

- **Date**: 2026-10-09
- **Action**: (pending) Turn Pages on.
- **Reason**: Needs a human.
- **Files affected**: GitHub settings

---

*No completed human interventions recorded yet.*
"#;
        let log = load(md);
        assert_eq!(log.completed, 0);
        assert!(log.html.contains("No interventions logged"));
    }

    #[test]
    fn completed_entry_counts() {
        let md = r#"
## Completed

- **Date**: 2026-10-09
- **Action**: Approved Phase 0.
- **Reason**: Bootstrap looked sound.
- **Files affected**: none
"#;
        let log = load(md);
        assert_eq!(log.completed, 1);
        assert!(log.html.contains("Approved Phase 0"));
    }
}
