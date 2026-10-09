//! Squarified treemaps: one whole square per unit, grouped into a labeled
//! block per component. Squares are a fixed size in every block and are
//! never clipped.

use std::fmt::Write;

use crate::glyphs::{self, BOLD, REGULAR};
use crate::model::{component_label, Unit, COMPONENTS};
use crate::status::{pct, State, Status};

/// Side of each unit square (Kite Status treemap).
pub const CELL: f64 = 26.0;
/// Gap between adjacent squares.
pub const GAP: f64 = 4.0;
const STRIDE: f64 = CELL + GAP;
const HEADER_H: f64 = 28.0;
const INNER_PAD: f64 = 12.0;
const BLOCK_GAP: f64 = 8.0;
const MARGIN: f64 = 16.0;
const BADGE_H: f64 = 20.0;
const TITLE_H: f64 = 36.0;
const LEGEND_H: f64 = 32.0;

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn inset(self, top: f64, side: f64) -> Rect {
        Rect {
            x: self.x + side,
            y: self.y + top,
            w: (self.w - 2.0 * side).max(0.0),
            h: (self.h - top - side).max(0.0),
        }
    }

    fn contains(self, inner: Rect) -> bool {
        inner.x + 1e-6 >= self.x
            && inner.y + 1e-6 >= self.y
            && inner.x + inner.w <= self.x + self.w + 1e-6
            && inner.y + inner.h <= self.y + self.h + 1e-6
    }
}

/// Squarified layout (Bruls, Huizing, van Wijk). Returns one rectangle per
/// value, in input order.
pub fn squarify(values: &[f64], rect: Rect) -> Vec<Rect> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|a, b| values[*b].total_cmp(&values[*a]).then(a.cmp(b)));
    let total: f64 = values.iter().sum();
    let mut out = vec![
        Rect {
            x: rect.x,
            y: rect.y,
            w: 0.0,
            h: 0.0
        };
        values.len()
    ];
    if total <= 0.0 {
        return out;
    }
    let scale = rect.w * rect.h / total;
    let areas: Vec<f64> = order.iter().map(|i| values[*i] * scale).collect();
    let worst = |row: &[f64], side: f64| {
        let sum: f64 = row.iter().sum();
        let max = row.iter().cloned().fold(f64::MIN, f64::max);
        let min = row.iter().cloned().fold(f64::MAX, f64::min);
        f64::max(
            side * side * max / (sum * sum),
            sum * sum / (side * side * min),
        )
    };
    let mut free = rect;
    let mut i = 0;
    while i < areas.len() {
        let side = free.w.min(free.h);
        let mut j = i + 1;
        while j < areas.len() && worst(&areas[i..=j], side) <= worst(&areas[i..j], side) {
            j += 1;
        }
        let row_area: f64 = areas[i..j].iter().sum();
        if free.w >= free.h {
            let w = if free.h > 0.0 { row_area / free.h } else { 0.0 };
            let mut y = free.y;
            for k in i..j {
                let h = if w > 0.0 { areas[k] / w } else { 0.0 };
                out[order[k]] = Rect { x: free.x, y, w, h };
                y += h;
            }
            free.x += w;
            free.w -= w;
        } else {
            let h = if free.w > 0.0 { row_area / free.w } else { 0.0 };
            let mut x = free.x;
            for k in i..j {
                let w = if h > 0.0 { areas[k] / h } else { 0.0 };
                out[order[k]] = Rect { x, y: free.y, w, h };
                x += w;
            }
            free.y += h;
            free.h -= h;
        }
        i = j;
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    /// Four colors: not started, implemented, tested, conformant.
    State,
    /// Green when implemented.
    Coverage,
    /// Green when conformant.
    Conformance,
}

#[derive(Clone)]
pub enum Heading {
    /// README / website Status chrome (badges + components header).
    Status,
    /// A single title bar, used for per-metric / per-component maps.
    Title(String),
}

// Megabase palette only (docs/brand/README.md).
const BG: &str = "#0B0E12";
const PANEL: &str = "#181A1D";
const TEXT: &str = "#F7F7F7";
const MUTED: &str = "#BABABB";
const NOT_STARTED: &str = "#303235";
const IMPLEMENTED: &str = "#005441";
const TESTED: &str = "#009366";
const GREEN: &str = "#00D892";

