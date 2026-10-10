use crate::metrics::Metrics;

pub const TOC: &[&str] = &[
    "What this is",
    "Why",
    "Rules of the experiment",
    "Scope",
    "Definition of success",
    "How progress is measured",
    "Licensing and credits",
];

const PULL_QUOTE: &str = "The experiment is the real product. The binary is its proof.";
const GITHUB: &str = super::GITHUB;

pub fn render(md: &str, metrics: &Metrics) -> String {
    let sections = split_sections(md);
    let mut parts = vec![
        r#"<p class="doc-eyebrow">MANIFESTO · WRITTEN BY HUMANS · DAY 0</p>"#.into(),
        concat!(
            r#"<h1 class="manifesto-title">Supabase,<br />rewritten in Rust.<br />"#,
            r#"<span class="headline-accent">By agents. In public.</span></h1>"#
        )
        .into(),
        concat!(
            r#"<p class="callout">Independent experiment. Not affiliated with, endorsed by, "#,
            r#"or sponsored by Supabase, Inc. “Supabase” is used only to describe compatibility.</p>"#
        )
        .into(),
        r#"<hr class="article-rule" />"#.into(),
    ];

    let mut open = false;
    for section in sections {
        let title = section.title;
        let level = section.level;
        if level == 0 || matches!(title.to_ascii_lowercase().as_str(), "megabase" | "status") {
            continue;
        }
        if level == 1 {
            continue;
        }
        let body = render_blocks(&section.lines, metrics);
        if level == 3 {
            parts.push(format!("<h3>{}</h3>", esc(&title)));
            parts.push(body);
            continue;
        }
        if open {
            parts.push("</section>".into());
        }
        let slug = slugify(&title);
        if let Some(num) = toc_num(&title) {
            parts.push(format!(
                r#"<section class="doc-section" aria-labelledby="{slug}"><h2 id="{slug}"><span class="sec-num">{num} </span>{}</h2>"#,
                esc(&title)
            ));
            parts.push(body);
            if num == "01" {
                parts.push(cta_row());
            }
            open = true;
        } else {
            parts.push(format!(
                r#"<section class="doc-section extra" aria-labelledby="{slug}"><h2 id="{slug}">{}</h2>"#,
                esc(&title)
            ));
            parts.push(body);
            open = true;
        }
    }
    if open {
        parts.push("</section>".into());
    }
    parts.join("\n")
}

pub fn toc(kind: &str) -> String {
    let mut links = String::new();
    for (i, title) in TOC.iter().enumerate() {
        let num = format!("{:02}", i + 1);
        let slug = slugify(title);
        let current = if i == 0 {
            r#" class="is-active" aria-current="location""#
        } else {
            ""
        };
        links.push_str(&format!(
            r##"<a href="#{slug}"{current}><span class="toc-num">{num}</span> {title}</a>"##,
            slug = slug,
            current = current,
            num = num,
            title = esc(title)
        ));
    }
    if kind == "desktop" {
        format!(
            r#"<nav class="toc toc-desktop" data-toc aria-label="Contents"><p class="toc-label">CONTENTS</p><div class="toc-list">{links}</div></nav>"#
        )
    } else {
        format!(
            r#"<details class="toc-mobile"><summary>Contents</summary><nav class="toc" data-toc aria-label="Contents"><div class="toc-list">{links}</div></nav></details>"#
        )
    }
}

fn cta_row() -> String {
    format!(
        r##"<div class="actions manifesto-cta"><a class="btn btn-primary" href="{GITHUB}" rel="noopener noreferrer">FOLLOW ON GITHUB ↗</a><a class="btn btn-ghost" href="../status/">SEE LIVE STATUS</a></div>"##
    )
}

struct Section {
    level: usize,
    title: String,
    lines: Vec<String>,
}

fn split_sections(md: &str) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut current: Option<Section> = None;
    for line in md.lines() {
        if let Some(heading) = parse_heading(line) {
            if let Some(prev) = current.take() {
                sections.push(prev);
            }
            current = Some(Section {
                level: heading.0,
                title: heading.1,
                lines: Vec::new(),
            });
        } else if let Some(section) = current.as_mut() {
            section.lines.push(line.to_string());
        } else {
            current = Some(Section {
                level: 0,
                title: String::new(),
                lines: vec![line.to_string()],
            });
        }
    }
    if let Some(section) = current {
        sections.push(section);
    }
    sections
}

fn parse_heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_end();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if (1..=3).contains(&hashes) && trimmed.as_bytes().get(hashes) == Some(&b' ') {
        Some((hashes, trimmed[hashes + 1..].trim().to_string()))
    } else {
        None
    }
}

