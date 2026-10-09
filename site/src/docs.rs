//! Docs pages rendered from `docs/*.md` YAML front matter.

use std::fs;
use std::io;
use std::path::Path;

use crate::chrome::Paths;
use crate::html::esc;
use crate::markdown::{self, Options};
use crate::metrics::{self, CatalogRow, Metrics, CATALOG};
use crate::GITHUB;

pub const PLANNED: &[&str] = &["Docker image", "API reference", "Migrating from Supabase"];

const SECTION_ORDER: &[&str] = &["get-started", "use", "project"];

#[derive(Clone, Debug)]
pub struct Doc {
    pub slug: String,
    pub title: String,
    pub description: String,
    pub section: String,
    pub order: u32,
    pub card: String,
    pub tag: Option<String>,
    pub source: String,
    pub html: String,
    pub headings: Vec<(String, String)>,
}

impl Doc {
    pub fn href(&self, paths: &Paths) -> String {
        paths.docs_slug(&self.slug)
    }
}

pub fn load(repo_root: &Path) -> io::Result<Vec<Doc>> {
    let dir = repo_root.join("docs");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut docs = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.eq_ignore_ascii_case("ROADMAP.md") {
            continue;
        }
        let slug = name.trim_end_matches(".md").to_string();
        let raw = fs::read_to_string(&path)?;
        let (fm, body) = markdown::split_front_matter(&raw);
        if fm.is_empty() {
            continue;
        }
        let title = fm.get("title").cloned().unwrap_or_else(|| slug.clone());
        let description = fm.get("description").cloned().unwrap_or_default();
        let section = fm
            .get("section")
            .cloned()
            .unwrap_or_else(|| "get-started".into());
        let order = fm.get("order").and_then(|s| s.parse().ok()).unwrap_or(99);
        let card = fm
            .get("card")
            .cloned()
            .unwrap_or_else(|| description.clone());
        let tag = fm.get("tag").cloned().filter(|s| !s.is_empty());
        let html = markdown::render_with(
            body,
            Options {
                copy_buttons: true,
                highlight: true,
                skip_h1: true,
            },
        );
        docs.push(Doc {
            slug,
            title,
            description,
            section,
            order,
            card,
            tag,
            source: format!("docs/{name}"),
            html,
            headings: markdown::headings_h2(body),
        });
    }
    docs.sort_by(|a, b| {
        section_rank(&a.section)
            .cmp(&section_rank(&b.section))
            .then(a.order.cmp(&b.order))
            .then(a.slug.cmp(&b.slug))
    });
    Ok(docs)
}

fn section_rank(section: &str) -> usize {
    SECTION_ORDER
        .iter()
        .position(|s| *s == section)
        .unwrap_or(SECTION_ORDER.len())
}

pub fn section_label(section: &str) -> &'static str {
    match section {
        "get-started" => "GET STARTED",
        "use" => "USE",
        "project" => "PROJECT",
        "planned" => "PLANNED",
        _ => "DOCS",
    }
}

