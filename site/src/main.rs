//! Static generator for megabase.sh.
//!
//! Shared chrome lives in `chrome.rs`. Page bodies live in `pages.rs` or,
//! for the manifesto, are rendered from `MANIFESTO.md`. Coverage numbers
//! and the nested treemap are inlined at build time from `coverage/` when
//! those files exist; otherwise Day-0 placeholders use an em dash.
//!
//! Usage:
//!   cargo run --manifest-path site/Cargo.toml -- --repo-root . --out _site

mod chrome;
mod devlog;
mod docs;
mod html;
mod human_log;
mod manifesto;
mod markdown;
mod metrics;
mod og;
mod pages;
mod treemap;

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrome::Paths;
use html::{esc, subst};
use metrics::Metrics;

pub const GITHUB: &str = "https://github.com/Zouhairmaj/megabase";
const ORIGIN: &str = "https://megabase.sh";

#[derive(Clone, Copy)]
enum Kind {
    Home,
    Manifesto,
    HowItWorks,
    Status,
    Roadmap,
    Components,
    Devlog,
    HumanLog,
    Faq,
    Docs,
    NotFound,
}

#[derive(Clone, Copy)]
struct Page {
    id: &'static str,
    dir: &'static str,
    title: &'static str,
    description: &'static str,
    og_type: &'static str,
    og_image: &'static str,
    og_alt: &'static str,
    kind: Kind,
    noindex: bool,
    extra_preload: ExtraPreload,
}

#[derive(Clone, Copy)]
enum ExtraPreload {
    Home,
    Manifesto,
    Inter,
    None,
}

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
        noindex: false,
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
        noindex: false,
        extra_preload: ExtraPreload::Manifesto,
    },
    Page {
        id: "how-it-works",
        dir: "how-it-works",
        title: "How it works — Megabase",
        description: "How Megabase is built: agents read pinned upstream source, implement one unit, and an external judge compares every response with real Supabase.",
        og_type: "website",
        og_image: "og/how-it-works.png",
        og_alt: "How Megabase works: agents read the source. A judge they cannot touch keeps score.",
        kind: Kind::HowItWorks,
        noindex: false,
        extra_preload: ExtraPreload::Inter,
    },
    Page {
        id: "status",
        dir: "status",
        title: "Status — Megabase",
        description: "Live experiment status: units passing the judge, coverage, conformance, and the nested component treemap, regenerated on every commit.",
        og_type: "website",
        og_image: "og/status.png",
        og_alt: "Megabase status: where the experiment stands. Not affiliated with or endorsed by Supabase, Inc.",
        kind: Kind::Status,
        noindex: false,
        extra_preload: ExtraPreload::Home,
    },
    Page {
        id: "roadmap",
        dir: "roadmap",
        title: "Roadmap — Megabase",
        description: "Five public levels, gated in order: REST and Auth, Storage, Realtime, Functions and the Studio test, then Studio itself.",
        og_type: "website",
        og_image: "og/roadmap.png",
        og_alt: "Megabase roadmap: five levels, gated in order. No skipping.",
        kind: Kind::Roadmap,
        noindex: false,
        extra_preload: ExtraPreload::Inter,
    },
    Page {
        id: "components",
        dir: "components",
        title: "Supabase components — Megabase",
        description: "Every Supabase-authored service in scope, one Rust crate each: REST, Auth, Storage, Realtime, Functions, Pooler, Meta, Studio.",
        og_type: "website",
        og_image: "og/components.png",
        og_alt: "Supabase components in Megabase: every service Supabase wrote. One crate each.",
        kind: Kind::Components,
        noindex: false,
        extra_preload: ExtraPreload::Home,
    },
    Page {
        id: "devlog",
        dir: "devlog",
        title: "Devlog — Megabase",
        description: "One short entry per day: what the agents did, written for humans, from files in devlog/.",
        og_type: "website",
        og_image: "og/devlog.png",
        og_alt: "Megabase devlog: what the agents did today, written for humans.",
        kind: Kind::Devlog,
        noindex: false,
        extra_preload: ExtraPreload::Inter,
    },
    Page {
        id: "human-log",
        dir: "human-log",
        title: "Human log — Megabase",
        description: "Every human intervention on the Megabase experiment, with the reason. The count is part of the result.",
        og_type: "website",
        og_image: "og/human-log.png",
        og_alt: "Megabase human log: every time a human touched the experiment.",
        kind: Kind::HumanLog,
        noindex: false,
        extra_preload: ExtraPreload::Inter,
    },
    Page {
        id: "faq",
        dir: "faq",
        title: "FAQ — Megabase",
        description: "Is this made by Supabase? Can I use it today? How is correctness judged? Answers from the manifesto.",
        og_type: "website",
        og_image: "og/faq.png",
        og_alt: "Megabase FAQ: questions people ask.",
        kind: Kind::Faq,
        noindex: false,
        extra_preload: ExtraPreload::Inter,
    },
    Page {
        id: "docs",
        dir: "docs",
        title: "Docs — Megabase",
        description: "Megabase documentation, rendered from markdown in docs/ on every commit. Day 0: nothing passes yet.",
        og_type: "website",
        og_image: "og/docs.png",
        og_alt: "Megabase docs: getting started. Day 0, nothing works yet.",
        kind: Kind::Docs,
        noindex: false,
        extra_preload: ExtraPreload::Inter,
    },
    Page {
        id: "404",
        dir: "",
        title: "Not found — Megabase",
        description: "This page does not exist. Failures are loud.",
        og_type: "website",
        og_image: "og-card.png",
        og_alt: "Megabase: The Supabase API. One Rust binary. Not affiliated with or endorsed by Supabase, Inc.",
        kind: Kind::NotFound,
        noindex: true,
        extra_preload: ExtraPreload::None,
    },
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

    let mut metrics = metrics::load(&repo_root);
    let human_md = fs::read_to_string(repo_root.join("HUMAN_LOG.md")).unwrap_or_default();
    let human = human_log::load(&human_md);
    metrics.human_interventions = human.completed;

    build(&site_root, &repo_root, &out, &metrics, &human)?;
    eprintln!(
        "Generated site from {}: {} passing, coverage {}, conformance {} → {}",
        metrics.source,
        metrics.passing_total_label(),
        metrics.coverage_label(),
        metrics.conformance_label(),
        out.display()
    );
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

