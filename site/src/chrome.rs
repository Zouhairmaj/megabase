//! Shared nav and footer, matching Kite components `Nav / Desktop`,
//! `Nav / Mobile closed` / `Nav / Mobile open`, `Footer / Desktop` and
//! `Footer / Mobile`. The release pill beside the wordmark is the
//! `Version` layer on those navs (frame `Home / Desktop (version)`).

use std::fs;
use std::io;
use std::path::Path;

use crate::html::esc;
use crate::GITHUB;

/// Prefixes used to emit relative (or, for 404, root-absolute) URLs.
#[derive(Clone, Debug)]
pub struct Paths {
    /// Site-root prefix: `""`, `"../"`, `"../../"`, or `"/"` on the 404 page.
    pub root: String,
    pub current: &'static str,
    pub is_404: bool,
    /// True on `/docs/<slug>/` (and similar) so `page(current)` goes up one directory.
    pub in_subdir: bool,
}

impl Paths {
    pub fn home(is_404: bool) -> Self {
        Self {
            root: if is_404 { "/".into() } else { String::new() },
            current: "home",
            is_404,
            in_subdir: false,
        }
    }

    pub fn nested(id: &'static str, is_404: bool) -> Self {
        Self {
            root: if is_404 { "/".into() } else { "../".into() },
            current: id,
            is_404,
            in_subdir: false,
        }
    }

    pub fn article(id: &'static str) -> Self {
        Self {
            root: "../../".into(),
            current: id,
            is_404: false,
            in_subdir: true,
        }
    }

    pub fn docs_article() -> Self {
        Self {
            root: "../../".into(),
            current: "docs",
            is_404: false,
            in_subdir: true,
        }
    }

    pub fn asset(&self) -> &str {
        &self.root
    }

    pub fn home_href(&self) -> String {
        if self.is_404 {
            "/".into()
        } else if self.current == "home" {
            "./".into()
        } else {
            self.root.clone()
        }
    }

    pub fn page(&self, dir: &str) -> String {
        if dir.is_empty() {
            return self.home_href();
        }
        if self.is_404 {
            return format!("/{dir}/");
        }
        if self.current == dir {
            if self.in_subdir {
                return "../".into();
            }
            return "./".into();
        }
        format!("{}{dir}/", self.root)
    }

    pub fn docs_slug(&self, slug: &str) -> String {
        if self.is_404 {
            return format!("/docs/{slug}/");
        }
        if self.current == "docs" {
            if self.in_subdir {
                return format!("../{slug}/");
            }
            return format!("./{slug}/");
        }
        format!("{}docs/{slug}/", self.root)
    }
}

const NOTICE: &str = "https://github.com/Zouhairmaj/megabase/blob/main/NOTICE";

const DESKTOP_NAV: &[(&str, &str, bool)] = &[
    ("manifesto", "MANIFESTO", false),
    ("how-it-works", "HOW IT WORKS", false),
    ("status", "STATUS", false),
    ("roadmap", "ROADMAP", false),
    ("components", "COMPONENTS", true),
    ("devlog", "DEVLOG", false),
    ("docs", "DOCS", false),
];

const MOBILE_NAV: &[(&str, &str)] = &[
    ("manifesto", "MANIFESTO"),
    ("how-it-works", "HOW IT WORKS"),
    ("status", "STATUS"),
    ("roadmap", "ROADMAP"),
    ("components", "COMPONENTS"),
    ("devlog", "DEVLOG"),
    ("docs", "DOCS"),
    ("faq", "FAQ"),
];

/// Workspace package version from `[workspace.package]` in `Cargo.toml`.
///
/// # Errors
///
/// Returns an error when the manifest cannot be read or has no
/// `[workspace.package] version`.
pub fn read_workspace_version(repo_root: &Path) -> io::Result<String> {
    let path = repo_root.join("Cargo.toml");
    let text = fs::read_to_string(&path).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("reading workspace manifest {}: {err}", path.display()),
        )
    })?;
    parse_workspace_version(&text).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("[workspace.package] version missing in {}", path.display()),
        )
    })
}

/// Version string from `[workspace.package] version` or a dotted
/// `package.version` key under `[workspace]`.
pub fn parse_workspace_version(toml: &str) -> Option<String> {
    let mut table: Vec<String> = Vec::new();
    for raw in toml.lines() {
        let line = strip_toml_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            table = toml_table_path(line);
            continue;
        }
        let Some((key, value)) = toml_key_value(line) else {
            continue;
        };
        let mut path = table.clone();
        path.extend(dotted_parts(key));
        if path != ["workspace", "package", "version"] {
            continue;
        }
        let quoted = unquote(value)?;
        if is_cargo_version(quoted) {
            return Some(quoted.to_string());
        }
        return None;
    }
    None
}

fn toml_table_path(header: &str) -> Vec<String> {
    let Some(inner) = header
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return Vec::new();
    };
    let inner = inner.trim();
    // Array-of-tables headers are not the workspace package table.
    if inner.starts_with('[') {
        return Vec::new();
    }
    dotted_parts(inner)
}

