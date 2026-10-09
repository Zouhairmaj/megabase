//! README banners (Kite page "Repo / README"): 1280×320, JetBrains Mono
//! outlines. Logo, wordmark, tagline and unofficial subline only.

use std::fmt::Write;

use crate::glyphs::{self, BOLD, REGULAR};

const W: f32 = 1280.0;
const H: f32 = 320.0;

const ARIA: &str =
    "Megabase — Supabase-compatible API. One Rust binary. An experiment built by AI agents · unofficial";

#[derive(Clone, Copy)]
struct Palette {
    bg: &'static str,
    tile: &'static str,
    center: &'static str,
    word: &'static str,
    body: &'static str,
    muted: &'static str,
    accent: &'static str,
    bar: &'static str,
}

const DARK: Palette = Palette {
    bg: "#0B0E12",
    tile: "#303235",
    center: "#00D892",
    word: "#F7F7F7",
    body: "#BABABB",
    muted: "#5D5E61",
    accent: "#00D892",
    bar: "#00D892",
};

const LIGHT: Palette = Palette {
    bg: "#F7F7F7",
    tile: "#181A1D",
    center: "#009366",
    word: "#181A1D",
    body: "#303235",
    muted: "#5D5E61",
    accent: "#00D892",
    bar: "#00D892",
};

fn logo(svg: &mut String, x: f32, y: f32, size: f32, pal: Palette) {
    let gap = size * 0.08;
    let tile = (size - 2.0 * gap) / 3.0;
    let step = tile + gap;
    for row in 0..3 {
        for col in 0..3 {
            let fill = if row == 1 && col == 1 {
                pal.center
            } else {
                pal.tile
            };
            let _ = write!(
                svg,
                r#"<rect x="{:.2}" y="{:.2}" width="{tile:.2}" height="{tile:.2}" fill="{fill}"/>"#,
                x + col as f32 * step,
                y + row as f32 * step
            );
        }
    }
}

pub fn render(dark: bool) -> String {
    let pal = if dark { DARK } else { LIGHT };
    let mut svg = String::new();
    let _ = write!(
        svg,
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}" role="img" "#,
            r#"aria-label="{aria}">"#
        ),
        w = W as u32,
        h = H as u32,
        aria = ARIA
    );
    let _ = write!(svg, r#"<title>{ARIA}</title>"#);
    let _ = write!(svg, r#"<rect width="{W}" height="{H}" fill="{}"/>"#, pal.bg);
    let _ = write!(
        svg,
        r#"<rect x="0" y="316" width="{W}" height="4" fill="{}"/>"#,
        pal.bar
    );

    let logo_size = 76.0;
    let logo_x = 72.0;
    let logo_y = 52.0;
    logo(&mut svg, logo_x, logo_y, logo_size, pal);

    let word_size = 54.0;
    let word_x = logo_x + logo_size + 26.0;
    let word_y = logo_y + (logo_size - word_size) / 2.0;
    glyphs::write_text(
        &mut svg, BOLD, "MEGABASE", word_x, word_y, word_size, pal.word, 0.16,
    );

    let tag_size = 22.0;
    let tag_x = logo_x;
    let tag_y = 164.0;
    let left = "Supabase-compatible API. ";
    glyphs::write_text(
        &mut svg, REGULAR, left, tag_x, tag_y, tag_size, pal.body, 0.0,
    );
    let rust_x = tag_x + glyphs::measure(REGULAR, left, tag_size, 0.0);
    glyphs::write_text(
        &mut svg,
        REGULAR,
        "One Rust binary.",
        rust_x,
        tag_y,
        tag_size,
        pal.accent,
        0.0,
    );

    glyphs::write_text(
        &mut svg,
        REGULAR,
        "An experiment built by AI agents · unofficial",
        tag_x,
        206.0,
        15.0,
        pal.muted,
        0.02,
    );

    svg.push_str("</svg>\n");
    svg
}

pub fn pair() -> (String, String) {
    (render(true), render(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_is_logo_wordmark_tagline_only() {
        let svg = render(true);
        assert!(svg.contains("viewBox=\"0 0 1280 320\""));
        assert_eq!(svg.matches("<rect").count(), 1 + 1 + 9); // bg, bar, logo tiles
        assert!(svg.contains("aria-label=\"Megabase — Supabase-compatible API. One Rust binary. An experiment built by AI agents · unofficial\""));
        assert!(svg.contains("unofficial"));
        assert!(svg.contains("Supabase-compatible"));
        assert!(!svg.to_ascii_lowercase().contains("official supabase"));
        assert!(!svg.contains("DAY "));
        assert!(!svg.contains("UNITS"));
        assert!(!svg.contains("1,024"));
        assert!(svg.contains("#00D892"));
        assert!(svg.contains("#0B0E12"));
    }

    #[test]
    fn light_banner_uses_light_palette() {
        let svg = render(false);
        assert!(svg.contains("#F7F7F7"));
        assert!(svg.contains("#009366"));
        assert_eq!(svg.matches("<rect").count(), 11);
    }

    #[test]
    fn pair_returns_dark_then_light() {
        let (dark, light) = pair();
        assert!(dark.contains("#0B0E12"));
        assert!(light.contains("fill=\"#F7F7F7\""));
    }
}