struct Shell<'a> {
    layout: &'a str,
    logo: &'a str,
    version: &'a str,
}

struct SiteData<'a> {
    metrics: &'a Metrics,
    human: &'a human_log::HumanLog,
    entries: &'a [devlog::Entry],
    docs: &'a [docs::Doc],
    roadmap_md: Option<&'a str>,
    sha: &'a str,
    date: &'a str,
}

fn build(
    site_root: &Path,
    repo_root: &Path,
    out: &Path,
    metrics: &Metrics,
    human: &human_log::HumanLog,
) -> io::Result<()> {
    let version = chrome::read_workspace_version(repo_root)?;
    if out.exists() && !same_path(out, site_root) {
        fs::remove_dir_all(out)?;
    }
    copy_dir(&site_root.join("static"), out)?;

    let layout = fs::read_to_string(site_root.join("templates/layout.html"))?;
    let logo = fs::read_to_string(site_root.join("static/logo.svg"))?
        .trim()
        .to_string();
    let shell = Shell {
        layout: &layout,
        logo: &logo,
        version: &version,
    };
    let entries = devlog::load(repo_root)?;
    let site_docs = docs::load(repo_root)?;
    let sha = git_sha7(repo_root);
    let date = build_date();
    let roadmap_md = fs::read_to_string(repo_root.join("docs/ROADMAP.md")).ok();
    let data = SiteData {
        metrics,
        human,
        entries: &entries,
        docs: &site_docs,
        roadmap_md: roadmap_md.as_deref(),
        sha: &sha,
        date: &date,
    };

    for page in PAGES {
        let html = render_page(page, &shell, repo_root, &data, None)?;
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

    for entry in &entries {
        let article = render_article(entry, &shell)?;
        let dir = out.join("devlog").join(&entry.slug);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("index.html"), article)?;
    }

    for doc in &site_docs {
        let article = render_docs_article(doc, &site_docs, &shell, metrics, &sha, &date)?;
        let dir = out.join("docs").join(&doc.slug);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("index.html"), article)?;
    }

    fs::write(out.join("sitemap.xml"), sitemap(&entries, &site_docs))?;
    let cname = site_root.join("CNAME");
    let cname_out = out.join("CNAME");
    if cname.is_file() && !same_path(&cname, &cname_out) {
        fs::copy(&cname, cname_out)?;
    }
    write_shields(repo_root, out)?;
    write_treemap_pngs(repo_root, out)?;
    Ok(())
}

/// Shields.io endpoint JSON plus the summary the units badge reads.
/// Only these files; never copy treemap SVGs (inlined at build time).
fn write_shields(repo_root: &Path, out: &Path) -> io::Result<()> {
    const FILES: &[&str] = &[
        "badge-coverage.json",
        "badge-conformance.json",
        "summary.json",
        "judge-results.json",
        "judge-history.json",
    ];
    let src_dir = repo_root.join("coverage");
    let dest_dir = out.join("coverage");
    let mut created = false;
    for name in FILES {
        let src = src_dir.join(name);
        if src.is_file() {
            if !created {
                fs::create_dir_all(&dest_dir)?;
                created = true;
            }
            fs::copy(&src, dest_dir.join(name))?;
        }
    }
    Ok(())
}

/// PNG rasters of the coverage treemaps, for the GitHub README.
///
/// The Status page inlines its own SVG. GitHub's README renderer does not
/// display an SVG loaded from `gh-pages`, so `pages-badges.yml` publishes
/// these PNGs next to the shields JSON. Paths are omitted when the source
/// SVG is absent (placeholder builds).
fn write_treemap_pngs(repo_root: &Path, out: &Path) -> io::Result<()> {
    const FILES: &[(&str, &str)] = &[
        ("treemap.svg", "treemap.png"),
        ("treemap-light.svg", "treemap-light.png"),
    ];
    let src_dir = repo_root.join("coverage");
    let dest_dir = out.join("coverage");
    for (svg_name, png_name) in FILES {
        let src = src_dir.join(svg_name);
        if !src.is_file() {
            continue;
        }
        let png = rasterize_svg(&fs::read_to_string(&src)?)?;
        fs::create_dir_all(&dest_dir)?;
        fs::write(dest_dir.join(png_name), png)?;
    }
    Ok(())
}

fn rasterize_svg(svg: &str) -> io::Result<Vec<u8>> {
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_str(svg, &opt)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    let size = tree.size();
    let scale = 2.0_f32;
    let width = (size.width() * scale).ceil() as u32;
    let height = (size.height() * scale).ceil() as u32;
    if width == 0 || height == 0 {
        return Err(io::Error::other("treemap svg has no area"));
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| io::Error::other("pixmap allocation failed"))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|err| io::Error::other(err.to_string()))
}

fn render_page(
    page: &Page,
    shell: &Shell<'_>,
    repo_root: &Path,
    data: &SiteData<'_>,
    loc: Option<&str>,
) -> io::Result<String> {
    let is_404 = page.id == "404";
    let paths = if is_404 {
        Paths::home(true)
    } else if page.dir.is_empty() {
        Paths::home(false)
    } else {
        Paths::nested(page.id, false)
    };
    wrap(page, shell, &paths, loc, |paths| {
        content(page, paths, repo_root, data)
    })
}