fn dotted_parts(value: &str) -> Vec<String> {
    value
        .split('.')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    for (index, ch) in line.char_indices() {
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '#' if !in_single && !in_double => return &line[..index],
            _ => {}
        }
    }
    line
}

fn toml_key_value(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() || key.contains(char::is_whitespace) {
        return None;
    }
    Some((key, value.trim()))
}

fn unquote(value: &str) -> Option<&str> {
    let bytes = value.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let quote = bytes[0];
    if (quote == b'"' || quote == b'\'') && bytes[bytes.len() - 1] == quote {
        Some(&value[1..value.len() - 1])
    } else {
        None
    }
}

/// `MAJOR.MINOR.PATCH` with an optional prerelease and build metadata.
fn is_cargo_version(version: &str) -> bool {
    let (main, build) = version.split_once('+').unwrap_or((version, ""));
    if version.contains('+')
        && (build.is_empty()
            || !build
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '.'))
    {
        return false;
    }
    let (core, pre) = main.split_once('-').unwrap_or((main, ""));
    if main.contains('-')
        && (pre.is_empty()
            || !pre
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-'))
    {
        return false;
    }
    let numbers: Vec<&str> = core.split('.').collect();
    numbers.len() == 3
        && numbers.iter().all(|part| {
            !part.is_empty()
                && part.chars().all(|ch| ch.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0'))
        })
}

pub fn header(paths: &Paths, logo: &str, version: &str) -> String {
    let home = paths.home_href();
    let version_text = esc(&format!("v{version}"));
    let release = esc(&format!("{GITHUB}/releases/tag/v{version}"));
    let mut desktop = String::new();
    for (id, label, hidden) in DESKTOP_NAV {
        let class = if *hidden { " nav-hide-desktop" } else { "" };
        desktop.push_str(&nav_link(
            label,
            &paths.page(id),
            paths.current == *id,
            class,
        ));
    }
    desktop.push_str(&format!(
        r#"<a class="nav-link nav-github" href="{GITHUB}" rel="noopener noreferrer">GITHUB ↗</a>"#
    ));

    let mut mobile = String::new();
    for (id, label) in MOBILE_NAV {
        mobile.push_str(&nav_link(label, &paths.page(id), paths.current == *id, ""));
    }
    mobile.push_str(&format!(
        r#"<a class="nav-link" href="{GITHUB}" rel="noopener noreferrer">GITHUB ↗</a>"#
    ));

    format!(
        r#"<header class="site-header">
  <div class="brand-lockup">
    <a class="brand" href="{home}">{logo}<span class="wordmark">MEGABASE</span></a>
    <a class="version-badge" href="{release}" rel="noopener noreferrer" title="{version_text}"><span class="version-badge-label">{version_text}</span></a>
  </div>
  <nav class="nav-desktop" aria-label="Primary">{desktop}</nav>
  <details class="nav-mobile">
    <summary class="nav-menu-btn"><span class="menu-open">MENU</span><span class="menu-close">CLOSE</span></summary>
    <nav class="nav-sheet" aria-label="Mobile">
      <p class="nav-sheet-brand">MEGABASE</p>
      {mobile}
    </nav>
  </details>
</header>"#
    )
}

fn nav_link(label: &str, href: &str, current: bool, extra_class: &str) -> String {
    if current {
        format!(
            r#"<a class="nav-link is-current{extra_class}" href="{href}" aria-current="page">{label}</a>"#
        )
    } else {
        format!(r#"<a class="nav-link{extra_class}" href="{href}">{label}</a>"#)
    }
}

pub fn footer(paths: &Paths, logo: &str) -> String {
    let col = |title: &str, items: &[(&str, String)]| {
        let mut links = format!(r#"<p class="footer-heading">{title}</p>"#);
        for (label, href) in items {
            let rel = if href.starts_with("http") {
                r#" rel="noopener noreferrer""#
            } else {
                ""
            };
            links.push_str(&format!(
                r#"<a href="{href}"{rel}>{label}</a>"#,
                href = esc(href),
                label = esc(label)
            ));
        }
        format!(r#"<div class="footer-col">{links}</div>"#)
    };

    let experiment = col(
        "EXPERIMENT",
        &[
            ("Manifesto", paths.page("manifesto")),
            ("How it works", paths.page("how-it-works")),
            ("Roadmap", paths.page("roadmap")),
            ("Components", paths.page("components")),
        ],
    );
    let live = col(
        "LIVE",
        &[
            ("Status", paths.page("status")),
            ("Devlog", paths.page("devlog")),
            ("Human log", paths.page("human-log")),
        ],
    );
    let resources = col(
        "RESOURCES",
        &[
            ("Docs", paths.page("docs")),
            ("FAQ", paths.page("faq")),
            ("GitHub ↗", GITHUB.into()),
            ("NOTICE & licenses", NOTICE.into()),
        ],
    );

    let mobile_left = [
        ("Manifesto", paths.page("manifesto")),
        ("How it works", paths.page("how-it-works")),
        ("Roadmap", paths.page("roadmap")),
        ("Components", paths.page("components")),
        ("Status", paths.page("status")),
    ];
    let mobile_right = [
        ("Devlog", paths.page("devlog")),
        ("Human log", paths.page("human-log")),
        ("Docs", paths.page("docs")),
        ("FAQ", paths.page("faq")),
        ("GitHub ↗", GITHUB.into()),
        ("NOTICE & licenses", NOTICE.into()),
    ];
    let mut mleft = String::new();
    for (label, href) in &mobile_left {
        let rel = if href.starts_with("http") {
            r#" rel="noopener noreferrer""#
        } else {
            ""
        };
        mleft.push_str(&format!(r#"<a href="{href}"{rel}>{label}</a>"#));
    }
    let mut mright = String::new();
    for (label, href) in &mobile_right {
        let rel = if href.starts_with("http") {
            r#" rel="noopener noreferrer""#
        } else {
            ""
        };
        mright.push_str(&format!(r#"<a href="{href}"{rel}>{label}</a>"#));
    }

    let home = paths.home_href();
    format!(
        r#"<footer class="site-footer">
  <div class="footer-desktop">
    <div class="footer-top">
      <div class="footer-brand">
        <a class="brand brand-footer" href="{home}">{logo}<span class="wordmark">MEGABASE</span></a>
        <p>Supabase, rewritten in Rust. By agents. In public.</p>
      </div>
      <div class="footer-cols">{experiment}{live}{resources}</div>
    </div>
    <div class="footer-legal">
      <p>Independent experiment. Not affiliated with or endorsed by Supabase, Inc.</p>
      <p>Apache-2.0 · Upstream licenses in NOTICE</p>
    </div>
  </div>
  <div class="footer-mobile">
    <a class="brand brand-footer" href="{home}">{logo}<span class="wordmark">MEGABASE</span></a>
    <div class="footer-mobile-cols">
      <div class="footer-col">{mleft}</div>
      <div class="footer-col">{mright}</div>
    </div>
    <div class="footer-legal">
      <p>Independent experiment. Not affiliated with or endorsed by Supabase, Inc.</p>
      <p>Apache-2.0 · Upstream licenses in NOTICE</p>
    </div>
  </div>
</footer>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_workspace_package_version_only() {
        let toml = r#"
[package]
name = "megabase-site"
version = "0.1.0"

[workspace.package]
edition = "2021"
# version = "9.9.9"
version = "0.1.6" # release

[workspace.dependencies]
tokio = { version = "1.40" }
"#;
        assert_eq!(parse_workspace_version(toml).as_deref(), Some("0.1.6"));
    }

    #[test]
    fn parses_dotted_workspace_package_version() {
        let toml = "[workspace]\nmembers = []\npackage.version = \"0.1.6\"\n";
        assert_eq!(parse_workspace_version(toml).as_deref(), Some("0.1.6"));
        let root = "workspace.package.version = '2.0.1'\n";
        assert_eq!(parse_workspace_version(root).as_deref(), Some("2.0.1"));
    }

    #[test]
    fn accepts_prerelease_and_single_quotes() {
        let toml = "[workspace.package]\nversion = '1.2.3-rc.1+build.7'\n";
        assert_eq!(
            parse_workspace_version(toml).as_deref(),
            Some("1.2.3-rc.1+build.7")
        );
    }

    #[test]
    fn rejects_a_missing_or_invalid_version() {
        assert!(parse_workspace_version("[package]\nversion = \"0.1.0\"\n").is_none());
        assert!(parse_workspace_version("[workspace.package]\nversion = \"01.2.3\"\n").is_none());
        assert!(
            parse_workspace_version("[workspace.package]\nversion.workspace = true\n").is_none()
        );
        assert!(parse_workspace_version("[workspace.package]\nversion = 0.1.6\n").is_none());
    }

    #[test]
    fn reads_the_workspace_manifest() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repo root");
        let version = read_workspace_version(root).expect("workspace version");
        let text = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        let section = text
            .split_once("[workspace.package]")
            .expect("workspace.package")
            .1
            .split('[')
            .next()
            .expect("section body");
        assert!(
            section.contains(&format!("version = \"{version}\"")),
            "parsed {version} is not the [workspace.package] version"
        );
    }

    #[test]
    fn header_places_one_release_badge_beside_the_wordmark() {
        let html = header(&Paths::home(false), "<svg></svg>", "1.2.3");
        let badge = html
            .find(r#"<a class="version-badge" href="https://github.com/Zouhairmaj/megabase/releases/tag/v1.2.3" rel="noopener noreferrer" title="v1.2.3"><span class="version-badge-label">v1.2.3</span></a>"#)
            .expect("badge");
        let brand_open = html.find(r#"<a class="brand""#).expect("brand");
        let brand_close = html[brand_open..].find("</a>").expect("brand close") + brand_open;
        let nav = html.find(r#"class="nav-desktop""#).expect("nav");
        assert!(brand_close < badge && badge < nav);
        assert_eq!(html.matches("class=\"version-badge\"").count(), 1);
        let footer = footer(&Paths::home(false), "<svg></svg>");
        assert!(!footer.contains("version-badge"));
    }
}