fn color(metric: Metric, state: State) -> &'static str {
    match (metric, state) {
        (Metric::State, State::Missing) => NOT_STARTED,
        (Metric::State, State::Implemented) => IMPLEMENTED,
        (Metric::State, State::Tested) => TESTED,
        (Metric::State, State::Conformant) => GREEN,
        (Metric::Coverage, s) if s >= State::Implemented => GREEN,
        (Metric::Conformance, State::Conformant) => GREEN,
        _ => NOT_STARTED,
    }
}

fn counted(metric: Metric, state: State) -> bool {
    match metric {
        Metric::Coverage => state >= State::Implemented,
        _ => state == State::Conformant,
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn cols_for(inner_w: f64, n: usize) -> usize {
    if n == 0 || inner_w < CELL {
        return 1;
    }
    let max = ((inner_w + GAP) / STRIDE).floor() as usize;
    max.max(1).min(n)
}

fn grid_size(n: usize, cols: usize) -> (f64, f64) {
    if n == 0 {
        return (0.0, 0.0);
    }
    let cols = cols.max(1);
    let rows = n.div_ceil(cols);
    let w = cols as f64 * CELL + (cols - 1) as f64 * GAP;
    let h = rows as f64 * CELL + (rows - 1) as f64 * GAP;
    (w, h)
}

/// One 26×26 square per unit, row-major, the whole grid centered in `inner`.
/// Returns an empty vec when `inner` cannot hold whole squares (never clips).
pub fn unit_cells(n: usize, inner: Rect) -> Vec<Rect> {
    if n == 0 || inner.w < CELL || inner.h < CELL {
        return Vec::new();
    }
    let cols = cols_for(inner.w, n);
    let (gw, gh) = grid_size(n, cols);
    if gw > inner.w + 1e-6 || gh > inner.h + 1e-6 {
        return Vec::new();
    }
    let ox = inner.x + (inner.w - gw) / 2.0;
    let oy = inner.y + (inner.h - gh) / 2.0;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let c = i % cols;
        let r = i / cols;
        out.push(Rect {
            x: ox + c as f64 * STRIDE,
            y: oy + r as f64 * STRIDE,
            w: CELL,
            h: CELL,
        });
    }
    out
}

fn block_frame(rect: Rect) -> Rect {
    rect.inset(BLOCK_GAP / 2.0, BLOCK_GAP / 2.0)
}

fn block_inner(frame: Rect) -> Rect {
    Rect {
        x: frame.x + INNER_PAD,
        y: frame.y + HEADER_H + INNER_PAD,
        w: (frame.w - 2.0 * INNER_PAD).max(0.0),
        h: (frame.h - HEADER_H - 2.0 * INNER_PAD).max(0.0),
    }
}

/// Squarify weights: unit counts, with a floor so the smallest block is still
/// wide enough for whole 26px squares and a `NAME 0/N` label.
fn layout_weights(counts: &[usize], canvas: f64) -> Vec<f64> {
    let total: f64 = counts.iter().map(|&n| n as f64).sum::<f64>().max(1.0);
    let min_w = CELL * 2.0 + GAP + INNER_PAD * 2.0 + BLOCK_GAP + 24.0;
    let min_h = HEADER_H + CELL * 2.0 + GAP + INNER_PAD * 2.0 + BLOCK_GAP;
    let min_share = (min_w * min_h) / canvas.max(1.0) * total;
    counts.iter().map(|&n| (n as f64).max(min_share)).collect()
}

fn block_fits(n: usize, rect: Rect) -> bool {
    let frame = block_frame(rect);
    if frame.w < CELL + 2.0 * INNER_PAD || frame.h < HEADER_H + CELL + 2.0 * INNER_PAD {
        return false;
    }
    let inner = block_inner(frame);
    unit_cells(n, inner).len() == n
}

/// Name + stats for a component header. Wide blocks get
/// `NAME 0.0% (0/N)`; narrow blocks get `NAME 0/N`.
pub fn component_header(
    name: &str,
    done: usize,
    n: usize,
    percent: f64,
    max_w: f32,
) -> (String, String, f32) {
    let size = 12.0_f32;
    let min = 8.0_f32;
    let wide_stats = format!(" {percent:.1}% ({done}/{n})");
    let narrow_stats = format!(" {done}/{n}");
    let fits = |nm: &str, stats: &str, sz: f32| {
        glyphs::measure(BOLD, nm, sz, 0.0) + glyphs::measure(REGULAR, stats, sz, 0.0) <= max_w
    };
    if fits(name, &wide_stats, size) {
        return (name.to_string(), wide_stats, size);
    }
    if fits(name, &narrow_stats, size) {
        return (name.to_string(), narrow_stats, size);
    }
    let mut sz = 11.0;
    while sz >= min {
        if fits(name, &narrow_stats, sz) {
            return (name.to_string(), narrow_stats, sz);
        }
        sz -= 0.5;
    }
    let stats_w = glyphs::measure(REGULAR, &narrow_stats, min, 0.0);
    let budget = (max_w - stats_w).max(0.0);
    let nm = glyphs::truncate(BOLD, name, min, 0.0, budget);
    (nm, narrow_stats, min)
}

fn draw_badge(svg: &mut String, x: f32, y: f32, label: &str, value: &str, value_bg: &str) -> f32 {
    let size = 11.0;
    let pad = 8.0;
    let h = BADGE_H as f32;
    let lw = (glyphs::measure(REGULAR, label, size, 0.0) + pad * 2.0).ceil();
    let vw = (glyphs::measure(REGULAR, value, size, 0.0) + pad * 2.0).ceil();
    let w = lw + vw;
    let value_fg = if value_bg == GREEN { BG } else { TEXT };
    let _ = write!(
        svg,
        r##"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="3" fill="{PANEL}"/>"##
    );
    let _ = write!(
        svg,
        r##"<rect x="{:.1}" y="{y:.1}" width="{vw:.1}" height="{h:.1}" rx="3" fill="{value_bg}"/>"##,
        x + lw
    );
    let _ = write!(
        svg,
        r##"<rect x="{:.1}" y="{y:.1}" width="4" height="{h:.1}" fill="{value_bg}"/>"##,
        x + lw
    );
    let ty = y + (h - size) / 2.0;
    glyphs::write_text(svg, REGULAR, label, x + pad, ty, size, MUTED, 0.0);
    glyphs::write_text(svg, REGULAR, value, x + lw + pad, ty, size, value_fg, 0.0);
    w
}

fn metric_badge_color(percent: f64) -> &'static str {
    if percent >= 90.0 {
        GREEN
    } else if percent >= 50.0 {
        TESTED
    } else {
        IMPLEMENTED
    }
}

