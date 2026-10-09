//! Small CommonMark-ish renderer for Human log, Roadmap, Devlog and Docs notes.
//! Headings, paragraphs, lists, tables, fenced code, hr, callouts, and inline `**` / `` ` ``.

use std::collections::BTreeMap;

use crate::html::esc;

#[derive(Clone, Copy)]
pub struct Options {
    pub copy_buttons: bool,
    pub highlight: bool,
    pub skip_h1: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            copy_buttons: false,
            highlight: false,
            skip_h1: false,
        }
    }
}

pub fn render(md: &str) -> String {
    render_with(md, Options::default())
}

pub fn headings_h2(md: &str) -> Vec<(String, String)> {
    let mut fence_len = 0usize;
    let mut headings = Vec::new();
    for line in md.lines() {
        if fence_len == 0 {
            if let Some(n) = fence_open_len(line) {
                fence_len = n;
                continue;
            }
        } else {
            if is_fence_close(line, fence_len) {
                fence_len = 0;
            }
            continue;
        }
        let t = line.trim();
        if let Some(title) = t.strip_prefix("## ") {
            let title = title.trim();
            if !title.is_empty() {
                headings.push((slugify(title), title.to_string()));
            }
        }
    }
    headings
}

pub fn split_front_matter(raw: &str) -> (BTreeMap<String, String>, &str) {
    let raw = raw.trim_start_matches('\u{feff}');
    let Some(rest) = raw.strip_prefix("---") else {
        return (BTreeMap::new(), raw);
    };
    let rest = rest.trim_start_matches('\r').trim_start_matches('\n');
    let Some(end) = rest.find("\n---") else {
        return (BTreeMap::new(), raw);
    };
    let yaml = &rest[..end];
    let mut body = &rest[end + 4..];
    if let Some(stripped) = body.strip_prefix('\r') {
        body = stripped;
    }
    if let Some(stripped) = body.strip_prefix('\n') {
        body = stripped;
    }
    let mut map = BTreeMap::new();
    for line in yaml.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            map.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string());
        }
    }
    (map, body)
}

pub fn render_with(md: &str, opts: Options) -> String {
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
        if let Some(open_len) = fence_open_len(lines[i]) {
            let lang = stripped.trim_start_matches('`').trim().to_string();
            i += 1;
            let mut code = String::new();
            while i < lines.len() && !is_fence_close(lines[i], open_len) {
                if !code.is_empty() {
                    code.push('\n');
                }
                code.push_str(lines[i]);
                i += 1;
            }
            if i < lines.len() {
                i += 1;
            }
            blocks.push(code_block(&lang, &code, opts));
            continue;
        }
        if let Some((kind, title)) = parse_callout(stripped) {
            i += 1;
            let mut body = Vec::new();
            while i < lines.len() {
                let nxt = lines[i].trim_end();
                if nxt.starts_with("> ") || nxt == ">" {
                    body.push(nxt.trim_start_matches('>').trim().to_string());
                    i += 1;
                } else {
                    break;
                }
            }
            blocks.push(callout_html(kind, title.as_deref(), &body));
            continue;
        }
        if let Some(heading) = parse_heading(stripped) {
            let (level, title) = heading;
            if opts.skip_h1 && level == 1 {
                i += 1;
                continue;
            }
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
                || nxt.starts_with("> [!")
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
                let consumed = start + mid + end + 1;
                if !safe_href(href) {
                    out.push_str(&esc(&rest[..consumed]));
                    rest = &rest[consumed..];
                    continue;
                }
                out.push_str(&esc(&rest[..start]));
                let rel =
                    if href.trim().starts_with("http://") || href.trim().starts_with("https://") {
                        r#" rel="noopener noreferrer""#
                    } else {
                        ""
                    };
                out.push_str(&format!(
                    r#"<a href="{}"{rel}>{}</a>"#,
                    esc(href),
                    esc(label)
                ));
                rest = &rest[consumed..];
                continue;
            }
        }
        out.push_str(&esc(&rest[..start + 1]));
        rest = &rest[start + 1..];
    }
    out.push_str(&esc(rest));
    out
}

fn parse_callout(line: &str) -> Option<(&'static str, Option<String>)> {
    let t = line.trim();
    let rest = t.strip_prefix("> [!")?;
    let close = rest.find(']')?;
    let kind = rest[..close].trim();
    let after = rest[close + 1..].trim();
    let title = if after.is_empty() {
        None
    } else {
        Some(after.to_string())
    };
    match kind {
        "NOTE" | "note" => Some(("note", title)),
        "PLANNED" | "planned" => Some(("planned", title)),
        _ => Some(("note", title)),
    }
}

fn callout_html(kind: &str, title: Option<&str>, body: &[String]) -> String {
    let class = if kind == "planned" {
        "callout callout-planned"
    } else {
        "callout callout-note"
    };
    let fallback = if kind == "planned" {
        "Planned — not available yet"
    } else {
        "Note"
    };
    let head = title.unwrap_or(fallback);
    let paras: String = body
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| format!("<p>{}</p>", inline(p)))
        .collect();
    format!(
        r#"<aside class="{class}"><strong>{}</strong>{paras}</aside>"#,
        esc(head)
    )
}

fn code_block(lang: &str, code: &str, opts: Options) -> String {
    let label = if lang.is_empty() {
        "CODE".to_string()
    } else {
        lang.to_ascii_uppercase()
    };
    let copy = if opts.copy_buttons {
        r#"<button type="button" class="copy-btn" data-copy>⧉ COPY</button>"#
    } else {
        ""
    };
    let inner = if opts.highlight {
        highlight(code)
    } else {
        esc(code)
    };
    format!(
        r#"<div class="code-block"><div class="code-head"><span>{}</span>{copy}</div><pre><code>{}</code></pre></div>"#,
        esc(&label),
        inner
    )
}

