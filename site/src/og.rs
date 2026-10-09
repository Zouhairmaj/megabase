//! Open Graph cards (1200×630 PNG), rendered from an SVG template with resvg.
//!
//! Design source of truth: Kite file `megabase-identity`, page
//! "Website / OG images" (frames "OG / Home", "OG / Manifesto",
//! "OG / Page template"). Coordinates below reproduce that layout: 72 px side
//! padding, brand row at the top, a two-line title with a one-line summary in
//! a 776 px column, the 3×3 mark on the right, and the disclaimer row at the
//! bottom.
//!
//! Regenerate the committed PNGs with:
//!   cargo run --manifest-path site/Cargo.toml --release -- og
//!
//! To add a page: append a `Card` to `CARDS`, run the command above and point
//! the page's `og_image` in `main.rs` at `og/<key>.png`.

use std::fs;
use std::io;
use std::path::Path;

pub const DISCLAIMER: &str = "Not affiliated with or endorsed by Supabase, Inc.";

const BG: &str = "#0B0E12";
const ACCENT: &str = "#00D892";
const MUTED: &str = "#303235";
const TEXT: &str = "#F7F7F7";
const SUB: &str = "#BABABB";
const DIM: &str = "#8E9094";

const W: f64 = 1200.0;
const H: f64 = 630.0;
const PAD_X: f64 = 72.0;
/// JetBrains Mono metrics (units per em = 1000): ascent 1020, descent 300,
/// advance 600. `normal` line height is therefore 1.32 em.
const ASCENT: f64 = 1.02;
const LINE_NORMAL: f64 = 1.32;
const ADVANCE: f64 = 0.6;

pub struct Card {
    /// Output file stem: `static/og/<key>.png`.
    pub key: &'static str,
    pub eyebrow: &'static str,
    pub line1: &'static str,
    pub line2: &'static str,
    pub summary: &'static str,
}

pub const CARDS: &[Card] = &[
    Card {
        key: "home",
        eyebrow: "DAY 0 · BUILT BY AGENTS, IN PUBLIC",
        line1: "The Supabase API.",
        line2: "One Rust binary.",
        summary: "AI agents rewrite every Supabase service in Rust, judged response by response against the real stack.",
    },
    Card {
        key: "manifesto",
        eyebrow: "MANIFESTO",
        line1: "Supabase, in Rust.",
        line2: "By agents. In public.",
        summary: "What this is, the rules of the experiment, and how progress is measured.",
    },
    Card {
        key: "how-it-works",
        eyebrow: "HOW IT WORKS",
        line1: "Agents read the source.",
        line2: "A judge keeps score.",
        summary: "Pinned upstream, one unit at a time, compared with the real Supabase stack.",
    },
    Card {
        key: "status",
        eyebrow: "STATUS",
        line1: "Where the experiment",
        line2: "stands.",
        summary: "Units, coverage, conformance and the nested treemap, regenerated every commit.",
    },
    Card {
        key: "roadmap",
        eyebrow: "ROADMAP",
        line1: "Five levels,",
        line2: "gated in order.",
        summary: "REST and Auth, Storage, Realtime, Functions and Studio. No skipping.",
    },
    Card {
        key: "components",
        eyebrow: "COMPONENTS",
        line1: "Every service",
        line2: "Supabase wrote.",
        summary: "One Rust crate each. PostgreSQL stays the database you already run.",
    },
    Card {
        key: "devlog",
        eyebrow: "DEVLOG",
        line1: "What the agents did",
        line2: "today.",
        summary: "One short entry per day, written for humans, from files in devlog/.",
    },
    Card {
        key: "human-log",
        eyebrow: "HUMAN LOG",
        line1: "Every human touch",
        line2: "is public.",
        summary: "A fix, a nudge, a reverted commit. The count is part of the result.",
    },
    Card {
        key: "faq",
        eyebrow: "FAQ",
        line1: "Questions",
        line2: "people ask.",
        summary: "Affiliation, Day 0, the judge, units, coverage versus conformance, cost.",
    },
    Card {
        key: "docs",
        eyebrow: "DOCS",
        line1: "Getting started.",
        line2: "Nothing works yet.",
        summary: "Build from source. Every endpoint answers 501 MEGABASE_NOT_IMPLEMENTED.",
    },
    Card {
        key: "cost",
        eyebrow: "COST",
        line1: "Tokens and money.",
        line2: "In public.",
        summary: "Tracking starts after Phase 0 review. Until then this card invents no figure.",
    },
];

