//! Static generator for the Megabase placeholder site.
//!
//! Shared chrome (layout, nav, footer, logo) lives in `templates/`. Page
//! bodies live in `templates/pages/` or, for the manifesto, are rendered from
//! `MANIFESTO.md`. Add a future page by appending a `Page` in `PAGES` and
//! dropping in a template — Status, Roadmap, How it works, Components,
//! Devlog, Human log, FAQ and Docs are listed as forthcoming.
//!
//! Usage:
//!   cargo run --manifest-path site/Cargo.toml -- --repo-root . --out _site

mod manifesto;
mod metrics;
mod og;
mod treemap;

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use metrics::Metrics;

pub const GITHUB: &str = "https://github.com/Zouhairmaj/megabase";
const ORIGIN: &str = "https://megabase.sh";

#[derive(Clone, Copy)]
enum Kind {
    Home,
    Manifesto,
    Template(&'static str),
}

#[derive(Clone, Copy)]
struct Page {
    id: &'static str,
    /// Directory under the site root. Empty string is `/`.
    dir: &'static str,
    title: &'static str,
    description: &'static str,
    og_type: &'static str,
    /// Per-page Open Graph card under `static/`, 1200×630 PNG. Designs live in
    /// the Kite file `megabase-identity`, page "Website / OG images";
    /// regenerate with `cargo run --manifest-path site/Cargo.toml -- og`
    /// (see `og.rs`).
    og_image: &'static str,
    og_alt: &'static str,
    kind: Kind,
    /// Home uses the compact Kite nav (Manifesto only). Inner pages share
    /// Home / Manifesto / GitHub, which grows as forthcoming pages ship.
    compact_nav: bool,
    noindex: bool,
    script: Option<&'static str>,
    extra_preload: ExtraPreload,
}

#[derive(Clone, Copy)]
enum ExtraPreload {
    Home,
    Manifesto,
    None,
}

/// Built now. Forthcoming Kite pages are documented here so the next agent
/// only adds `built` content — do not invent their layout ahead of design.
const PAGES: &[Page] = &[
    Page {
        id: "home",
        dir: "",
        title: "Megabase — The Supabase API. One Rust binary.",
        description: "Unofficial open-source experiment: AI agents rewrite every Supabase service as one Rust binary, tested response by response against the real stack.",
        og_type: "website",
        og_image: "og/home.png",
        og_alt: "Megabase: The Supabase API. One Rust binary. Unofficial experiment. Not affiliated with or endorsed by Supabase, Inc.",
        kind: Kind::Home,
        compact_nav: true,
        noindex: false,
        script: None,
        extra_preload: ExtraPreload::Home,
    },
    Page {
        id: "manifesto",
        dir: "manifesto",
        title: "Manifesto — Megabase",
        description: "The Megabase manifesto: why AI agents are rewriting Supabase in Rust, the rules of the experiment, its scope, and how progress is judged.",
        og_type: "article",
        og_image: "og/manifesto.png",
        og_alt: "Megabase manifesto: Supabase, in Rust. By agents. In public. Not affiliated with or endorsed by Supabase, Inc.",
        kind: Kind::Manifesto,
        compact_nav: false,
        noindex: false,
        script: Some("toc.js"),
        extra_preload: ExtraPreload::Manifesto,
    },
    Page {
        id: "404",
        dir: "",
        title: "Not found — Megabase",
        description: "This unit is not implemented.",
        og_type: "website",
        og_image: "og-card.png",
        og_alt: "Megabase: The Supabase API. One Rust binary. Not affiliated with or endorsed by Supabase, Inc.",
        kind: Kind::Template("pages/not-found.html"),
        compact_nav: false,
        noindex: true,
        script: None,
        extra_preload: ExtraPreload::None,
    },
];

/// Forthcoming routes from the in-progress Kite site. Not built yet.
#[allow(dead_code)]
const FORTHCOMING: &[&str] = &[
    "status",
    "roadmap",
    "how-it-works",
    "components",
    "devlog",
    "human-log",
    "faq",
    "docs",
];

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut repo_root = None;
    let mut out = None;
    let site_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if args.get(1).map(String::as_str) == Some("og") {
        return og::generate(&site_root);
    }
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--repo-root" => {
                repo_root = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--out" => {
                out = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    let repo_root = repo_root.unwrap_or_else(|| site_root.parent().unwrap().to_path_buf());
    let out = out.unwrap_or_else(|| site_root.join("dist"));