fn highlight(code: &str) -> String {
    let chars: Vec<char> = code.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '/' && chars.get(i + 1) == Some(&'/') {
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            out.push_str(&span_class("tok-cmt", &collect(&chars[start..i])));
            continue;
        }
        if chars[i] == '#' && (i == 0 || chars[i - 1] == '\n' || chars[i - 1].is_whitespace()) {
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            out.push_str(&span_class("tok-cmt", &collect(&chars[start..i])));
            continue;
        }
        if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != quote && chars[i] != '\n' {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            if i < chars.len() && chars[i] == quote {
                i += 1;
            }
            out.push_str(&span_class("tok-str", &collect(&chars[start..i])));
            continue;
        }
        if chars[i].is_ascii_alphabetic() || chars[i] == '_' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '-')
            {
                i += 1;
            }
            let word = collect(&chars[start..i]);
            if is_keyword(&word) {
                out.push_str(&span_class("tok-kw", &word));
            } else {
                out.push_str(&esc(&word));
            }
            continue;
        }
        out.push_str(&esc(&chars[i].to_string()));
        i += 1;
    }
    out
}

fn collect(chars: &[char]) -> String {
    chars.iter().collect()
}

fn span_class(class: &str, text: &str) -> String {
    format!(r#"<span class="{class}">{}</span>"#, esc(text))
}

fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "import"
            | "from"
            | "const"
            | "let"
            | "await"
            | "export"
            | "async"
            | "function"
            | "return"
            | "if"
            | "else"
            | "for"
            | "while"
            | "match"
            | "fn"
            | "pub"
            | "struct"
            | "impl"
            | "use"
            | "mod"
            | "true"
            | "false"
            | "null"
            | "undefined"
            | "git"
            | "cd"
            | "cargo"
            | "curl"
            | "python"
            | "docker"
            | "compose"
            | "build"
            | "run"
            | "clone"
    )
}

fn fence_open_len(line: &str) -> Option<usize> {
    let stripped = line.trim_end();
    if !stripped.starts_with("```") {
        return None;
    }
    Some(stripped.chars().take_while(|c| *c == '`').count())
}

fn is_fence_close(line: &str, open_len: usize) -> bool {
    let stripped = line.trim_start();
    if !stripped.starts_with("```") {
        return false;
    }
    let n = stripped.chars().take_while(|c| *c == '`').count();
    n >= open_len && stripped[n..].chars().all(char::is_whitespace)
}

fn safe_href(href: &str) -> bool {
    let href = href.trim();
    if href.is_empty() || href.starts_with("//") {
        return false;
    }
    if href.starts_with('#') || href.starts_with("./") || href.starts_with("../") {
        return true;
    }
    if href.starts_with('/') && !href.starts_with("//") {
        return true;
    }
    match href.split_once(':') {
        None => true,
        Some((scheme, rest)) => {
            rest.starts_with("//")
                && (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        }
    }
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

    #[test]
    fn callouts_and_front_matter() {
        let (fm, body) = split_front_matter(
            "---\ntitle: Quickstart\nsection: get-started\n---\n\n# Quickstart\n\n> [!NOTE]\n> Day 0.\n\n> [!PLANNED]\n> Docker image.\n",
        );
        assert_eq!(fm.get("title").map(String::as_str), Some("Quickstart"));
        let html = render_with(
            body,
            Options {
                copy_buttons: true,
                highlight: true,
                skip_h1: true,
            },
        );
        assert!(!html.contains("<h1"));
        assert!(html.contains("callout-note"));
        assert!(html.contains("callout-planned"));
        assert_eq!(headings_h2(body).len(), 0);
    }

    #[test]
    fn headings_h2_skips_fenced_regions() {
        let md = "Intro\n\n```md\n## not a heading\n```\n\n## Real heading\n\n    ```\n## still a heading\n";
        let headings = headings_h2(md);
        assert_eq!(
            headings,
            vec![
                ("real-heading".into(), "Real heading".into()),
                ("still-a-heading".into(), "still a heading".into()),
            ]
        );
        let html = render(md);
        assert!(html.contains("id=\"real-heading\""));
        assert!(html.contains("id=\"still-a-heading\""));
        assert!(!html.contains("id=\"not-a-heading\""));
    }

    #[test]
    fn fence_close_requires_matching_length_and_no_info_string() {
        let md = "````md\n## not a heading\n```\nstill code\n````\n\n## Real\n";
        assert_eq!(headings_h2(md), vec![("real".into(), "Real".into())]);
        let html = render(md);
        assert!(html.contains("## not a heading"));
        assert!(html.contains("still code"));
        assert!(html.contains("id=\"real\""));
        assert!(!html.contains("id=\"not-a-heading\""));

        let inner = render("```\n```text\nstill code\n```\n");
        assert!(inner.contains("```text"));
        assert!(inner.contains("still code"));
    }

    #[test]
    fn unsafe_markdown_hrefs_render_as_text() {
        let html = render(
            "[ok](https://megabase.sh/) and [run](javascript:alert(1)) and [local](/docs/).",
        );
        assert!(html.contains("href=\"https://megabase.sh/\""));
        assert!(html.contains("href=\"/docs/\""));
        assert!(!html.contains("href=\"javascript:"));
        assert!(html.contains("[run](javascript:alert(1))"));
    }
}