fn render_article(entry: &devlog::Entry, shell: &Shell<'_>) -> io::Result<String> {
    let paths = Paths::article("devlog-article");
    let title = format!("{} — Megabase", entry.title);
    let description = if entry.summary.is_empty() {
        "A daily Megabase agent log.".to_string()
    } else {
        entry.summary.clone()
    };
    let loc = format!("{ORIGIN}/devlog/{}/", entry.slug);
    let page = Page {
        id: "devlog-article",
        dir: "devlog",
        title: Box::leak(title.into_boxed_str()),
        description: Box::leak(description.into_boxed_str()),
        og_type: "article",
        og_image: "og/devlog.png",
        og_alt: "Megabase devlog: what the agents did today, written for humans.",
        kind: Kind::Devlog,
        noindex: false,
        extra_preload: ExtraPreload::Inter,
    };
    wrap(&page, shell, &paths, Some(&loc), |paths| {
        Ok(pages::devlog_article(paths, entry))
    })
}

fn render_docs_article(
    doc: &docs::Doc,
    all: &[docs::Doc],
    shell: &Shell<'_>,
    metrics: &Metrics,
    sha: &str,
    date: &str,
) -> io::Result<String> {
    let paths = Paths::docs_article();
    let title = format!("{} — Megabase", doc.title);
    let description = if doc.description.is_empty() {
        format!("Megabase docs: {}.", doc.title)
    } else {
        doc.description.clone()
    };
    let loc = format!("{ORIGIN}/docs/{}/", doc.slug);
    let page = Page {
        id: Box::leak(format!("docs-{}", doc.slug).into_boxed_str()),
        dir: Box::leak(format!("docs/{}", doc.slug).into_boxed_str()),
        title: Box::leak(title.into_boxed_str()),
        description: Box::leak(description.into_boxed_str()),
        og_type: "article",
        og_image: "og/docs.png",
        og_alt: "Megabase docs: getting started. Day 0, nothing works yet.",
        kind: Kind::Docs,
        noindex: false,
        extra_preload: ExtraPreload::Inter,
    };
    wrap(&page, shell, &paths, Some(&loc), |paths| {
        Ok(docs::article(paths, all, doc, metrics, sha, date))
    })
}

fn content(
    page: &Page,
    paths: &Paths,
    repo_root: &Path,
    data: &SiteData<'_>,
) -> io::Result<String> {
    Ok(match page.kind {
        Kind::Home => pages::home(paths, data.metrics),
        Kind::Manifesto => {
            let md = fs::read_to_string(repo_root.join("MANIFESTO.md"))?;
            let article = manifesto::render(&md, data.metrics);
            let toc_desktop = manifesto::toc("desktop");
            let toc_mobile = manifesto::toc("mobile");
            format!(
                "{toc_mobile}\n<div class=\"manifesto-layout\">{toc_desktop}<div id=\"main\" class=\"article\" role=\"main\">\n{article}\n</div></div>"
            )
        }
        Kind::HowItWorks => pages::how_it_works(paths, data.metrics),
        Kind::Status => pages::status(paths, data.metrics),
        Kind::Roadmap => pages::roadmap(paths, data.metrics, data.roadmap_md),
        Kind::Components => pages::components(paths, data.metrics),
        Kind::Devlog => pages::devlog_index(paths, data.entries),
        Kind::HumanLog => pages::human_log(paths, data.metrics, &data.human.html),
        Kind::Faq => pages::faq(paths, data.metrics),
        Kind::Docs => docs::index(paths, data.docs, data.metrics, data.sha, data.date),
        Kind::NotFound => pages::not_found(paths),
    })
}