pub fn render(
    heading: Heading,
    units: &[&Unit],
    status: &Status,
    metric: Metric,
    width: f64,
) -> String {
    let mut components: Vec<(&str, Vec<&Unit>)> = Vec::new();
    for (id, _) in COMPONENTS {
        let mut members: Vec<&Unit> = units
            .iter()
            .copied()
            .filter(|u| u.component == *id)
            .collect();
        if members.is_empty() {
            continue;
        }
        members.sort_by(|a, b| a.group.cmp(&b.group).then(a.id.cmp(&b.id)));
        components.push((id, members));
    }

    let implemented = units
        .iter()
        .filter(|u| status.state(&u.id) >= State::Implemented)
        .count();
    let conformant = units
        .iter()
        .filter(|u| status.state(&u.id) == State::Conformant)
        .count();
    let metric_done = units
        .iter()
        .filter(|u| counted(metric, status.state(&u.id)))
        .count();
    let total = units.len();
    let coverage_pct = pct(implemented, total);
    let conformant_pct = pct(conformant, total);
    let metric_pct = pct(metric_done, total);

    let chrome_h = match heading {
        Heading::Status => MARGIN + BADGE_H + 12.0 + TITLE_H + 8.0,
        Heading::Title(_) => MARGIN + TITLE_H + 8.0,
    };
    let counts: Vec<usize> = components.iter().map(|(_, m)| m.len()).collect();
    let body_x = MARGIN;
    let body_w = (width - 2.0 * MARGIN).max(CELL * 4.0);
    let body_y = chrome_h;
    let area_guess: f64 = counts
        .iter()
        .map(|&n| {
            let n = n.max(1);
            let cols = (n as f64).sqrt().ceil().max(1.0) as usize;
            let (gw, gh) = grid_size(n, cols);
            (gw + 2.0 * INNER_PAD + BLOCK_GAP) * (gh + HEADER_H + 2.0 * INNER_PAD + BLOCK_GAP)
        })
        .sum();
    let mut body_h = (area_guess / body_w * 1.2).max(360.0);
    let rects = loop {
        let body = Rect {
            x: body_x,
            y: body_y,
            w: body_w,
            h: body_h,
        };
        let sizes = layout_weights(&counts, body_w * body_h);
        let rects = squarify(&sizes, body);
        let ok = components
            .iter()
            .zip(&rects)
            .all(|((_, members), r)| block_fits(members.len(), *r));
        if ok || body_h > 8000.0 {
            break rects;
        }
        body_h += 32.0;
    };
    let height = body_y + body_h + LEGEND_H + MARGIN;

    let aria = match &heading {
        Heading::Status => {
            format!("Supabase components: {conformant_pct:.1}% conformant ({conformant}/{total})")
        }
        Heading::Title(t) => format!("{t} — {metric_pct:.1}% ({metric_done}/{total})"),
    };

    let mut svg = String::new();
    let _ = write!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height:.0}" viewBox="0 0 {width} {height:.0}" role="img" aria-label="{}">"#,
        esc(&aria)
    );
    let _ = write!(svg, r#"<title>{}</title>"#, esc(&aria));
    let _ = write!(svg, r#"<rect width="100%" height="100%" fill="{BG}"/>"#);

    let mut cursor_x = MARGIN as f32;
    let badge_y = MARGIN as f32;
    if matches!(heading, Heading::Status) {
        cursor_x += draw_badge(
            &mut svg,
            cursor_x,
            badge_y,
            "coverage",
            &format!("{coverage_pct:.1}%"),
            metric_badge_color(coverage_pct),
        ) + 8.0;
        cursor_x += draw_badge(
            &mut svg,
            cursor_x,
            badge_y,
            "conformance",
            &format!("{conformant_pct:.1}%"),
            metric_badge_color(conformant_pct),
        ) + 8.0;
        draw_badge(
            &mut svg,
            cursor_x,
            badge_y,
            "units",
            &format!("{implemented} / {total}"),
            NOT_STARTED,
        );
    }

    let title_y = match heading {
        Heading::Status => MARGIN + BADGE_H + 12.0,
        Heading::Title(_) => MARGIN,
    };
    let title_bar = Rect {
        x: MARGIN,
        y: title_y,
        w: body_w,
        h: TITLE_H,
    };
    let _ = write!(
        svg,
        r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{PANEL}"/>"#,
        title_bar.x, title_bar.y, title_bar.w, title_bar.h
    );
    let title_size = 14.0_f32;
    let title_text = match &heading {
        Heading::Status => {
            format!("Supabase components: {conformant_pct:.1}% conformant ({conformant}/{total})")
        }
        Heading::Title(t) => format!("{t}: {metric_pct:.1}% ({metric_done}/{total})"),
    };
    let title_max = title_bar.w as f32 - 24.0;
    let title_draw = glyphs::truncate(REGULAR, &title_text, title_size, 0.0, title_max);
    glyphs::write_text(
        &mut svg,
        REGULAR,
        &title_draw,
        title_bar.x as f32 + 12.0,
        title_bar.y as f32 + (TITLE_H as f32 - title_size) / 2.0,
        title_size,
        TEXT,
        0.0,
    );

    for ((component, members), rect) in components.iter().zip(&rects) {
        let frame = block_frame(*rect);
        let inner = block_inner(frame);
        let name = component_label(component).to_ascii_uppercase();
        let done = members
            .iter()
            .filter(|u| counted(metric, status.state(&u.id)))
            .count();
        let percent = pct(done, members.len());
        let header_max = (frame.w as f32 - 16.0).max(8.0);
        let (nm, stats, sz) = component_header(&name, done, members.len(), percent, header_max);
        let _ = write!(
            svg,
            r#"<g><title>{} {}</title>"#,
            esc(nm.trim()),
            esc(stats.trim())
        );
        let _ = write!(
            svg,
            r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{PANEL}"/>"#,
            frame.x, frame.y, frame.w, HEADER_H
        );
        let tx = frame.x as f32 + 8.0;
        let ty = frame.y as f32 + (HEADER_H as f32 - sz) / 2.0;
        glyphs::write_text(&mut svg, BOLD, &nm, tx, ty, sz, TEXT, 0.0);
        let nx = tx + glyphs::measure(BOLD, &nm, sz, 0.0);
        glyphs::write_text(&mut svg, REGULAR, &stats, nx, ty, sz, MUTED, 0.0);

        let cells = unit_cells(members.len(), inner);
        debug_assert_eq!(cells.len(), members.len());
        for (unit, cell) in members.iter().zip(&cells) {
            debug_assert!(inner.contains(*cell));
            let fill = color(metric, status.state(&unit.id));
            let _ = write!(
                svg,
                r#"<rect x="{:.1}" y="{:.1}" width="{:.0}" height="{:.0}" fill="{fill}"><title>{} ({:?})</title></rect>"#,
                cell.x,
                cell.y,
                cell.w,
                cell.h,
                esc(&unit.id),
                status.state(&unit.id)
            );
        }
        svg.push_str("</g>");
    }

    let legend: &[(&str, &str)] = match metric {
        Metric::State => &[
            (NOT_STARTED, "not started"),
            (IMPLEMENTED, "implemented"),
            (TESTED, "tested"),
            (GREEN, "conformant (matches real Supabase)"),
        ],
        Metric::Coverage => &[(GREEN, "implemented"), (NOT_STARTED, "not implemented")],
        Metric::Conformance => &[(GREEN, "conformant"), (NOT_STARTED, "not conformant")],
    };
    let mut x = MARGIN as f32;
    let y = (height - MARGIN - 14.0) as f32;
    let legend_size = 11.0_f32;
    for (fill, label) in legend {
        let _ = write!(svg, r#"<g><title>{}</title>"#, esc(label));
        let _ = write!(
            svg,
            r#"<rect x="{x:.1}" y="{y:.1}" width="10" height="10" fill="{fill}"/>"#
        );
        glyphs::write_text(
            &mut svg,
            REGULAR,
            label,
            x + 14.0,
            y - 1.0,
            legend_size,
            MUTED,
            0.0,
        );
        svg.push_str("</g>");
        x += 26.0 + glyphs::measure(REGULAR, label, legend_size, 0.0);
    }
    svg.push_str("</svg>\n");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;
    use std::collections::BTreeMap;

    fn unit(id: &str, component: &str, group: &str) -> Unit {
        Unit {
            id: id.into(),
            component: component.into(),
            group: group.into(),
            kind: "t".into(),
            name: id.into(),
            method: None,
            path: None,
            level: 1,
            source: Source {
                repo: "postgrest".into(),
                file: "a.rs".into(),
                line: 1,
            },
        }
    }

    fn empty_status() -> Status {
        Status {
            states: BTreeMap::new(),
            cases_total: 0,
            cases_passing: 0,
        }
    }

    #[test]
    fn squarify_preserves_area_and_bounds() {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 300.0,
            h: 200.0,
        };
        let values = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0];
        let rects = squarify(&values, rect);
        let total: f64 = rects.iter().map(|r| r.w * r.h).sum();
        assert!((total - 60000.0).abs() < 1e-6);
        for (r, v) in rects.iter().zip(values) {
            assert!((r.w * r.h - v / 24.0 * 60000.0).abs() < 1e-6);
            assert!(r.x >= -1e-9 && r.y >= -1e-9);
            assert!(r.x + r.w <= 300.0 + 1e-6 && r.y + r.h <= 200.0 + 1e-6);
        }
    }

    #[test]
    fn unit_cells_are_whole_fixed_squares_centered() {
        let inner = Rect {
            x: 10.0,
            y: 20.0,
            w: 200.0,
            h: 140.0,
        };
        let n = 10;
        let cells = unit_cells(n, inner);
        assert_eq!(cells.len(), n);
        for c in &cells {
            assert!((c.w - CELL).abs() < 1e-9);
            assert!((c.h - CELL).abs() < 1e-9);
            assert!(inner.contains(*c), "square clipped: {c:?} in {inner:?}");
        }
        let min_x = cells.iter().map(|c| c.x).fold(f64::MAX, f64::min);
        let max_x = cells.iter().map(|c| c.x + c.w).fold(f64::MIN, f64::max);
        let min_y = cells.iter().map(|c| c.y).fold(f64::MAX, f64::min);
        let max_y = cells.iter().map(|c| c.y + c.h).fold(f64::MIN, f64::max);
        assert!(((min_x - inner.x) - (inner.x + inner.w - max_x)).abs() < 0.6);
        assert!(((min_y - inner.y) - (inner.y + inner.h - max_y)).abs() < 0.6);
        assert!((cells[1].x - cells[0].x - STRIDE).abs() < 1e-9);
    }

    #[test]
    fn unit_cells_refuse_to_clip() {
        let tiny = Rect {
            x: 0.0,
            y: 0.0,
            w: 40.0,
            h: 20.0,
        };
        assert!(unit_cells(8, tiny).is_empty());
    }

    #[test]
    fn narrow_header_drops_percentage() {
        let (name, stats, _) = component_header("STORAGE", 0, 34, 0.0, 90.0);
        assert_eq!(name, "STORAGE");
        assert_eq!(stats.trim(), "0/34");
        assert!(!stats.contains('%'));
        let (name, stats, _) = component_header("REST", 0, 57, 0.0, 400.0);
        assert_eq!(name, "REST");
        assert!(stats.contains('%'), "{stats}");
        assert!(stats.contains("(0/57)"));
    }

    #[test]
    fn status_svg_uses_live_totals_and_brand_colors() {
        let rest: Vec<Unit> = (0..5)
            .map(|i| unit(&format!("rest:x:{i}"), "rest", "g"))
            .collect();
        let pooler: Vec<Unit> = (0..2)
            .map(|i| unit(&format!("pooler:x:{i}"), "pooler", "g"))
            .collect();
        let all: Vec<&Unit> = rest.iter().chain(pooler.iter()).collect();
        let svg = render(Heading::Status, &all, &empty_status(), Metric::State, 800.0);
        assert!(svg.contains("Supabase components: 0.0% conformant (0/7)"));
        assert!(svg.contains("aria-label=\"Supabase components: 0.0% conformant (0/7)\""));
        assert!(!svg.contains("1,024"));
        assert!(!svg.contains("1024"));
        assert!(!svg.contains("334"));
        let squares = svg.matches("width=\"26\" height=\"26\"").count();
        assert_eq!(squares, 7);
        for c in [
            "#0B0E12", "#181A1D", "#303235", "#005441", "#009366", "#00D892", "#F7F7F7", "#BABABB",
        ] {
            assert!(svg.contains(c), "missing {c}");
        }
        assert!(!svg.contains("#1FCB5B"));
        assert!(!svg.contains("#3ECF8E"));
        assert!(!svg.contains("#24B47E"));
        assert!(svg.contains("not started"));
        assert!(svg.contains("conformant (matches real Supabase)"));
    }

    #[test]
    fn pooler_label_does_not_use_wide_form_when_narrow() {
        let (name, stats, _) = component_header("POOLER", 0, 13, 0.0, 110.0);
        assert_eq!(name, "POOLER");
        assert_eq!(stats.trim(), "0/13");
    }

    #[test]
    fn production_component_sizes_draw_one_square_each() {
        let spec = [
            ("rest", 93),
            ("auth", 152),
            ("realtime", 19),
            ("storage", 149),
            ("functions", 276),
            ("pooler", 16),
            ("meta", 69),
            ("studio", 250),
        ];
        let mut owned = Vec::new();
        for (c, n) in spec {
            for i in 0..n {
                owned.push(unit(&format!("{c}:x:{i}"), c, "g"));
            }
        }
        let all: Vec<&Unit> = owned.iter().collect();
        let svg = render(
            Heading::Status,
            &all,
            &empty_status(),
            Metric::State,
            1280.0,
        );
        let n: usize = spec.iter().map(|(_, n)| n).sum();
        assert_eq!(svg.matches("width=\"26\" height=\"26\"").count(), n);
        assert!(svg.contains(&format!("conformant (0/{n})")));
    }
}