    let metrics = metrics::load(&repo_root);
    build(&site_root, &repo_root, &out, &metrics)?;
    eprintln!(
        "Generated site from {}: {} passing, coverage {}, conformance {} → {}",
        metrics.source,
        metrics.passing_total_label(),
        metrics.coverage_label(),
        metrics.conformance_label(),
        out.display()
    );
    let _ = FORTHCOMING;
    Ok(())
}

fn same_path(a: &Path, b: &Path) -> bool {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let abs = |p: &Path| {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            cwd.join(p)
        }
    };
    match (abs(a).canonicalize(), abs(b).canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => abs(a) == abs(b),
    }
}

fn build(site_root: &Path, repo_root: &Path, out: &Path, metrics: &Metrics) -> io::Result<()> {
    // Never wipe the generator crate if --out points at site/ itself.
    if out.exists() && !same_path(out, site_root) {
        fs::remove_dir_all(out)?;
    }
    copy_dir(&site_root.join("static"), out)?;

    let templates = site_root.join("templates");
    let layout = fs::read_to_string(templates.join("layout.html"))?;
    let logo = fs::read_to_string(site_root.join("static/logo.svg"))?
        .trim()
        .to_string();

    for page in PAGES {
        let html = render_page(page, &layout, &logo, &templates, repo_root, metrics)?;
        let dest = if page.id == "404" {
            out.join("404.html")
        } else if page.dir.is_empty() {
            out.join("index.html")
        } else {
            let dir = out.join(page.dir);
            fs::create_dir_all(&dir)?;
            dir.join("index.html")
        };
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(dest, html)?;
    }

    fs::write(out.join("sitemap.xml"), sitemap())?;
    let cname = site_root.join("CNAME");
    let cname_out = out.join("CNAME");
    if cname.is_file() && !same_path(&cname, &cname_out) {
        fs::copy(&cname, cname_out)?;
    }
    Ok(())
}

