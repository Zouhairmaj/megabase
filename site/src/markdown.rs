//! Small CommonMark-ish renderer for Human log, Roadmap, Devlog and Docs notes.
//! Headings, paragraphs, lists, tables, fenced code, hr, and inline `**` / `` ` ``.

use crate::html::esc;

pub fn render(md: &str) -> String {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let stripped = lines[i].trim_end();
        if stripped.is_empty() {
            i += 1;
            continue;
        }
        if stripped == "---" {
            blocks.push(r#"<hr class="article-rule" />"#.into());
            i += 1;
            continue;
        }
        if stripped.starts_with("```") {
            let lang = stripped.trim_start_matches('`').trim().to_string();
            i += 1;
            let mut code = String::new();
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                if !code.is_empty() {
                    code.push('\n');
                }
                code.push_str(lines[i]);
                i += 1;
            }
            if i < lines.len() {
                i += 1;
            }
            let label = if lang.is_empty() {
                "CODE".to_string()
            } else {
                lang.to_ascii_uppercase()
            };
            blocks.push(format!(
                r#"<div class="code-block"><div class="code-head"><span>{}</span></div><pre><code>{}</code></pre></div>"#,
                esc(&label),
                esc(&code)
            ));
            continue;
        }
        if let Some(heading) = parse_heading(stripped) {
            let (level, title) = heading;
            let slug = slugify(&title);
            blocks.push(format!(
                "<h{level} id=\"{slug}\">{}</h{level}>",
                inline(&title)
            ));
            i += 1;
            continue;
        }
        if stripped.starts_with('|') {
            let mut table = Vec::new();
            while i < lines.len() && lines[i].trim().starts_with('|') {
                table.push(lines[i].trim().to_string());
                i += 1;
            }
            blocks.push(html_table(&parse_table(&table)));
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
            blocks.push(format!("<ol>{lis}</ol>"));
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
            blocks.push(format!("<ul>{lis}</ul>"));
            continue;
        }
        let mut para = vec![stripped.trim().to_string()];
        i += 1;
        while i < lines.len() {
            let nxt = lines[i].trim();
            if nxt.is_empty()
                || nxt.starts_with('|')
                || nxt.starts_with('#')
                || nxt == "---"
                || nxt.starts_with("```")
                || numbered(nxt)
                || bulleted(nxt)
            {
                break;
            }
            para.push(nxt.to_string());
            i += 1;
        }
        blocks.push(format!("<p>{}</p>", inline(&para.join(" "))));
    }
    blocks.join("\n")
}

pub fn inline(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(idx) = rest.find("**").or_else(|| rest.find('`')) {
        let which = if rest[idx..].starts_with("**") {
            "**"
        } else {
            "`"
        };
        out.push_str(&linkify(&rest[..idx]));
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
    out.push_str(&linkify(rest));
    out
}

fn linkify(text: &str) -> String {
    // [label](url)
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        if let Some(mid) = rest[start..].find("](") {
            if let Some(end) = rest[start + mid..].find(')') {
                let label = &rest[start + 1..start + mid];
                let href_start = start + mid + 2;
                let href = &rest[href_start..start + mid + end];
                out.push_str(&esc(&rest[..start]));
                let rel = if href.starts_with("http") {
                    r#" rel="noopener noreferrer""#
                } else {
                    ""
                };
                out.push_str(&format!(
                    r#"<a href="{}"{rel}>{}</a>"#,
                    esc(href),
                    esc(label)
                ));
                rest = &rest[start + mid + end + 1..];
                continue;
            }
        }
        out.push_str(&esc(&rest[..start + 1]));
        rest = &rest[start + 1..];
    }
    out.push_str(&esc(rest));
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_code_and_emphasis() {
        let html = render("Hello **world** and `code`.\n\n```shell\ngit clone\n```\n");
        assert!(html.contains("<strong>world</strong>"));
        assert!(html.contains("<code>code</code>"));
        assert!(html.contains("git clone"));
    }
}