pub fn index(paths: &Paths, docs: &[Doc], metrics: &Metrics, sha: &str, date: &str) -> String {
    let cards = index_cards(paths, docs, metrics);
    let table = status_table(metrics);
    let n = docs.len();
    let passing = metrics.passing_total_label();
    let units_line = match metrics.total {
        Some(_) => format!(
            "The binary builds and starts. Every API answers 501 MEGABASE_NOT_IMPLEMENTED and {} units pass the judge.",
            esc(&passing)
        ),
        None => "The binary is not in this repository yet (Phase 0). When it exists, every API answers 501 MEGABASE_NOT_IMPLEMENTED. Unit counts appear when coverage/summary.json is present.".to_string(),
    };
    format!(
        r#"<main id="main" class="docs-index">
  <header class="docs-hero">
    <div class="docs-hero-copy">
      <p class="kicker">DOCS · {stage}</p>
      <h1 class="display-sm">Megabase documentation</h1>
      <p class="lede">Build Megabase from source, point supabase-js at it, follow what each API does today and help the agents get there. Every page is rendered from the markdown in docs/ on each commit.</p>
    </div>
    <aside class="callout callout-note docs-hero-note"><strong>Read this first</strong><p>Nothing passes yet.</p><p>{units_line} Use these docs to follow along and contribute, not to run production.</p></aside>
  </header>
  <section class="docs-start">
    <div class="docs-start-head">
      <h2>Start here</h2>
      <p class="muted">{n} guides · rendered from docs/*.md</p>
    </div>
    <div class="docs-cards">{cards}</div>
  </section>
  <section class="docs-index-split">
    <div>
      <div class="docs-start-head">
        <h2>API status per service</h2>
        <a class="text-link" href="{status}">Full status →</a>
      </div>
      {table}
      <p class="note">Generated at build from coverage/summary.json · commit {sha} · {date}</p>
    </div>
    <aside class="docs-how">
      <h2>How these docs work</h2>
      <ul>
        <li>Written in markdown under docs/ in the repository, next to the code.</li>
        <li>Rendered to static HTML by the Rust site generator on every commit.</li>
        <li>Status numbers come from coverage/summary.json. Nothing is typed by hand.</li>
      </ul>
      <p><a class="text-link" href="{edit}" rel="noopener noreferrer">Edit a page on GitHub ↗</a></p>
      <div class="docs-planned-box">
        <p class="toc-label">Planned · not written yet</p>
        <p>Docker image</p>
        <p>API reference per component</p>
        <p>Migrating from Supabase</p>
      </div>
    </aside>
  </section>
</main>"#,
        stage = esc(&metrics.stage_short.to_ascii_uppercase()),
        status = paths.page("status"),
        edit = format_args!("{GITHUB}/tree/main/docs"),
    )
}

fn index_cards(paths: &Paths, docs: &[Doc], metrics: &Metrics) -> String {
    let mut out = String::new();
    for (i, doc) in docs.iter().enumerate() {
        let n = format!("{:02}", i + 1);
        let tag = card_tag(doc, metrics);
        let tag_html = if tag.is_empty() {
            String::new()
        } else {
            format!(
                r#"<span class="docs-card-tag">{tag}</span>"#,
                tag = esc(&tag)
            )
        };
        out.push_str(&format!(
            r#"<a class="docs-card" href="{href}" aria-label="{title}">
  <div class="docs-card-top"><span class="docs-card-idx" aria-hidden="true">{n}</span>{tag_html}</div>
  <h2 aria-hidden="true">{title_vis}</h2>
  <p aria-hidden="true">{card}</p>
  <span class="docs-card-foot" aria-hidden="true"><span>{source}</span><span>Read →</span></span>
</a>"#,
            href = doc.href(paths),
            title = esc(&doc.title),
            title_vis = esc(&doc.title),
            card = esc(&doc.card),
            source = esc(&doc.source),
        ));
    }
    out
}

fn card_tag(doc: &Doc, metrics: &Metrics) -> String {
    match doc.tag.as_deref() {
        Some("501") => "EVERY CALL 501 TODAY".into(),
        Some("status") => format!("{} CONFORMANT", metrics.passing_total_label()),
        Some(other) => other.to_ascii_uppercase(),
        None => String::new(),
    }
}

pub fn article(
    paths: &Paths,
    docs: &[Doc],
    current: &Doc,
    metrics: &Metrics,
    sha: &str,
    date: &str,
) -> String {
    let side = sidebar(paths, docs, Some(&current.slug), true);
    let mobile_nav = sidebar(paths, docs, Some(&current.slug), false);
    let toc: String = current
        .headings
        .iter()
        .map(|(id, title)| format!("<a href=\"#{}\">{}</a>", id, esc(title)))
        .collect();
    let (prev, next) = neighbors(docs, &current.slug);
    let prev_html = match prev {
        Some(d) => format!(
            r#"<a href="{}"><span class="muted">Previous</span><strong>{}</strong></a>"#,
            d.href(paths),
            esc(&d.title)
        ),
        None => r#"<span></span>"#.into(),
    };
    let next_html = match next {
        Some(d) => format!(
            r#"<a class="docs-next-fwd" href="{}"><span class="muted">Next →</span><strong>{}</strong></a>"#,
            d.href(paths),
            esc(&d.title)
        ),
        None => r#"<span></span>"#.into(),
    };
    let extra = if current.slug == "status" {
        status_table(metrics)
    } else {
        String::new()
    };
    let n_sections = current.headings.len();
    let toc_mobile: String = current
        .headings
        .iter()
        .map(|(id, title)| format!("<a href=\"#{}\">{}</a>", id, esc(title)))
        .collect();
    let crumb = format!("Docs / {}", esc(&current.title));
    let edit = format!("{GITHUB}/edit/main/{}", current.source);
    format!(
        r#"<main id="main" class="docs-article">
  <div class="docs-mobile-bars hide-desktop">
    <p class="docs-mobile-crumb">{crumb}</p>
    <details class="docs-mobile-panel">
      <summary>On this page · {n_sections} sections</summary>
      <nav aria-label="On this page">{toc_mobile}</nav>
    </details>
    <details class="docs-mobile-panel">
      <summary>Contents</summary>
      {mobile_nav}
    </details>
  </div>
  <nav class="docs-side hide-mobile" aria-label="Docs">{side}</nav>
  <article class="docs-body article-prose">
    <p class="docs-breadcrumb hide-mobile"><a href="{home}">Docs</a> / {section} / {title}</p>
    <h1>{title}</h1>
    <p class="lede">{description}</p>
    <p class="docs-meta"><span>{source}</span> · Built from commit {sha} · {date} · <a href="{edit}" rel="noopener noreferrer">Edit on GitHub ↗</a></p>
    {html}
    {extra}
    <nav class="docs-pager" aria-label="Adjacent guides">{prev_html}{next_html}</nav>
    <p class="note">Wrong or unfinished? Open an issue or edit {source} on GitHub.</p>
  </article>
  <nav class="docs-toc hide-mobile" aria-label="On this page">
    <p class="toc-label">ON THIS PAGE</p>
    {toc}
  </nav>
</main>"#,
        home = paths.page("docs"),
        section = esc(section_label(&current.section)),
        title = esc(&current.title),
        description = esc(&current.description),
        source = esc(&current.source),
        html = current.html,
    )
}

fn sidebar(paths: &Paths, docs: &[Doc], current: Option<&str>, with_home: bool) -> String {
    let mut out = String::new();
    if with_home {
        out.push_str(&format!(
            r#"<a class="docs-back" href="{}">← Docs home</a>"#,
            paths.page("docs")
        ));
    }
    for section in SECTION_ORDER {
        let group: Vec<&Doc> = docs.iter().filter(|d| d.section == *section).collect();
        if group.is_empty() {
            continue;
        }
        out.push_str(&format!(
            r#"<div class="docs-group"><p class="toc-label">{}</p>"#,
            section_label(section)
        ));
        for doc in group {
            let active = current == Some(doc.slug.as_str());
            let class = if active { " is-active" } else { "" };
            let aria = if active {
                r#" aria-current="page""#
            } else {
                ""
            };
            out.push_str(&format!(
                r#"<a class="docs-nav-link{class}" href="{href}"{aria}>{title}</a>"#,
                href = doc.href(paths),
                title = esc(&doc.title),
            ));
        }
        out.push_str("</div>");
    }
    out.push_str(r#"<div class="docs-group"><p class="toc-label">PLANNED</p>"#);
    for item in PLANNED {
        out.push_str(&format!(
            r#"<span class="docs-nav-planned">{}</span>"#,
            esc(item)
        ));
    }
    out.push_str("</div>");
    out
}

fn neighbors<'a>(docs: &'a [Doc], slug: &str) -> (Option<&'a Doc>, Option<&'a Doc>) {
    let idx = docs.iter().position(|d| d.slug == slug);
    match idx {
        Some(i) => (i.checked_sub(1).and_then(|j| docs.get(j)), docs.get(i + 1)),
        None => (None, None),
    }
}

pub fn status_table(metrics: &Metrics) -> String {
    let mut rows = String::from(
        r#"<div class="data-table docs-status-table" role="table" aria-label="API status per service">
<div class="data-row head" role="row"><span>Service</span><span>Endpoint</span><span>Level</span><span>Units</span><span>Conformant</span><span>Status</span></div>"#,
    );
    let mut units_total = 0usize;
    let mut conf_total = 0usize;
    let mut any = false;
    for row in CATALOG {
        let block = metrics.component(row.id);
        if let Some(b) = block {
            any = true;
            units_total += b.total();
            conf_total += b.conformant;
        }
        let units = match block {
            Some(b) if b.total() > 0 => metrics::comma(b.total()),
            _ => "—".into(),
        };
        let conf = match block {
            Some(b) if b.total() > 0 => metrics::comma(b.conformant),
            _ => "—".into(),
        };
        rows.push_str(&format!(
            r#"<div class="data-row" role="row"><span class="scope-name">{name}</span><span>{path}</span><span>{levels}</span><span>{units}</span><span>{conf}</span><span class="tag">{tag}</span></div>"#,
            name = esc(short_name(row)),
            path = esc(row.path),
            levels = esc(&row.levels.replace('L', "")),
            tag = metrics::status_tag(block),
        ));
    }
    let (tot_units, tot_conf, tot_pct) = if any {
        (
            metrics::comma(units_total),
            metrics::comma(conf_total),
            metrics.conformance_label(),
        )
    } else {
        ("—".into(), "—".into(), "—".into())
    };
    rows.push_str(&format!(
        r#"<div class="data-row docs-total" role="row"><span>Total</span><span></span><span></span><span>{tot_units}</span><span>{tot_conf}</span><span>{tot_pct}</span></div></div>"#
    ));
    rows
}

fn short_name(row: &CatalogRow) -> &'static str {
    match row.id {
        "rest" => "REST",
        "auth" => "Auth",
        "storage" => "Storage",
        "realtime" => "Realtime",
        "functions" => "Functions",
        "pooler" => "Pooler",
        "meta" => "Meta",
        "studio" => "Studio",
        _ => row.name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planned_items_are_not_links() {
        let html = sidebar(&Paths::nested("docs", false), &[], Some("quickstart"), true);
        assert!(html.contains("docs-nav-planned"));
        assert!(html.contains("Docker image"));
        assert!(!html.contains("href=\"#planned"));
        assert!(html.contains("← Docs home"));
    }
}