fn wrap(
    page: &Page,
    shell: &Shell<'_>,
    paths: &Paths,
    loc: Option<&str>,
    body: impl FnOnce(&Paths) -> io::Result<String>,
) -> io::Result<String> {
    let is_404 = page.id == "404";
    let asset = paths.asset();
    let canonical = loc.map(str::to_string).unwrap_or_else(|| page_url(page));
    let mut vars = BTreeMap::new();
    vars.insert("title".into(), esc(page.title));
    vars.insert("description".into(), esc(page.description));
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
    vars.insert("og_alt".into(), esc(page.og_alt));
    vars.insert("jsonld".into(), jsonld(page, &canonical));
    vars.insert("origin".into(), ORIGIN.into());
    vars.insert("asset".into(), asset.into());
    vars.insert("github".into(), GITHUB.into());
    vars.insert("logo".into(), shell.logo.into());
    vars.insert(
        "robots".into(),
        if page.noindex {
            r#"<meta name="robots" content="noindex" />"#.into()
        } else {
            String::new()
        },
    );
    vars.insert("preload".into(), preload(page, asset));
    let body_class = if page.id == "manifesto" {
        "manifesto-page"
    } else if page.id == "docs" || page.id.starts_with("docs-") {
        "docs-page"
    } else {
        ""
    };
    vars.insert("body_class".into(), body_class.into());
    vars.insert(
        "page_class".into(),
        match page.id {
            "manifesto" => "page-manifesto",
            "home" => "page-home",
            _ => "page-inner",
        }
        .into(),
    );
    vars.insert(
        "header".into(),
        chrome::header(paths, shell.logo, shell.version),
    );
    vars.insert("footer".into(), chrome::footer(paths, shell.logo));
    let needs_copy =
        matches!(page.kind, Kind::Docs | Kind::HowItWorks) || page.id.starts_with("docs-");
    let mut scripts = String::new();
    if page.id == "manifesto" {
        scripts.push_str(&format!(r#"<script src="{asset}toc.js" defer></script>"#));
    }
    if needs_copy {
        scripts.push_str(&format!(r#"<script src="{asset}copy.js" defer></script>"#));
    }
    vars.insert("scripts".into(), scripts);
    vars.insert("content".into(), body(paths)?);
    Ok(subst(shell.layout, &vars))
}

fn preload(page: &Page, asset: &str) -> String {
    match page.extra_preload {
        ExtraPreload::Home => format!(
            r#"<link rel="preload" href="{asset}fonts/JetBrainsMono-Regular.woff2" as="font" type="font/woff2" crossorigin /><link rel="preload" href="{asset}fonts/JetBrainsMono-Bold.woff2" as="font" type="font/woff2" crossorigin />"#
        ),
        ExtraPreload::Manifesto => format!(
            r#"<link rel="preload" href="{asset}fonts/JetBrainsMono-Bold.woff2" as="font" type="font/woff2" crossorigin /><link rel="preload" href="{asset}fonts/Inter-Regular.woff2" as="font" type="font/woff2" crossorigin />"#
        ),
        ExtraPreload::Inter => format!(
            r#"<link rel="preload" href="{asset}fonts/JetBrainsMono-Regular.woff2" as="font" type="font/woff2" crossorigin /><link rel="preload" href="{asset}fonts/Inter-Regular.woff2" as="font" type="font/woff2" crossorigin />"#
        ),
        ExtraPreload::None => String::new(),
    }
}

fn sitemap(entries: &[devlog::Entry], docs_pages: &[docs::Doc]) -> String {
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
    for entry in entries {
        urls.push_str(&format!(
            "<url><loc>{ORIGIN}/devlog/{}/</loc><lastmod>{}</lastmod></url>",
            entry.slug, entry.date
        ));
    }
    for doc in docs_pages {
        urls.push_str(&format!(
            "<url><loc>{ORIGIN}/docs/{}/</loc><lastmod>{lastmod}</lastmod></url>",
            doc.slug
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

fn git_sha7(repo_root: &Path) -> String {
    if let Ok(sha) = std::env::var("GITHUB_SHA") {
        let short: String = sha.chars().take(7).collect();
        if short.len() == 7 {
            return short;
        }
    }
    Command::new("git")
        .args([
            "-C",
            repo_root.to_str().unwrap_or("."),
            "rev-parse",
            "--short=7",
            "HEAD",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| s.len() == 7)
        .unwrap_or_else(|| "unknown".into())
}

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
        id if id.starts_with("docs-") => {
            let name = page.title.split(" — ").next().unwrap_or(page.title);
            vec![
                org,
                website,
                json!({
                    "@type": "TechArticle",
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
                        { "@type": "ListItem", "position": 2, "name": "Docs", "item": format!("{ORIGIN}/docs/") },
                        { "@type": "ListItem", "position": 3, "name": name, "item": url }
                    ]
                }),
            ]
        }
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
    let body = doc.to_string().replace('<', "\\u003c");
    format!(r#"<script type="application/ld+json">{body}</script>"#)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterize_svg_writes_a_png() {
        let png = rasterize_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="4" height="2" fill="#00D892"/></svg>"##,
        )
        .expect("svg");
        assert!(png.starts_with(b"\x89PNG"));
        assert!(png.len() > 8);
    }

    fn generate_tmp() -> PathBuf {
        generate_tmp_with(|_| {})
    }

    /// Isolated repo root: real manifesto/devlog/human-log, never `coverage/`.
    fn generate_tmp_with(extra: impl FnOnce(&Path)) -> PathBuf {
        let site_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let real_root = site_root.parent().unwrap().to_path_buf();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp_root = std::env::temp_dir().join(format!("megabase-site-root-{stamp}"));
        let out = std::env::temp_dir().join(format!("megabase-site-{stamp}"));
        seed_repo_without_coverage(&real_root, &tmp_root);
        extra(&tmp_root);
        let mut metrics = metrics::load(&tmp_root);
        let human =
            human_log::load(&fs::read_to_string(tmp_root.join("HUMAN_LOG.md")).unwrap_or_default());
        metrics.human_interventions = human.completed;
        build(&site_root, &tmp_root, &out, &metrics, &human).expect("build");
        let _ = fs::remove_dir_all(&tmp_root);
        out
    }

    fn seed_repo_without_coverage(real_root: &Path, tmp_root: &Path) {
        fs::create_dir_all(tmp_root).expect("tmp root");
        for name in ["MANIFESTO.md", "HUMAN_LOG.md"] {
            let src = real_root.join(name);
            if src.is_file() {
                fs::copy(&src, tmp_root.join(name)).expect(name);
            }
        }
        fs::write(
            tmp_root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"0.0.0\"\n",
        )
        .expect("cargo toml");
        let docs_dir = real_root.join("docs");
        if docs_dir.is_dir() {
            let dest = tmp_root.join("docs");
            fs::create_dir_all(&dest).expect("docs");
            for entry in fs::read_dir(&docs_dir).expect("read docs") {
                let entry = entry.expect("docs entry");
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    fs::copy(&path, dest.join(entry.file_name())).expect("docs copy");
                }
            }
        }
        let devlog = real_root.join("devlog");
        if devlog.is_dir() {
            let dest = tmp_root.join("devlog");
            fs::create_dir_all(&dest).expect("devlog");
            for entry in fs::read_dir(&devlog).expect("read devlog") {
                let entry = entry.expect("devlog entry");
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    fs::copy(&path, dest.join(entry.file_name())).expect("devlog copy");
                }
            }
        }
    }

    #[test]
    fn day0_does_not_invent_a_unit_total() {
        let out = generate_tmp();
        let home = fs::read_to_string(out.join("index.html")).unwrap();
        assert!(
            !home.contains("334"),
            "placeholder must not hardcode the Kite mock 334"
        );
        assert!(
            home.contains("—"),
            "day-0 pages must use an em dash, not an invented unit total"
        );
        assert!(home.contains("Not affiliated with or endorsed by Supabase, Inc."));
        assert!(!home.to_ascii_lowercase().contains("oxide"));
        assert!(home.contains("EXPERIMENT STATUS"));
        assert!(home.contains("updated on every commit"));
        assert!(home.contains("treemap-svg"));
        assert!(home.contains("visually-hidden"));
        assert!(home.contains("Coverage · Conformance"));
        assert!(home.contains("Units passing the judge"));
        assert!(home.contains("https://analytics.ahrefs.com/analytics.js"));
        assert!(home.contains("NOTHING PASSES YET"));
        assert!(!home.contains("footer-measure"));
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn every_kite_route_is_written() {
        let out = generate_tmp();
        for rel in [
            "index.html",
            "manifesto/index.html",
            "how-it-works/index.html",
            "status/index.html",
            "roadmap/index.html",
            "components/index.html",
            "devlog/index.html",
            "human-log/index.html",
            "faq/index.html",
            "docs/index.html",
            "docs/quickstart/index.html",
            "docs/install/index.html",
            "404.html",
            "sitemap.xml",
        ] {
            assert!(out.join(rel).is_file(), "missing {rel}");
        }
        let sitemap = fs::read_to_string(out.join("sitemap.xml")).unwrap();
        assert!(sitemap.contains("https://megabase.sh/status/"));
        assert!(sitemap.contains("https://megabase.sh/docs/quickstart/"));
        assert!(!sitemap.contains("/404"));
        let docs_index = fs::read_to_string(out.join("docs/index.html")).unwrap();
        assert!(docs_index.contains("docs-card"));
        assert!(docs_index.contains("aria-label=\"Install\""));
        assert!(!docs_index.to_ascii_lowercase().contains("oxide"));
        assert!(!docs_index.contains("334"));
        assert!(docs_index.contains("nav-link is-current"));
        let quick = fs::read_to_string(out.join("docs/quickstart/index.html")).unwrap();
        assert!(quick.contains("aria-current=\"page\""));
        assert!(quick.contains("<details"));
        assert!(quick.contains("On this page"));
        assert!(quick.contains("docs-nav-planned"));
        assert!(quick.contains("TechArticle"));
        assert!(quick.contains("Built from commit"));
        assert!(quick.contains("callout-note"));
        assert!(quick.contains("data-copy"));
        assert!(!quick.contains("334"));
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn header_badge_uses_the_workspace_version_on_every_page() {
        let out = generate_tmp_with(|root| {
            fs::write(
                root.join("Cargo.toml"),
                "[package]\nversion = \"9.9.9\"\n\n[workspace.package]\nversion = \"1.2.3\"\n",
            )
            .expect("version fixture");
        });
        let needle = r#"<a class="version-badge" href="https://github.com/Zouhairmaj/megabase/releases/tag/v1.2.3" rel="noopener noreferrer" title="v1.2.3"><span class="version-badge-label">v1.2.3</span></a>"#;
        for rel in [
            "index.html",
            "manifesto/index.html",
            "status/index.html",
            "docs/index.html",
            "docs/quickstart/index.html",
            "404.html",
        ] {
            let html =
                fs::read_to_string(out.join(rel)).unwrap_or_else(|_| panic!("missing {rel}"));
            assert!(html.contains(needle), "{rel} missing release badge");
            assert!(
                !html.contains("v9.9.9"),
                "{rel} used the site package version"
            );
            let footer = html.split("site-footer").nth(1).expect("footer");
            assert!(
                !footer.contains("version-badge"),
                "{rel} footer has a badge"
            );
        }
        let css = fs::read_to_string(out.join("styles.css")).unwrap();
        assert!(css.contains(".version-badge"));
        assert!(css.contains("#00d89214"));
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn status_inlines_generated_treemap_never_coverage_svgs() {
        let out = generate_tmp();
        let html = fs::read_to_string(out.join("status/index.html")).unwrap();
        assert!(html.contains("data-component=\"rest\""));
        assert!(html.contains("treemap-svg"));
        assert!(!html.contains("coverage/treemap.svg"));
        assert!(!html.contains("coverage/treemap-light.svg"));
        assert!(!html.contains("<picture"));
        assert!(!out.join("coverage/treemap.svg").exists());
        assert!(!out.join("coverage/badge-coverage.json").exists());
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn coverage_treemap_files_are_never_copied_or_linked() {
        let out = generate_tmp_with(|root| {
            let cov = root.join("coverage");
            fs::create_dir_all(&cov).expect("coverage dir");
            fs::write(
                cov.join("treemap.svg"),
                "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
            )
            .expect("dark svg");
            fs::write(
                cov.join("treemap-light.svg"),
                "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
            )
            .expect("light svg");
        });
        let html = fs::read_to_string(out.join("status/index.html")).unwrap();
        assert!(!html.contains("coverage/treemap.svg"));
        assert!(!html.contains("coverage/treemap-light.svg"));
        assert!(!out.join("coverage/treemap.svg").exists());
        assert!(!out.join("coverage/badge-coverage.json").exists());
        assert!(html.contains("data-component=\"rest\""));
        let _ = fs::remove_dir_all(&out);
    }

    fn assert_treemap_svgs_are_fluid(html: &str) {
        let mut rest = html;
        while let Some(at) = rest.find("<svg") {
            let tag_end = rest[at..].find('>').expect("svg tag");
            let tag = &rest[at..at + tag_end];
            if tag.contains("treemap-svg") {
                assert!(
                    tag.contains("width=\"100%\""),
                    "treemap svg must use width=100%: {tag}"
                );
                assert!(
                    tag.contains("height=\"auto\""),
                    "treemap svg must use height=auto: {tag}"
                );
                for attr in tag.split_whitespace() {
                    if let Some(val) = attr
                        .strip_prefix("width=\"")
                        .and_then(|s| s.strip_suffix('"'))
                    {
                        assert_eq!(val, "100%", "fixed treemap width {val}");
                    }
                }
            }
            rest = &rest[at + 4..];
        }
    }

    #[test]
    fn treemap_svgs_have_no_fixed_width_wider_than_container() {
        let out = generate_tmp();
        for rel in ["index.html", "status/index.html"] {
            let html = fs::read_to_string(out.join(rel)).unwrap();
            assert_treemap_svgs_are_fluid(&html);
        }
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn live_coverage_pages_use_comma_grouped_unit_total() {
        let site_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let real_root = site_root.parent().unwrap().to_path_buf();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let out = std::env::temp_dir().join(format!("megabase-site-live-{stamp}"));
        let metrics = metrics::load(&real_root);
        let human = human_log::load(
            &fs::read_to_string(real_root.join("HUMAN_LOG.md")).unwrap_or_default(),
        );
        build(&site_root, &real_root, &out, &metrics, &human).expect("build live");
        let home = fs::read_to_string(out.join("index.html")).unwrap();
        let status = fs::read_to_string(out.join("status/index.html")).unwrap();
        let total = metrics
            .total
            .expect("live coverage must include totals.units");
        assert!(total > 0, "live coverage must have units");
        let shown = metrics::comma(total);
        assert!(
            home.contains(&shown),
            "home must show summary.totals.units ({shown})"
        );
        assert!(
            status.contains(&shown),
            "status must show summary.totals.units ({shown})"
        );
        assert!(!home.contains(">334<") && !status.contains(">334<"));
        assert_treemap_svgs_are_fluid(&home);
        assert_treemap_svgs_are_fluid(&status);
        assert!(!home.contains("coverage/treemap.svg"));
        assert!(!status.contains("coverage/treemap-light.svg"));
        assert!(!status.to_ascii_lowercase().contains("oxide"));
        assert!(!home.to_ascii_lowercase().contains("the spend"));
        assert!(
            home.contains(r#"class="tm-cells implemented""#),
            "home hero/live treemaps must paint implemented units"
        );
        assert!(home.contains("#005441"));
        assert!(status.contains(r#"class="tm-cells implemented""#));
        assert!(status.contains("#005441"));
        assert!(home.contains("status-panel-legend"));
        assert!(home.contains("Units passing the judge"));
        assert!(status.contains("conformant (matches real Supabase)"));
        assert!(
            out.join("coverage/badge-coverage.json").is_file(),
            "live site must publish shields coverage JSON"
        );
        assert!(
            out.join("coverage/badge-conformance.json").is_file(),
            "live site must publish shields conformance JSON"
        );
        assert!(out.join("coverage/summary.json").is_file());
        assert!(!out.join("coverage/treemap.svg").exists());
        assert!(
            out.join("coverage/treemap.png").is_file(),
            "README treemap PNG is published with the shields JSON"
        );
        assert!(out.join("coverage/treemap-light.png").is_file());
        let png = fs::read(out.join("coverage/treemap.png")).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn no_horizontal_overflow_if_chrome() {
        let Some(chrome) = find_chrome() else {
            return;
        };
        let out = build_live_site("chrome");
        for (page, width) in [
            ("index.html", 1440u32),
            ("status/index.html", 1440),
            ("index.html", 390),
            ("status/index.html", 390),
        ] {
            let src = out.join(page);
            let wrapper = src.with_file_name(format!("probe-{width}.html"));
            let injected = inject_probe(
                &fs::read_to_string(&src).unwrap(),
                "document.documentElement.setAttribute('data-sw', String(document.documentElement.scrollWidth));document.documentElement.setAttribute('data-cw', String(document.documentElement.clientWidth));",
            );
            fs::write(&wrapper, injected).unwrap();
            let uri = format!("file://{}", wrapper.display());
            let profile = out.join(format!("chrome-profile-{width}-{}", page.replace('/', "-")));
            fs::create_dir_all(&profile).unwrap();
            let dom = chrome_dump_dom(
                &chrome,
                &uri,
                width,
                &profile,
                std::time::Duration::from_secs(20),
            );
            let sw = attr_after(&dom, "data-sw=\"").unwrap_or_else(|| {
                panic!("{page} at {width}px: missing data-sw on <html> after Chrome dump-dom")
            });
            let cw = attr_after(&dom, "data-cw=\"").unwrap_or_else(|| {
                panic!("{page} at {width}px: missing data-cw on <html> after Chrome dump-dom")
            });
            assert!(
                sw <= cw,
                "{page} at {width}px overflowed: scrollWidth={sw} clientWidth={cw}"
            );
        }
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn level_card_progress_aligns_on_desktop_if_chrome() {
        let Some(chrome) = find_chrome() else {
            return;
        };
        let out = build_live_site("chrome-levels");
        const PROBE: &str = r#"
(function () {
  var cards = Array.prototype.slice.call(document.querySelectorAll('.level-row .level-card'));
  var heights = [];
  var tops = [];
  var gaps = [];
  for (var i = 0; i < cards.length; i++) {
    var card = cards[i];
    var foot = card.querySelector('.level-foot') || card.querySelector('.bar');
    var desc = card.querySelector('h3 + p');
    var cr = card.getBoundingClientRect();
    heights.push(Math.round(cr.height));
    tops.push(foot ? Math.round(foot.getBoundingClientRect().top) : -1);
    if (desc && foot) {
      gaps.push(Math.round(foot.getBoundingClientRect().top - desc.getBoundingClientRect().bottom));
    } else {
      gaps.push(-1);
    }
  }
  var root = document.documentElement;
  root.setAttribute('data-level-n', String(cards.length));
  root.setAttribute('data-level-heights', heights.join(','));
  root.setAttribute('data-level-bar-tops', tops.join(','));
  root.setAttribute('data-level-gaps', gaps.join(','));
})();
"#;
        for (page, width, desktop) in [
            ("index.html", 1440u32, true),
            ("roadmap/index.html", 1440, true),
            ("index.html", 390, false),
            ("roadmap/index.html", 390, false),
        ] {
            let src = out.join(page);
            let slug = page.replace('/', "-");
            let wrapper = src.with_file_name(format!("levels-{width}-{slug}"));
            fs::write(
                &wrapper,
                inject_probe(&fs::read_to_string(&src).unwrap(), PROBE),
            )
            .unwrap();
            let uri = format!("file://{}", wrapper.display());
            let profile = out.join(format!("chrome-levels-{width}-{slug}"));
            fs::create_dir_all(&profile).unwrap();
            let dom = chrome_dump_dom(
                &chrome,
                &uri,
                width,
                &profile,
                std::time::Duration::from_secs(20),
            );
            let n = attr_after(&dom, "data-level-n=\"").unwrap_or_else(|| {
                panic!("{page} at {width}px: missing data-level-n after Chrome dump-dom")
            });
            assert_eq!(n, 5, "{page} at {width}px: expected 5 level cards, got {n}");
            let heights = attr_csv(&dom, "data-level-heights=\"");
            let tops = attr_csv(&dom, "data-level-bar-tops=\"");
            let gaps = attr_csv(&dom, "data-level-gaps=\"");
            assert_eq!(heights.len(), 5, "{page} at {width}px heights={heights:?}");
            assert_eq!(tops.len(), 5, "{page} at {width}px tops={tops:?}");
            if desktop {
                assert!(
                    spread(&heights) <= 1,
                    "{page} at {width}px: cards must share a height, got {heights:?}"
                );
                assert!(
                    spread(&tops) <= 1,
                    "{page} at {width}px: progress footers must share a top edge, got {tops:?}"
                );
            } else {
                assert!(
                    gaps.iter().all(|&g| (0..=24).contains(&g)),
                    "{page} at {width}px: stacked cards must keep natural flow (small copy-to-bar gap), got {gaps:?}"
                );
            }
        }
        let _ = fs::remove_dir_all(&out);
    }

    fn build_live_site(tag: &str) -> PathBuf {
        let site_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let real_root = site_root.parent().unwrap().to_path_buf();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let out = std::env::temp_dir().join(format!("megabase-site-{tag}-{stamp}"));
        let metrics = metrics::load(&real_root);
        let human = human_log::load(
            &fs::read_to_string(real_root.join("HUMAN_LOG.md")).unwrap_or_default(),
        );
        build(&site_root, &real_root, &out, &metrics, &human).expect("build");
        out
    }

    fn inject_probe(html: &str, script: &str) -> String {
        // Drop remote scripts so headless Chrome does not wait on analytics.
        let mut stripped = String::new();
        let mut rest = html;
        while let Some(start) = rest.find("<script") {
            stripped.push_str(&rest[..start]);
            if let Some(end) = rest[start..].find("</script>") {
                rest = &rest[start + end + 9..];
            } else {
                rest = "";
                break;
            }
        }
        stripped.push_str(rest);
        stripped.replace("</body>", &format!("<script>{script}</script></body>"))
    }

    fn attr_csv(html: &str, key: &str) -> Vec<i32> {
        attr_str(html, key)
            .unwrap_or_default()
            .split(',')
            .filter(|p| !p.is_empty())
            .map(|p| p.parse().expect("probe csv"))
            .collect()
    }

    fn attr_str(html: &str, key: &str) -> Option<String> {
        let start = html.find(key)? + key.len();
        let rest = html.get(start..)?;
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    }

    fn spread(vals: &[i32]) -> i32 {
        match (vals.iter().min(), vals.iter().max()) {
            (Some(lo), Some(hi)) => hi - lo,
            _ => 0,
        }
    }

    fn find_chrome() -> Option<PathBuf> {
        const CANDIDATES: &[&str] = &[
            "/opt/google/chrome/chrome",
            "/opt/google/chrome/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/google-chrome",
            "/usr/bin/chromium-browser",
            "/usr/bin/chromium",
            "google-chrome-stable",
            "google-chrome",
            "chromium-browser",
            "chromium",
        ];
        CANDIDATES.iter().find_map(|name| {
            let path = unwrap_chrome_launcher(&resolve_chrome(name)?);
            if injects_remote_debugging(&path) {
                return None;
            }
            Command::new(&path)
                .arg("--version")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|_| path)
        })
    }

    fn resolve_chrome(name: &str) -> Option<PathBuf> {
        let path = PathBuf::from(name);
        if name.contains('/') {
            return path.is_file().then_some(path);
        }
        let search = std::env::var_os("PATH")?;
        std::env::split_paths(&search)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    }

    fn unwrap_chrome_launcher(path: &Path) -> PathBuf {
        if !is_shell_script(path) {
            return path.to_path_buf();
        }
        if let Some(sibling) = path.parent().map(|dir| dir.join("chrome")) {
            if sibling.is_file() && !is_shell_script(&sibling) {
                return sibling;
            }
        }
        path.to_path_buf()
    }

    fn injects_remote_debugging(path: &Path) -> bool {
        is_shell_script(path)
            && fs::read_to_string(path)
                .map(|text| text.contains("remote-debugging-port"))
                .unwrap_or(false)
    }

    fn is_shell_script(path: &Path) -> bool {
        use std::io::Read;
        let Ok(mut file) = fs::File::open(path) else {
            return false;
        };
        let mut magic = [0u8; 2];
        file.read_exact(&mut magic).is_ok() && magic == *b"#!"
    }

    fn chrome_dump_dom(
        chrome: &Path,
        uri: &str,
        width: u32,
        profile: &Path,
        timeout: std::time::Duration,
    ) -> String {
        use std::io::Read;
        use std::process::Stdio;
        use std::time::Instant;

        let mut child = Command::new(chrome)
            .args([
                "--headless=new",
                "--disable-gpu",
                "--no-sandbox",
                "--disable-dev-shm-usage",
                "--disable-extensions",
                "--no-first-run",
                "--timeout=15000",
                "--virtual-time-budget=2000",
                &format!("--user-data-dir={}", profile.display()),
                &format!("--window-size={width},900"),
                "--dump-dom",
                uri,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("chrome");
        let stdout = child.stdout.take().expect("chrome stdout");
        let stderr = child.stderr.take().expect("chrome stderr");
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut r = stdout;
            let _ = r.read_to_end(&mut buf);
            buf
        });
        let err_reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut r = stderr;
            let _ = r.read_to_end(&mut buf);
            buf
        });
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if start.elapsed() >= timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "chrome timed out after {}s for {uri} at {width}px",
                        timeout.as_secs()
                    );
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
                Err(e) => panic!("wait chrome: {e}"),
            }
        };
        let dumped = reader.join().expect("chrome stdout thread");
        let err = err_reader.join().unwrap_or_default();
        assert!(
            status.success(),
            "chrome failed for {uri} at {width}px: {status}\n{}",
            String::from_utf8_lossy(&err)
        );
        String::from_utf8_lossy(&dumped).into_owned()
    }

    fn attr_after(html: &str, key: &str) -> Option<i32> {
        let start = html.find(key)? + key.len();
        let rest = html.get(start..)?;
        let end = rest.find('"')?;
        rest[..end].parse().ok()
    }

    #[test]
    fn article_metadata_escapes_quotes() {
        let out = generate_tmp_with(|root| {
            let dir = root.join("devlog");
            fs::create_dir_all(&dir).expect("devlog");
            fs::write(
                dir.join("2099-01-01.md"),
                "# Called it \"compatible\" today\n\nCalled it \"compatible\" today.\n",
            )
            .expect("devlog entry");
        });
        let html = fs::read_to_string(out.join("devlog/2099-01-01/index.html")).unwrap();
        assert!(html.contains("Called it &quot;compatible&quot; today"));
        assert!(html.contains("content=\"Called it &quot;compatible&quot; today.\""));
        assert!(!html.contains("content=\"Called it \"compatible\""));
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn generated_pages_omit_cost_copy() {
        let out = generate_tmp();
        assert!(!out.join("cost").exists());
        assert!(!out.join("cost/index.html").exists());
        let sitemap = fs::read_to_string(out.join("sitemap.xml")).unwrap();
        assert!(!sitemap.to_ascii_lowercase().contains("cost"));
        let manifesto = fs::read_to_string(out.join("manifesto/index.html")).unwrap();
        assert!(manifesto.contains("the loop and the logs, in real time"));
        assert!(!manifesto.contains("token spend"));
        assert!(!manifesto.contains("tokens and money spent"));
        let human_log = fs::read_to_string(out.join("human-log/index.html")).unwrap();
        assert!(
            human_log.contains("2026-10-10"),
            "the human log page records the owner decision"
        );
        assert!(human_log.contains("token spend"));
        let mut hits = Vec::new();
        collect_cost_hits(&out, &out, &mut hits);
        let _ = fs::remove_dir_all(&out);
        assert!(
            hits.is_empty(),
            "cost copy still on the site:\n{}",
            hits.join("\n")
        );
    }

    fn collect_cost_hits(root: &Path, dir: &Path, hits: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                collect_cost_hits(root, &path, hits);
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy();
            if !(name.ends_with(".html") || name == "sitemap.xml") {
                continue;
            }
            let rel = path.strip_prefix(root).unwrap_or(&path);
            // The intervention log quotes the owner decision. Other pages do not.
            if rel == Path::new("human-log/index.html") {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            let lower = text.to_ascii_lowercase();
            for needle in [
                "cost",
                "spend",
                "budget",
                "money",
                "dollar",
                "price",
                "token usage",
                "token spend",
            ] {
                let mut from = 0;
                while let Some(rel_idx) = find_term(&lower[from..], needle) {
                    let idx = from + rel_idx;
                    from = idx + needle.len();
                    // bcrypt's work factor is "cost 10", or "cost-10" next to
                    // hash/bcrypt. Other "cost-10" phrases are spend copy.
                    if needle == "cost" && is_bcrypt_work_factor(&text, idx) {
                        continue;
                    }
                    let end = (idx + 80).min(text.len());
                    hits.push(format!(
                        "{}: {needle:?} …{}…",
                        rel.display(),
                        &text[idx..end]
                    ));
                    break;
                }
            }
        }
    }

    /// True for bcrypt's cost factor, not for a price written as `cost-10`.
    fn is_bcrypt_work_factor(text: &str, idx: usize) -> bool {
        let rest = text[idx + "cost".len()..].trim_start();
        let hyphenated = rest.starts_with('-');
        let rest = rest.strip_prefix('-').unwrap_or(rest).trim_start();
        let bytes = rest.as_bytes();
        let ten = bytes.starts_with(b"10") && bytes.get(2).is_none_or(|b| !b.is_ascii_digit());
        if !ten {
            return false;
        }
        if !hyphenated {
            return true;
        }
        let after = rest
            .get(2..)
            .unwrap_or("")
            .trim_start()
            .to_ascii_lowercase();
        if after.starts_with("hash") || after.starts_with("bcrypt") {
            return true;
        }
        let start = idx.saturating_sub(1200);
        text[start..idx].to_ascii_lowercase().contains("bcrypt")
    }

    #[test]
    fn hyphenated_cost_without_bcrypt_context_is_spend_copy() {
        let spend = "the plan is cost-10 tokens per query";
        let idx = spend.find("cost").unwrap();
        assert!(!is_bcrypt_work_factor(spend, idx));
        let hash = "stores a new cost-10 hash";
        assert!(is_bcrypt_work_factor(hash, hash.find("cost").unwrap()));
        let spaced = "bcrypt hash (cost 10) and does not log the password";
        assert!(is_bcrypt_work_factor(spaced, spaced.find("cost").unwrap()));
    }

    fn find_term(haystack: &str, needle: &str) -> Option<usize> {
        let mut start = 0;
        while let Some(rel) = haystack[start..].find(needle) {
            let abs = start + rel;
            let before = haystack[..abs].chars().next_back();
            let after = haystack[abs + needle.len()..].chars().next();
            let word_before = before.is_none_or(|c| !c.is_ascii_alphanumeric());
            let word_after = after.is_none_or(|c| !c.is_ascii_alphanumeric());
            if word_before && word_after {
                return Some(abs);
            }
            start = abs + needle.len();
        }
        None
    }

    #[test]
    fn four_oh_four_uses_root_absolute_urls() {
        let out = generate_tmp();
        let html = fs::read_to_string(out.join("404.html")).unwrap();
        assert!(html.contains("href=\"/\""));
        assert!(html.contains("href=\"/components/\""));
        assert!(html.contains("MEGABASE_PAGE_NOT_FOUND"));
        let _ = fs::remove_dir_all(&out);
    }
}