/// Baseline of a text line whose line box starts at `top`.
fn baseline(top: f64, size: f64, line_height: f64) -> f64 {
    top + (line_height - LINE_NORMAL * size) / 2.0 + ASCENT * size
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Greedy word wrap for a monospace font.
fn wrap(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let next = if line.is_empty() {
            word.chars().count()
        } else {
            line.chars().count() + 1 + word.chars().count()
        };
        if next > max_chars && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn grid(x: f64, y: f64, square: f64, gap: f64) -> String {
    let mut out = String::new();
    for i in 0..9 {
        let fill = if i == 4 { ACCENT } else { MUTED };
        let cx = x + f64::from(i % 3) * (square + gap);
        let cy = y + f64::from(i / 3) * (square + gap);
        out.push_str(&format!(
            r#"<rect x="{cx}" y="{cy}" width="{square}" height="{square}" fill="{fill}"/>"#
        ));
    }
    out
}

fn text(x: f64, y: f64, size: f64, fill: &str, weight: u16, extra: &str, body: &str) -> String {
    format!(
        r#"<text x="{x}" y="{y:.2}" font-family="JetBrains Mono" font-size="{size}" font-weight="{weight}" fill="{fill}"{extra}>{}</text>"#,
        esc(body)
    )
}

/// The card as SVG. Layout mirrors the Kite frame (flex column, space-between).
pub fn svg(card: &Card) -> String {
    let mut s = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}"><rect width="{W}" height="{H}" fill="{BG}"/>"#
    );

    // Brand row: 64 px from the top, 37 px tall (28 px wordmark).
    let top = 64.0;
    let row_h = LINE_NORMAL * 28.0;
    s.push_str(&grid(PAD_X, top + (row_h - 36.0) / 2.0, 10.0, 3.0));
    s.push_str(&text(
        PAD_X + 54.0,
        baseline(top, 28.0, row_h),
        28.0,
        TEXT,
        700,
        r#" letter-spacing="5""#,
        "MEGABASE",
    ));
    let url_h = LINE_NORMAL * 22.0;
    s.push_str(&text(
        W - PAD_X,
        baseline(top + (row_h - url_h) / 2.0, 22.0, url_h),
        22.0,
        DIM,
        400,
        r#" text-anchor="end""#,
        "megabase.sh",
    ));

    // Footer row: bottom padding 56, 18 px text, 22 px above it, 1 px rule.
    let foot_h = 23.0;
    let foot_top = H - 56.0 - foot_h;
    let rule_y = foot_top - 23.0;
    s.push_str(&format!(
        r#"<rect x="{PAD_X}" y="{rule_y}" width="{}" height="1" fill="{MUTED}"/>"#,
        W - 2.0 * PAD_X
    ));
    let foot_base = baseline(foot_top, 18.0, foot_h);
    s.push_str(&text(
        PAD_X,
        foot_base,
        18.0,
        SUB,
        400,
        "",
        &format!("Unofficial experiment. {DISCLAIMER}"),
    ));
    s.push_str(&text(
        W - PAD_X,
        foot_base,
        18.0,
        DIM,
        400,
        r#" text-anchor="end""#,
        "Apache-2.0",
    ));

    // Middle: text column (776 px) and the 232 px mark, centred between rows.
    let column = 776.0;
    let sub_size = 23.0;
    let sub_lh = sub_size * 1.45;
    let summary = wrap(card.summary, (column / (sub_size * ADVANCE)) as usize);
    let title_lh = 56.0 * 1.15;
    let eyebrow_h = 26.0;
    let col_h = eyebrow_h + 22.0 + (2.0 * title_lh + 4.0) + 22.0 + sub_lh * summary.len() as f64;
    let mark = 232.0;
    let mid_h = col_h.max(mark);
    let region_top = top + row_h;
    let mid_top = region_top + (rule_y - region_top - mid_h) / 2.0;
    let col_top = mid_top + (mid_h - col_h) / 2.0;

    s.push_str(&grid(
        W - PAD_X - mark,
        mid_top + (mid_h - mark) / 2.0,
        64.0,
        20.0,
    ));
    s.push_str(&text(
        PAD_X,
        baseline(col_top, 20.0, eyebrow_h),
        20.0,
        ACCENT,
        400,
        r#" letter-spacing="4""#,
        card.eyebrow,
    ));
    let t1 = col_top + eyebrow_h + 22.0;
    s.push_str(&text(
        PAD_X,
        baseline(t1, 56.0, title_lh),
        56.0,
        TEXT,
        700,
        "",
        card.line1,
    ));
    let t2 = t1 + title_lh + 4.0;
    s.push_str(&text(
        PAD_X,
        baseline(t2, 56.0, title_lh),
        56.0,
        ACCENT,
        700,
        "",
        card.line2,
    ));
    let mut y = t2 + title_lh + 22.0;
    for line in &summary {
        s.push_str(&text(
            PAD_X,
            baseline(y, sub_size, sub_lh),
            sub_size,
            SUB,
            400,
            "",
            line,
        ));
        y += sub_lh;
    }

    s.push_str("</svg>");
    s
}

/// Render every card into `<site_root>/static/og/` and copy the home card to
/// `static/og-card.png` (the default card, used by the 404 page).
pub fn generate(site_root: &Path) -> io::Result<()> {
    let mut opt = resvg::usvg::Options::default();
    {
        let db = opt.fontdb_mut();
        for weight in ["Regular", "Bold"] {
            let path = site_root.join(format!("og-fonts/JetBrainsMono-{weight}.ttf"));
            db.load_font_data(fs::read(path)?);
        }
    }
    let out_dir = site_root.join("static/og");
    fs::create_dir_all(&out_dir)?;
    for card in CARDS {
        let tree = resvg::usvg::Tree::from_str(&svg(card), &opt)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(W as u32, H as u32)
            .ok_or_else(|| io::Error::other("pixmap allocation failed"))?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::default(),
            &mut pixmap.as_mut(),
        );
        let png = pixmap
            .encode_png()
            .map_err(|e| io::Error::other(e.to_string()))?;
        let dest = out_dir.join(format!("{}.png", card.key));
        fs::write(&dest, png)?;
        eprintln!("wrote {}", dest.display());
    }
    fs::copy(
        out_dir.join("home.png"),
        site_root.join("static/og-card.png"),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_like_the_browser() {
        let lines = wrap(CARDS[0].summary, 56);
        assert_eq!(
            lines,
            vec![
                "AI agents rewrite every Supabase service in Rust, judged",
                "response by response against the real stack."
            ]
        );
    }

    #[test]
    fn svg_parses() {
        for card in CARDS {
            let svg = svg(card);
            assert!(resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).is_ok());
            assert!(svg.contains("endorsed by Supabase, Inc."));
        }
    }
}