fn render_page(
    page: &Page,
    layout: &str,
    logo: &str,
    templates: &Path,
    repo_root: &Path,
    metrics: &Metrics,
) -> io::Result<String> {
    // GitHub Pages serves 404.html at any missing path (e.g. /a/b/c), so its
    // links must be root-absolute; every other page stays relative.
    let is_404 = page.id == "404";
    let asset = if is_404 {
        "/"
    } else if page.dir.is_empty() {
        ""
    } else {
        "../"
    };
    let to_home = if is_404 {
        "/"
    } else if page.dir.is_empty() {
        "./"
    } else {
        "../"
    };
    let to_manifesto: String = if is_404 {
        "/manifesto/".into()
    } else if page.id == "manifesto" {
        "./".into()
    } else if page.dir.is_empty() {
        "manifesto/".into()
    } else {
        "../manifesto/".into()
    };
    let canonical = page_url(page);

    let mut vars = BTreeMap::new();
    vars.insert("title".into(), page.title.into());
    vars.insert("description".into(), page.description.into());
    // The 404 has no canonical URL: it must not claim to be the home page.
    vars.insert(
        "canonical_tags".into(),
        if is_404 {
            String::new()
        } else {
            format!(
                r#"<link rel="canonical" href="{canonical}" />
    <meta property="og:url" content="{canonical}" />"#
            )
        },
    );
    vars.insert("og_type".into(), page.og_type.into());
    vars.insert("og_image".into(), format!("{ORIGIN}/{}", page.og_image));
    vars.insert("og_alt".into(), page.og_alt.into());
    vars.insert("jsonld".into(), jsonld(page, &canonical));
    vars.insert("origin".into(), ORIGIN.into());
    vars.insert("asset".into(), asset.into());
    vars.insert("github".into(), GITHUB.into());
    vars.insert("logo".into(), logo.into());
    vars.insert("to_home".into(), to_home.into());
    vars.insert("to_manifesto".into(), to_manifesto.clone());
    vars.insert(
        "robots".into(),
        if page.noindex {
            r#"<meta name="robots" content="noindex" />"#.into()
        } else {
            String::new()
        },
    );
    vars.insert("preload".into(), preload(page, asset));
    vars.insert(
        "body_class".into(),
        if page.id == "manifesto" {
            "manifesto-page".into()
        } else {
            String::new()
        },
    );
    vars.insert(
        "page_class".into(),
        if page.id == "manifesto" {
            "page-manifesto".into()
        } else {
            String::new()
        },
    );
    vars.insert("header".into(), header(page, logo, &to_manifesto, to_home));
    vars.insert("footer".into(), footer(GITHUB));
    vars.insert(
        "scripts".into(),
        page.script
            .map(|file| format!(r#"<script src="{asset}{file}" defer></script>"#))
            .unwrap_or_default(),
    );

    vars.insert("passing_total".into(), metrics.passing_total_label());
    vars.insert("coverage".into(), metrics.coverage_label());
    vars.insert("conformance".into(), metrics.conformance_label());
    vars.insert(
        "coverage_conformance".into(),
        metrics.coverage_conformance_label(),
    );
    vars.insert("stage".into(), metrics.stage.clone());
    vars.insert("stage_short".into(), metrics.stage_short.clone());
    vars.insert("treemap".into(), treemap::svg(metrics));

    let content = match page.kind {
        Kind::Home => {
            let tpl = fs::read_to_string(templates.join("pages/home.html"))?;
            subst(&tpl, &vars)
        }
        Kind::Manifesto => {
            let md = fs::read_to_string(repo_root.join("MANIFESTO.md"))?;
            let article = manifesto::render(&md, metrics);
            let toc_desktop = manifesto::toc("desktop");
            let toc_mobile = manifesto::toc("mobile");
            format!(
                "{toc_mobile}\n<div class=\"manifesto-layout\">{toc_desktop}<main id=\"main\" class=\"article\">\n{article}\n</main></div>"
            )
        }
        Kind::Template(path) => {
            let tpl = fs::read_to_string(templates.join(path))?;
            subst(&tpl, &vars)
        }
    };
    vars.insert("content".into(), content);
    Ok(subst(layout, &vars))
}

fn header(page: &Page, logo: &str, manifesto_href: &str, to_home: &str) -> String {
    let mut links = String::new();
    if page.compact_nav {
        links.push_str(&nav_link(
            "MANIFESTO",
            manifesto_href,
            page.id == "manifesto",
        ));
    } else {
        links.push_str(&nav_link("HOME", to_home, page.id == "home"));
        links.push_str(&nav_link(
            "MANIFESTO",
            manifesto_href,
            page.id == "manifesto",
        ));
        links.push_str(&format!(
            r#"<a class="nav-link" href="{GITHUB}" rel="noopener noreferrer">GITHUB ↗</a>"#
        ));
    }
    format!(
        r#"<header class="site-header"><a class="brand" href="{to_home}">{logo}<span class="wordmark">MEGABASE</span></a><nav class="nav-links" aria-label="Primary">{links}</nav></header>"#
    )
}

fn nav_link(label: &str, href: &str, current: bool) -> String {
    if current {
        format!(r#"<a class="nav-link is-current" href="{href}" aria-current="page">{label}</a>"#)
    } else {
        format!(r#"<a class="nav-link" href="{href}">{label}</a>"#)
    }
}

fn footer(github: &str) -> String {
    format!(
        r#"<footer class="site-footer"><p class="footer-split">Independent experiment. Not affiliated with or endorsed by Supabase, Inc.</p><p class="footer-split">Apache-2.0 · <a href="{github}" rel="noopener noreferrer">GitHub</a></p><p class="footer-compact">Independent experiment. Not affiliated with or endorsed by Supabase, Inc. · Apache-2.0</p></footer>"#
    )
}

fn preload(page: &Page, asset: &str) -> String {
    match page.extra_preload {
        ExtraPreload::Home => format!(
            r#"<link rel="preload" href="{asset}fonts/JetBrainsMono-Regular.woff2" as="font" type="font/woff2" crossorigin /><link rel="preload" href="{asset}fonts/JetBrainsMono-Bold.woff2" as="font" type="font/woff2" crossorigin />"#
        ),
        ExtraPreload::Manifesto => format!(
            r#"<link rel="preload" href="{asset}fonts/JetBrainsMono-Bold.woff2" as="font" type="font/woff2" crossorigin /><link rel="preload" href="{asset}fonts/Inter-Regular.woff2" as="font" type="font/woff2" crossorigin />"#
        ),
        ExtraPreload::None => String::new(),
    }
}

fn sitemap() -> String {
    let lastmod = build_date();
    let mut urls = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#,
    );
    for page in PAGES {
        if page.noindex {
            continue;
        }
        let loc = page_url(page);
        urls.push_str(&format!(
            "<url><loc>{loc}</loc><lastmod>{lastmod}</lastmod></url>"
        ));
    }
    urls.push_str("</urlset>\n");
    urls
}

fn page_url(page: &Page) -> String {
    if page.dir.is_empty() {
        format!("{ORIGIN}/")
    } else {
        format!("{ORIGIN}/{}/", page.dir)
    }
}

/// Build date (UTC, YYYY-MM-DD) for sitemap `lastmod`. Honours
/// `SOURCE_DATE_EPOCH` for reproducible builds.
fn build_date() -> String {
    let secs = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        });
    // Civil-from-days (Howard Hinnant).
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// schema.org JSON-LD. Home: Organization + WebSite + SoftwareSourceCode.
/// Inner pages: an Article/WebPage with a breadcrumb. 404: none.
fn jsonld(page: &Page, url: &str) -> String {
    use serde_json::json;

    let org = json!({
        "@type": "Organization",
        "@id": format!("{ORIGIN}/#organization"),
        "name": "Megabase",
        "url": format!("{ORIGIN}/"),
        "logo": {
            "@type": "ImageObject",
            "url": format!("{ORIGIN}/icon-512.png"),
            "width": 512,
            "height": 512
        },
        "sameAs": [GITHUB]
    });
    let website = json!({
        "@type": "WebSite",
        "@id": format!("{ORIGIN}/#website"),
        "name": "Megabase",
        "url": format!("{ORIGIN}/"),
        "inLanguage": "en",
        "description": format!(
            "Unofficial open-source experiment: AI agents rewrite Supabase as one Rust binary. {}",
            og::DISCLAIMER
        ),
        "publisher": { "@id": format!("{ORIGIN}/#organization") }
    });
    let image = format!("{ORIGIN}/{}", page.og_image);
    let graph = match page.id {
        "404" => return String::new(),
        "home" => vec![
            org,
            website,
            json!({
                "@type": "SoftwareSourceCode",
                "@id": format!("{ORIGIN}/#code"),
                "name": "Megabase",
                "description": page.description,
                "url": format!("{ORIGIN}/"),
                "codeRepository": GITHUB,
                "programmingLanguage": { "@type": "ComputerLanguage", "name": "Rust" },
                "license": "https://www.apache.org/licenses/LICENSE-2.0",
                "author": { "@id": format!("{ORIGIN}/#organization") },
                "image": image
            }),
        ],
        _ => {
            let kind = if page.og_type == "article" {
                "Article"
            } else {
                "WebPage"
            };
            let name = page.title.split(" — ").next().unwrap_or(page.title);
            vec![
                org,
                website,
                json!({
                    "@type": kind,
                    "@id": format!("{url}#page"),
                    "url": url,
                    "headline": name,
                    "name": page.title,
                    "description": page.description,
                    "inLanguage": "en",
                    "image": image,
                    "isPartOf": { "@id": format!("{ORIGIN}/#website") },
                    "author": { "@id": format!("{ORIGIN}/#organization") },
                    "publisher": { "@id": format!("{ORIGIN}/#organization") }
                }),
                json!({
                    "@type": "BreadcrumbList",
                    "itemListElement": [
                        { "@type": "ListItem", "position": 1, "name": "Megabase", "item": format!("{ORIGIN}/") },
                        { "@type": "ListItem", "position": 2, "name": name, "item": url }
                    ]
                }),
            ]
        }
    };
    let doc = json!({ "@context": "https://schema.org", "@graph": graph });
    // `<` is escaped so no value can close the <script> element early.
    let body = doc.to_string().replace('<', "\\u003c");
    format!(r#"<script type="application/ld+json">{body}</script>"#)
}

fn subst(tpl: &str, vars: &BTreeMap<String, String>) -> String {
    let mut out = tpl.to_string();
    let mut keys: Vec<_> = vars.keys().collect();
    keys.sort_by_key(|k| std::cmp::Reverse(k.len()));
    for key in keys {
        let needle = format!("{{{{{key}}}}}");
        if let Some(value) = vars.get(key) {
            out = out.replace(&needle, value);
        }
    }
    out
}

fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let from = entry.path();
        let to = dst.join(&name);
        if entry.file_type()?.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
