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

use chrome::Paths;
use html::subst;
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
    Cost,
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
        title: "Docs — Getting started — Megabase",
        description: "Build Megabase from source, configure it, and run the judge. Day 0: every endpoint returns 501.",
        og_type: "website",
        og_image: "og/docs.png",
        og_alt: "Megabase docs: getting started. Day 0, nothing works yet.",
        kind: Kind::Docs,
        noindex: false,
        extra_preload: ExtraPreload::Inter,
    },
    Page {
        id: "cost",
        dir: "cost",
        title: "Cost — Megabase",
        description: "Tokens and money spent on the Megabase experiment, published continuously once tracking starts.",
        og_type: "website",
        og_image: "og/cost.png",
        og_alt: "Megabase cost: tokens and money, published continuously.",
        kind: Kind::Cost,
        noindex: false,
        extra_preload: ExtraPreload::Home,
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

struct SiteData<'a> {
    metrics: &'a Metrics,
    human: &'a human_log::HumanLog,
    entries: &'a [devlog::Entry],
    roadmap_md: Option<&'a str>,
    coverage_svg: bool,
}

fn build(
    site_root: &Path,
    repo_root: &Path,
    out: &Path,
    metrics: &Metrics,
    human: &human_log::HumanLog,
) -> io::Result<()> {
    if out.exists() && !same_path(out, site_root) {
        fs::remove_dir_all(out)?;
    }
    copy_dir(&site_root.join("static"), out)?;
    let coverage_svg = copy_coverage_svgs(repo_root, out)?;

    let layout = fs::read_to_string(site_root.join("templates/layout.html"))?;
    let logo = fs::read_to_string(site_root.join("static/logo.svg"))?
        .trim()
        .to_string();
    let entries = devlog::load(repo_root)?;
    let roadmap_md = fs::read_to_string(repo_root.join("docs/ROADMAP.md")).ok();
    let data = SiteData {
        metrics,
        human,
        entries: &entries,
        roadmap_md: roadmap_md.as_deref(),
        coverage_svg,
    };

    for page in PAGES {
        let html = render_page(page, &layout, &logo, repo_root, &data, None)?;
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
        let article = render_article(entry, &layout, &logo)?;
        let dir = out.join("devlog").join(&entry.slug);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("index.html"), article)?;
    }

    fs::write(out.join("sitemap.xml"), sitemap(&entries))?;
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
    wrap(page, layout, logo, &paths, loc, |paths| {
        content(page, paths, repo_root, data)
    })
}

fn render_article(entry: &devlog::Entry, layout: &str, logo: &str) -> io::Result<String> {
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
    wrap(&page, layout, logo, &paths, Some(&loc), |paths| {
        Ok(pages::devlog_article(paths, entry))
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
        Kind::Status => pages::status(paths, data.metrics, data.coverage_svg),
        Kind::Roadmap => pages::roadmap(paths, data.metrics, data.roadmap_md),
        Kind::Components => pages::components(paths, data.metrics),
        Kind::Devlog => pages::devlog_index(paths, data.entries),
        Kind::HumanLog => pages::human_log(paths, data.metrics, &data.human.html),
        Kind::Faq => pages::faq(paths, data.metrics),
        Kind::Docs => pages::docs(paths),
        Kind::Cost => pages::cost(paths, data.metrics),
        Kind::NotFound => pages::not_found(paths),
    })
}

fn wrap(
    page: &Page,
    layout: &str,
    logo: &str,
    paths: &Paths,
    loc: Option<&str>,
    body: impl FnOnce(&Paths) -> io::Result<String>,
) -> io::Result<String> {
    let is_404 = page.id == "404";
    let asset = paths.asset();
    let canonical = loc.map(str::to_string).unwrap_or_else(|| page_url(page));
    let mut vars = BTreeMap::new();
    vars.insert("title".into(), page.title.into());
    vars.insert("description".into(), page.description.into());
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
    vars.insert(
        "robots".into(),
        if page.noindex {
            r#"<meta name="robots" content="noindex" />"#.into()
        } else {
            String::new()
        },
    );
    vars.insert("preload".into(), preload(page, asset));
    let body_class = match page.id {
        "manifesto" => "manifesto-page",
        "docs" => "docs-page",
        _ => "",
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
    vars.insert("header".into(), chrome::header(paths, logo));
    vars.insert("footer".into(), chrome::footer(paths, logo));
    let needs_copy = matches!(page.kind, Kind::Docs | Kind::HowItWorks);
    let mut scripts = String::new();
    if page.id == "manifesto" {
        scripts.push_str(&format!(r#"<script src="{asset}toc.js" defer></script>"#));
    }
    if needs_copy {
        scripts.push_str(&format!(r#"<script src="{asset}copy.js" defer></script>"#));
    }
    vars.insert("scripts".into(), scripts);
    vars.insert("content".into(), body(paths)?);
    Ok(subst(layout, &vars))
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

fn sitemap(entries: &[devlog::Entry]) -> String {
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

fn copy_coverage_svgs(repo_root: &Path, out: &Path) -> io::Result<bool> {
    let dark = repo_root.join("coverage/treemap.svg");
    if !dark.is_file() {
        return Ok(false);
    }
    let dest = out.join("coverage");
    fs::create_dir_all(&dest)?;
    fs::copy(&dark, dest.join("treemap.svg"))?;
    let light = repo_root.join("coverage/treemap-light.svg");
    if light.is_file() {
        fs::copy(&light, dest.join("treemap-light.svg"))?;
    }
    Ok(true)
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
        let roadmap = real_root.join("docs/ROADMAP.md");
        if roadmap.is_file() {
            fs::create_dir_all(tmp_root.join("docs")).expect("docs");
            fs::copy(&roadmap, tmp_root.join("docs/ROADMAP.md")).expect("roadmap");
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
        assert!(!home.contains("1024"));
        assert!(home.contains("Not affiliated with or endorsed by Supabase, Inc."));
        assert!(!home.to_ascii_lowercase().contains("oxide"));
        assert!(home.contains("EXPERIMENT STATUS"));
        assert!(home.contains("updated on every commit"));
        assert!(home.contains("panel-treemap"));
        assert!(home.contains("visually-hidden"));
        assert!(home.contains("Coverage · Conformance"));
        assert!(home.contains("Units passing the judge"));
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
            "cost/index.html",
            "404.html",
            "sitemap.xml",
        ] {
            assert!(out.join(rel).is_file(), "missing {rel}");
        }
        let sitemap = fs::read_to_string(out.join("sitemap.xml")).unwrap();
        assert!(sitemap.contains("https://megabase.sh/status/"));
        assert!(!sitemap.contains("/404"));
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn status_uses_generated_treemap_until_coverage_svgs_exist() {
        let out = generate_tmp();
        let html = fs::read_to_string(out.join("status/index.html")).unwrap();
        assert!(html.contains("data-component=\"rest\""));
        assert!(!html.contains("coverage/treemap.svg"));
        assert!(!out.join("coverage/treemap.svg").exists());
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn status_embeds_coverage_svgs_when_present() {
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
        assert!(html.contains("coverage/treemap.svg"));
        assert!(html.contains("coverage/treemap-light.svg"));
        assert!(out.join("coverage/treemap.svg").is_file());
        assert!(!html.contains("data-component=\"rest\""));
        let _ = fs::remove_dir_all(&out);
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
