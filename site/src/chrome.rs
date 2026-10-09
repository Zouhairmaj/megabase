//! Shared nav and footer, matching Kite components `Nav / Desktop`,
//! `Nav / Mobile closed` / `Nav / Mobile open`, `Footer / Desktop` and
//! `Footer / Mobile`.

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

pub fn header(paths: &Paths, logo: &str) -> String {
    let home = paths.home_href();
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
  <a class="brand" href="{home}">{logo}<span class="wordmark">MEGABASE</span></a>
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
