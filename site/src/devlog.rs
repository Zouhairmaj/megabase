//! Devlog entries from `devlog/YYYY-MM-DD.md`.

use std::fs;
use std::io;
use std::path::Path;

use crate::html::esc;
use crate::markdown;

#[derive(Clone, Debug)]
pub struct Entry {
    pub date: String,
    pub slug: String,
    pub title: String,
    pub day: Option<u32>,
    pub summary: String,
    pub body_html: String,
}

pub fn load(repo_root: &Path) -> io::Result<Vec<Entry>> {
    let dir = repo_root.join("devlog");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for file in fs::read_dir(&dir)? {
        let file = file?;
        let path = file.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if !is_date_slug(&stem) {
            continue;
        }
        let md = fs::read_to_string(&path)?;
        entries.push(parse(&stem, &md));
    }
    entries.sort_by(|a, b| b.date.cmp(&a.date));
    Ok(entries)
}

fn is_date_slug(stem: &str) -> bool {
    let bytes = stem.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes.iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                true
            } else {
                b.is_ascii_digit()
            }
        })
}

fn parse(date: &str, md: &str) -> Entry {
    let mut title = format!("Devlog · {date}");
    let mut day = None;
    let mut body = md;
    if let Some(rest) = md
        .strip_prefix("---\n")
        .or_else(|| md.strip_prefix("---\r\n"))
    {
        if let Some(end) = rest.find("\n---\n").or_else(|| rest.find("\n---\r\n")) {
            let fm = &rest[..end];
            for line in fm.lines() {
                if let Some(v) = line.strip_prefix("title:") {
                    title = v.trim().trim_matches('"').to_string();
                }
                if let Some(v) = line.strip_prefix("day:") {
                    day = v.trim().parse().ok();
                }
            }
            body = rest[end + 5..].trim_start();
        }
    }
    let mut lines: Vec<&str> = body.lines().collect();
    if let Some(first) = lines.first().copied() {
        if let Some(heading) = first.strip_prefix("# ") {
            title = heading.trim().to_string();
            lines.remove(0);
            body = body[first.len()..].trim_start_matches(['\n', '\r']);
        }
    }
    let summary = first_paragraph(body);
    Entry {
        date: date.to_string(),
        slug: date.to_string(),
        title,
        day,
        summary,
        body_html: markdown::render(body),
    }
}

fn first_paragraph(md: &str) -> String {
    let mut buf = String::new();
    for line in md.lines() {
        let t = line.trim();
        if t.starts_with('#') || t.starts_with("```") || t == "---" {
            if buf.is_empty() {
                continue;
            }
            break;
        }
        if t.is_empty() {
            if !buf.is_empty() {
                break;
            }
            continue;
        }
        if !buf.is_empty() {
            buf.push(' ');
        }
        buf.push_str(t.trim_start_matches(['-', '*', '>']).trim());
    }
    buf
}

pub fn index_html(entries: &[Entry], root: &str) -> String {
    if entries.is_empty() {
        return format!(
            r#"<div class="empty-panel">
  <p class="empty-title">No entries yet.</p>
  <p class="empty-body">Day 0, Phase 0. The first entry appears when the agents start the daily log in <code>devlog/</code>. Until then this page stays empty rather than inventing a post.</p>
  <p><a class="text-link" href="{root}status/">Live status →</a></p>
</div>"#
        );
    }
    let mut cards = String::new();
    for entry in entries {
        let day = entry.day.map(|n| format!(" · DAY {n}")).unwrap_or_default();
        cards.push_str(&format!(
            r#"<a class="devlog-card" href="{root}devlog/{slug}/">
  <p class="card-meta">{date}{day}</p>
  <h2>{title}</h2>
  <p>{summary}</p>
</a>"#,
            slug = esc(&entry.slug),
            date = esc(&entry.date),
            title = esc(&entry.title),
            summary = esc(&entry.summary),
        ));
    }
    format!(r#"<div class="devlog-grid">{cards}</div>"#)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_heading_title() {
        let entry = parse("2026-10-09", "# Bootstrap\n\nAgents laid the workspace.\n");
        assert_eq!(entry.title, "Bootstrap");
        assert_eq!(entry.summary, "Agents laid the workspace.");
        assert!(entry.body_html.contains("workspace"));
    }
}