fn render_blocks(lines: &[String], metrics: &Metrics) -> String {
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let stripped = lines[i].trim();
        if stripped.is_empty() {
            i += 1;
            continue;
        }
        if stripped == "---" {
            blocks.push(r#"<hr class="article-rule" />"#.into());
            i += 1;
            continue;
        }
        if stripped.starts_with('|') {
            let mut table = Vec::new();
            while i < lines.len() && lines[i].trim().starts_with('|') {
                table.push(lines[i].trim().to_string());
                i += 1;
            }
            let rows = parse_table(&table);
            if is_scope_table(&rows) {
                blocks.push(render_scope_table(&rows, metrics));
            } else {
                blocks.push(html_table(&rows));
            }
            continue;
        }
        if numbered(stripped) {
            let mut items = Vec::new();
            while i < lines.len() && numbered(lines[i].trim()) {
                items.push(strip_number(lines[i].trim()));
                i += 1;
            }
            let lis: String = items
                .iter()
                .map(|item| format!("<li>{}</li>", inline(item)))
                .collect();
            if !lis.is_empty() {
                blocks.push(format!("<ol>{lis}</ol>"));
            }
            continue;
        }
        if bulleted(stripped) {
            let mut items = Vec::new();
            while i < lines.len() && bulleted(lines[i].trim()) {
                items.push(strip_bullet(lines[i].trim()));
                i += 1;
            }
            let lis: String = items
                .iter()
                .map(|item| format!("<li>{}</li>", inline(item)))
                .collect();
            if !lis.is_empty() {
                blocks.push(format!("<ul>{lis}</ul>"));
            }
            continue;
        }
        let mut para = vec![stripped.to_string()];
        i += 1;
        while i < lines.len() {
            let nxt = lines[i].trim();
            if nxt.is_empty()
                || nxt.starts_with('|')
                || nxt.starts_with('#')
                || nxt == "---"
                || numbered(nxt)
                || bulleted(nxt)
            {
                break;
            }
            para.push(nxt.to_string());
            i += 1;
        }
        let text = para.join(" ");
        if text == PULL_QUOTE {
            blocks.push(format!(
                r#"<blockquote class="pullquote">{}</blockquote>"#,
                inline(&text)
            ));
        } else {
            blocks.push(format!("<p>{}</p>", inline(&text)));
        }
    }
    blocks.join("\n")
}

fn numbered(s: &str) -> bool {
    let digits = s.bytes().take_while(|b| b.is_ascii_digit()).count();
    digits > 0 && s[digits..].starts_with(". ")
}

fn bulleted(s: &str) -> bool {
    s.starts_with("- ") || s.starts_with("* ")
}

fn strip_number(s: &str) -> String {
    let rest = s.trim_start_matches(|c: char| c.is_ascii_digit());
    rest.trim_start_matches('.').trim().to_string()
}

fn strip_bullet(s: &str) -> String {
    s.trim_start_matches(['-', '*']).trim().to_string()
}

fn parse_table(rows: &[String]) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for row in rows {
        let compact: String = row.chars().filter(|c| !c.is_whitespace()).collect();
        if compact
            .trim_start_matches('|')
            .chars()
            .all(|c| c == '-' || c == ':' || c == '|')
        {
            continue;
        }
        let cells: Vec<String> = row
            .trim()
            .trim_matches('|')
            .split('|')
            .map(|c| c.trim().to_string())
            .collect();
        if !cells.is_empty() {
            out.push(cells);
        }
    }
    out
}

fn is_scope_table(rows: &[Vec<String>]) -> bool {
    rows.first()
        .and_then(|r| r.first())
        .map(|h| h.eq_ignore_ascii_case("component"))
        .unwrap_or(false)
}

fn render_scope_table(rows: &[Vec<String>], metrics: &Metrics) -> String {
    let mut parts = String::from(
        r#"<div class="table-scroll" tabindex="0" role="region" aria-label="Scope"><div class="scope-table" role="table" aria-label="Scope"><div class="scope-row scope-head" role="row"><span role="columnheader">COMPONENT</span><span role="columnheader">UPSTREAM</span><span role="columnheader">STATUS</span></div>"#,
    );
    for cells in rows.iter().skip(1) {
        let name = cells.first().map(String::as_str).unwrap_or("");
        let language_fallback = cells.get(1).map(String::as_str).unwrap_or("");
        let (component, language, license, level) = scope_meta(name, language_fallback);
        let coverage = component
            .map(|key| metrics.component_coverage_label(key))
            .unwrap_or_else(|| "—".into());
        let status = if level > 0 {
            format!("{} · Level {level}", coverage)
        } else {
            coverage
        };
        let upstream = format!("{language} · {license}");
        parts.push_str(&format!(
            r#"<div class="scope-row" role="row"><span class="scope-name" role="cell">{}</span><span role="cell">{}</span><span role="cell">{}</span></div>"#,
            esc(name),
            esc(&upstream),
            esc(&status)
        ));
    }
    parts.push_str(
        r#"</div></div><p class="scope-note">Upstream licenses are preserved and credited in <code>NOTICE</code> and <code>LICENSES/</code>. Status is regenerated on every commit.</p>"#,
    );
    parts
}

fn scope_meta<'a>(
    name: &str,
    language: &'a str,
) -> (Option<&'static str>, &'a str, &'static str, u8) {
    match name {
        "REST API (PostgREST)" => (Some("rest"), "Haskell", "MIT", 1),
        "Auth (GoTrue)" => (Some("auth"), "Go", "MIT", 1),
        "Realtime" => (Some("realtime"), "Elixir", "Apache-2.0", 3),
        "Storage API" => (Some("storage"), "TypeScript", "Apache-2.0", 2),
        "Edge Functions runtime" => (Some("functions"), "Rust + Deno", "Apache-2.0", 4),
        "Connection pooler (Supavisor)" => (Some("pooler"), "Elixir", "Apache-2.0", 4),
        "Postgres Meta" => (Some("meta"), "TypeScript", "Apache-2.0", 4),
        "API gateway (replaces Kong)" => (None, "Lua / Nginx", "Apache-2.0", 1),
        "Studio (dashboard)" => (Some("studio"), "TypeScript / Next.js", "Apache-2.0", 4),
        _ => (None, language, "see NOTICE", 0),
    }
}

fn html_table(rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let head: String = rows[0]
        .iter()
        .map(|c| format!("<th scope='col'>{}</th>", inline(c)))
        .collect();
    let body: String = rows[1..]
        .iter()
        .map(|row| {
            let tds: String = row
                .iter()
                .map(|c| format!("<td>{}</td>", inline(c)))
                .collect();
            format!("<tr>{tds}</tr>")
        })
        .collect();
    format!(
        r#"<div class="table-scroll" tabindex="0"><table><thead><tr>{head}</tr></thead><tbody>{body}</tbody></table></div>"#
    )
}

fn inline(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(idx) = rest.find("**").or_else(|| rest.find('`')) {
        let which = if rest[idx..].starts_with("**") {
            "**"
        } else {
            "`"
        };
        out.push_str(&esc(&rest[..idx]));
        rest = &rest[idx + which.len()..];
        if let Some(end) = rest.find(which) {
            let inner = esc(&rest[..end]);
            if which == "**" {
                out.push_str(&format!("<strong>{inner}</strong>"));
            } else {
                out.push_str(&format!("<code>{inner}</code>"));
            }
            rest = &rest[end + which.len()..];
        } else {
            out.push_str(&esc(which));
        }
    }
    out.push_str(&esc(rest));
    out
}

fn toc_num(title: &str) -> Option<&'static str> {
    TOC.iter()
        .position(|t| t.eq_ignore_ascii_case(title))
        .map(|i| ["01", "02", "03", "04", "05", "06", "07"][i])
}

pub fn slugify(title: &str) -> String {
    let mut slug = String::new();
    for ch in title.to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::Metrics;

    #[test]
    fn manifesto_prose_matches_the_source() {
        let sentence = "the loop and the logs, in real time";
        let md = format!(
            "\
## Rules of the experiment

7. **Everything is public.** The code, the agent prompts, {sentence}.

## How progress is measured

- **Coverage:** the share of units.
- **Treemaps:** one square per unit.
"
        );
        let html = render(&md, &Metrics::placeholder());
        assert!(html.contains(sentence));
        assert!(html.contains("<strong>Everything is public.</strong>"));
        assert!(html.contains("Coverage"));
        assert!(html.contains("Treemaps"));
        assert!(!html.to_ascii_lowercase().contains("token spend"));
    }

    #[test]
    fn renderer_does_not_rewrite_source_sentences() {
        let clause = "the loop, the logs, the token spend and the cost, in real time";
        let md = format!(
            "\
## Rules of the experiment

7. **Everything is public.** The code, the agent prompts, {clause}.

## How progress is measured

- **Cost:** tokens and money spent, published continuously.
"
        );
        let html = render(&md, &Metrics::placeholder());
        assert!(html.contains(clause));
        assert!(html.contains("tokens and money spent, published continuously"));
    }
}
