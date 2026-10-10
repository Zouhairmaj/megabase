//! Nested squarified treemaps (Kite "Repo / README" Status): component cards
//! sized by unit count, feature groups inside each card, unit cells filling
//! each group. Totals always come from `coverage/units.json`.

use std::fmt::Write;

use crate::glyphs::{self, BOLD, REGULAR};
use crate::model::{component_label, Unit, COMPONENTS};
use crate::status::{pct, State, Status};

/// Gap between adjacent component cards and between feature groups (Kite).
pub const GUTTER: f64 = 10.0;
/// Gap between packed unit cells inside a group.
pub const CELL_GAP: f64 = 2.0;

const CARD_RX: f64 = 3.0;
const CARD_PAD: f64 = 8.0;
const NAME_INSET: f64 = 11.0;
const NAME_SIZE: f32 = 15.0;
const NAME_TRACKING_PX: f32 = 0.5;
const COUNT_SIZE: f32 = 12.0;
const TRACK_Y: f64 = 30.0;
const TRACK_H: f64 = 2.0;
const HEADER_H: f64 = 32.0;
const GROUP_LABEL_H: f64 = 21.0;
const GROUP_LABEL_SIZE: f32 = 10.5;
const GROUP_LABEL_TRACKING_PX: f32 = 0.4;
const MARGIN: f64 = 20.0;
const BADGE_H: f64 = 20.0;
const TITLE_H: f64 = 28.0;
const LEGEND_H: f64 = 32.0;
const BODY_H_MIN: f64 = 620.0;
const PX_PER_UNIT: f64 = 1100.0;
const IMPLEMENTED_OUTLINE: &str = "#009366";

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn right(self) -> f64 {
        self.x + self.w
    }

    fn bottom(self) -> f64 {
        self.y + self.h
    }

    fn contains(self, inner: Rect) -> bool {
        inner.x + 1e-6 >= self.x
            && inner.y + 1e-6 >= self.y
            && inner.right() <= self.right() + 1e-6
            && inner.bottom() <= self.bottom() + 1e-6
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
        let max = row.iter().copied().fold(f64::MIN, f64::max);
        let min = row.iter().copied().fold(f64::MAX, f64::min);
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

/// Shrink shared edges so neighbouring rects are separated by `gutter` px.
/// Outer edges of the parent bounds stay flush (no outer margin).
pub fn apply_gutters(rects: &mut [Rect], gutter: f64) {
    let n = rects.len();
    if n < 2 || gutter <= 0.0 {
        return;
    }
    let half = gutter / 2.0;
    let orig = rects.to_vec();
    const EPS: f64 = 0.75;
    for i in 0..n {
        let r = orig[i];
        let mut left = false;
        let mut right = false;
        let mut top = false;
        let mut bottom = false;
        for (j, s) in orig.iter().enumerate() {
            if i == j {
                continue;
            }
            let y_overlap = r.y < s.bottom() - EPS && s.y < r.bottom() - EPS;
            let x_overlap = r.x < s.right() - EPS && s.x < r.right() - EPS;
            if y_overlap && (s.right() - r.x).abs() < EPS {
                left = true;
            }
            if y_overlap && (r.right() - s.x).abs() < EPS {
                right = true;
            }
            if x_overlap && (s.bottom() - r.y).abs() < EPS {
                top = true;
            }
            if x_overlap && (r.bottom() - s.y).abs() < EPS {
                bottom = true;
            }
        }
        let d = &mut rects[i];
        if left {
            d.x += half;
            d.w -= half;
        }
        if right {
            d.w -= half;
        }
        if top {
            d.y += half;
            d.h -= half;
        }
        if bottom {
            d.h -= half;
        }
        d.w = d.w.max(0.0);
        d.h = d.h.max(0.0);
    }
}

/// Column count that makes packed cells as square as possible.
pub fn best_cols(n: usize, w: f64, h: f64, gap: f64) -> usize {
    if n <= 1 {
        return n.max(1);
    }
    let mut best = 1usize;
    let mut best_ratio = f64::MAX;
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let cw = (w - gap * (cols - 1) as f64) / cols as f64;
        let ch = (h - gap * (rows - 1) as f64) / rows as f64;
        if cw < 0.5 || ch < 0.5 {
            continue;
        }
        let ratio = cw.max(ch) / cw.min(ch);
        if ratio < best_ratio - 1e-9 || ((ratio - best_ratio).abs() < 1e-9 && cols > best) {
            best_ratio = ratio;
            best = cols;
        }
    }
    best
}

fn split_axis(len: f64, n: usize, gap: f64) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    let inner = (len - gap * (n.saturating_sub(1) as f64)).max(0.0);
    vec![inner / n as f64; n]
}

/// Pack `n` cells so they fill `inner`. Last row may have fewer, wider cells.
pub fn unit_cells(n: usize, inner: Rect) -> Vec<Rect> {
    if n == 0 || inner.w < 0.5 || inner.h < 0.5 {
        return Vec::new();
    }
    let cols = best_cols(n, inner.w, inner.h, CELL_GAP);
    let rows = n.div_ceil(cols);
    let row_h = split_axis(inner.h, rows, CELL_GAP);
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    let mut y = inner.y;
    for (r, h) in row_h.iter().enumerate() {
        let remaining = n - i;
        let cols_this = if r + 1 == rows {
            remaining
        } else {
            cols.min(remaining)
        };
        let col_w = split_axis(inner.w, cols_this, CELL_GAP);
        let mut x = inner.x;
        for w in col_w {
            out.push(Rect { x, y, w, h: *h });
            x += w + CELL_GAP;
            i += 1;
        }
        y += *h + CELL_GAP;
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

struct Palette {
    bg: &'static str,
    card: &'static str,
    name: &'static str,
    count: &'static str,
    slash: &'static str,
    group_name: &'static str,
    group_count: &'static str,
    not_started: &'static str,
    implemented: &'static str,
    tested: &'static str,
    conformant: &'static str,
    muted: &'static str,
    title: &'static str,
}

impl Theme {
    fn palette(self) -> Palette {
        match self {
            Theme::Dark => Palette {
                bg: "#0B0E12",
                card: "#14171B",
                name: "#F7F7F7",
                count: "#F7F7F7",
                slash: "#BABABB",
                group_name: "#ABACAE",
                group_count: "#76777A",
                not_started: "#2A2C2F",
                implemented: "#005441",
                tested: "#009366",
                conformant: "#00D892",
                muted: "#BABABB",
                title: "#F7F7F7",
            },
            Theme::Light => Palette {
                bg: "#FFFFFF",
                card: "#F7F7F7",
                name: "#0B0E12",
                count: "#0B0E12",
                slash: "#303235",
                group_name: "#303235",
                group_count: "#76777A",
                not_started: "#DCDDDE",
                implemented: "#005441",
                tested: "#009366",
                conformant: "#00D892",
                muted: "#303235",
                title: "#0B0E12",
            },
        }
    }
}

fn color(pal: &Palette, metric: Metric, state: State) -> &'static str {
    match (metric, state) {
        (Metric::State, State::Missing) => pal.not_started,
        (Metric::State, State::Implemented) => pal.implemented,
        (Metric::State, State::Tested) => pal.tested,
        (Metric::State, State::Conformant) => pal.conformant,
        (Metric::Coverage, s) if s >= State::Implemented => pal.conformant,
        (Metric::Conformance, State::Conformant) => pal.conformant,
        _ => pal.not_started,
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

fn display_group_name(group: &str) -> String {
    group.replace(['-', '_'], " ").to_ascii_uppercase()
}

/// Name (truncated) and optional `n/total`. Drops the label when nothing fits.
pub fn fit_group_label(
    name: &str,
    done: usize,
    n: usize,
    max_w: f32,
) -> Option<(String, Option<String>)> {
    let size = GROUP_LABEL_SIZE;
    let track = glyphs::tracking_for_px(REGULAR, size, GROUP_LABEL_TRACKING_PX);
    if max_w < 8.0 {
        return None;
    }
    let count = format!("{done}/{n}");
    let count_w = glyphs::measure(REGULAR, &count, size, 0.0);
    let gap = 6.0;
    let name_budget = max_w - count_w - gap;
    if name_budget >= 10.0 {
        let nm = glyphs::truncate(REGULAR, name, size, track, name_budget);
        if !nm.is_empty() {
            return Some((nm, Some(count)));
        }
    }
    let nm = glyphs::truncate(REGULAR, name, size, track, max_w);
    if nm.is_empty() {
        None
    } else {
        Some((nm, None))
    }
}

fn card_inner(frame: Rect) -> Rect {
    Rect {
        x: frame.x + CARD_PAD,
        y: frame.y + HEADER_H,
        w: (frame.w - 2.0 * CARD_PAD).max(0.0),
        h: (frame.h - HEADER_H - CARD_PAD).max(0.0),
    }
}

fn body_height(total: usize, body_w: f64) -> f64 {
    let h = (total.max(1) as f64 * PX_PER_UNIT) / body_w.max(1.0);
    h.clamp(BODY_H_MIN, 1800.0)
}

fn draw_badge(
    svg: &mut String,
    pal: &Palette,
    x: f32,
    y: f32,
    label: &str,
    value: &str,
    value_bg: &str,
) -> f32 {
    let size = 11.0;
    let pad = 8.0;
    let h = BADGE_H as f32;
    let lw = (glyphs::measure(REGULAR, label, size, 0.0) + pad * 2.0).ceil();
    let vw = (glyphs::measure(REGULAR, value, size, 0.0) + pad * 2.0).ceil();
    let w = lw + vw;
    let value_fg = if value_bg == pal.conformant {
        pal.bg
    } else {
        pal.name
    };
    let _ = write!(
        svg,
        r##"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="3" fill="{card}"/>"##,
        card = pal.card
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
    glyphs::write_text(svg, REGULAR, label, x + pad, ty, size, pal.muted, 0.0);
    glyphs::write_text(svg, REGULAR, value, x + lw + pad, ty, size, value_fg, 0.0);
    w
}

fn metric_badge_color(pal: &Palette, percent: f64) -> &'static str {
    if percent >= 90.0 {
        pal.conformant
    } else if percent >= 50.0 {
        pal.tested
    } else if percent > 0.0 {
        pal.implemented
    } else {
        pal.not_started
    }
}

fn draw_cell(
    svg: &mut String,
    pal: &Palette,
    metric: Metric,
    unit: &Unit,
    state: State,
    cell: Rect,
) {
    let fill = color(pal, metric, state);
    let _ = write!(
        svg,
        r#"<rect data-unit="{}" x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{fill}"><title>{} ({state:?})</title></rect>"#,
        esc(&unit.id),
        cell.x,
        cell.y,
        cell.w,
        cell.h,
        esc(&unit.id),
    );
    if metric == Metric::State && state == State::Implemented && cell.w > 2.0 && cell.h > 2.0 {
        let _ = write!(
            svg,
            r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="none" stroke="{IMPLEMENTED_OUTLINE}" stroke-width="1"/>"#,
            cell.x + 0.5,
            cell.y + 0.5,
            (cell.w - 1.0).max(0.0),
            (cell.h - 1.0).max(0.0),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_card(
    svg: &mut String,
    pal: &Palette,
    clip_id: &str,
    frame: Rect,
    name: &str,
    members: &[&Unit],
    status: &Status,
    metric: Metric,
) {
    let done = members
        .iter()
        .filter(|u| counted(metric, status.state(&u.id)))
        .count();
    let n = members.len();
    let implemented = members
        .iter()
        .filter(|u| status.state(&u.id) >= State::Implemented)
        .count();
    let tested = members
        .iter()
        .filter(|u| status.state(&u.id) >= State::Tested)
        .count();
    let conformant = members
        .iter()
        .filter(|u| status.state(&u.id) == State::Conformant)
        .count();

    let _ = write!(svg, r#"<g><title>{} {done}/{n}</title>"#, esc(name));
    let _ = write!(
        svg,
        r#"<clipPath id="{clip_id}"><rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="{CARD_RX}"/></clipPath>"#,
        frame.x, frame.y, frame.w, frame.h
    );
    let _ = write!(
        svg,
        r#"<g clip-path="url(#{clip_id})"><rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="{CARD_RX}" fill="{}"/>"#,
        frame.x, frame.y, frame.w, frame.h, pal.card
    );

    let pad = NAME_INSET as f32;
    let done_s = format!("{done}");
    let slash_s = format!("/{n}");
    let slash_w = glyphs::measure(REGULAR, &slash_s, COUNT_SIZE, 0.0);
    let done_w = glyphs::measure(BOLD, &done_s, COUNT_SIZE, 0.0);
    let right = frame.x as f32 + frame.w as f32 - pad;
    let name_x = frame.x as f32 + pad;
    let name_max = (right - slash_w - done_w - 8.0 - name_x).max(0.0);
    let mut name_size = NAME_SIZE;
    let mut track = glyphs::tracking_for_px(BOLD, name_size, NAME_TRACKING_PX);
    let mut label = name.to_string();
    while name_size > 10.0
        && glyphs::measure(BOLD, name, name_size, track) > name_max
        && name_max > 0.0
    {
        name_size -= 0.5;
        track = glyphs::tracking_for_px(BOLD, name_size, NAME_TRACKING_PX);
    }
    if name_max > 0.0 {
        label = glyphs::truncate(BOLD, name, name_size, track, name_max);
    } else {
        label.clear();
    }
    let name_y = frame.y as f32 + 8.0;
    if !label.is_empty() {
        glyphs::write_text(
            svg, BOLD, &label, name_x, name_y, name_size, pal.name, track,
        );
    }
    let count_y = frame.y as f32 + 8.5;
    glyphs::write_text(
        svg,
        BOLD,
        &done_s,
        right - slash_w - done_w,
        count_y,
        COUNT_SIZE,
        pal.count,
        0.0,
    );
    glyphs::write_text(
        svg,
        REGULAR,
        &slash_s,
        right - slash_w,
        count_y,
        COUNT_SIZE,
        pal.slash,
        0.0,
    );

    let track_x = frame.x + CARD_PAD;
    let track_y = frame.y + TRACK_Y;
    let track_w = (frame.w - 2.0 * CARD_PAD).max(0.0);
    let _ = write!(
        svg,
        r#"<rect x="{track_x:.1}" y="{track_y:.1}" width="{track_w:.1}" height="{TRACK_H:.1}" fill="{}"/>"#,
        pal.not_started
    );
    if n > 0 && track_w > 0.0 {
        let draw = |svg: &mut String, count: usize, fill: &str| {
            if count == 0 {
                return;
            }
            let w = track_w * count as f64 / n as f64;
            let _ = write!(
                svg,
                r#"<rect x="{track_x:.1}" y="{track_y:.1}" width="{w:.1}" height="{TRACK_H:.1}" fill="{fill}"/>"#
            );
        };
        match metric {
            Metric::State => {
                draw(svg, implemented, pal.implemented);
                draw(svg, tested, pal.tested);
                draw(svg, conformant, pal.conformant);
            }
            Metric::Coverage => draw(svg, implemented, pal.conformant),
            Metric::Conformance => draw(svg, conformant, pal.conformant),
        }
    }

    let inner = card_inner(frame);
    if inner.w < 1.0 || inner.h < 1.0 || members.is_empty() {
        svg.push_str("</g></g>");
        return;
    }

    let mut groups: Vec<(String, Vec<&Unit>)> = Vec::new();
    for unit in members {
        if let Some((_, list)) = groups.iter_mut().find(|(g, _)| g == &unit.group) {
            list.push(*unit);
        } else {
            groups.push((unit.group.clone(), vec![*unit]));
        }
    }
    let weights: Vec<f64> = groups.iter().map(|(_, m)| m.len() as f64).collect();
    let mut group_rects = squarify(&weights, inner);
    apply_gutters(&mut group_rects, GUTTER);

    for ((gname, gunits), grec) in groups.iter().zip(&group_rects) {
        if grec.w < 1.0 || grec.h < 1.0 {
            continue;
        }
        let gdone = gunits
            .iter()
            .filter(|u| counted(metric, status.state(&u.id)))
            .count();
        let _ = write!(
            svg,
            r#"<g data-group="{}"><title>{} {gdone}/{}</title>"#,
            esc(&display_group_name(gname)),
            esc(&display_group_name(gname)),
            gunits.len()
        );
        let label_w = grec.w as f32 - 2.0;
        let shown = if grec.h >= GROUP_LABEL_H + 8.0 {
            fit_group_label(&display_group_name(gname), gdone, gunits.len(), label_w)
        } else {
            None
        };
        let cells_rect = if shown.is_some() {
            Rect {
                x: grec.x,
                y: grec.y + GROUP_LABEL_H,
                w: grec.w,
                h: (grec.h - GROUP_LABEL_H).max(0.0),
            }
        } else {
            *grec
        };
        if let Some((nm, count)) = shown {
            let ly = grec.y as f32 + 6.0;
            let track = glyphs::tracking_for_px(REGULAR, GROUP_LABEL_SIZE, GROUP_LABEL_TRACKING_PX);
            glyphs::write_text(
                svg,
                REGULAR,
                &nm,
                grec.x as f32 + 1.0,
                ly,
                GROUP_LABEL_SIZE,
                pal.group_name,
                track,
            );
            if let Some(count) = count {
                let cw = glyphs::measure(REGULAR, &count, GROUP_LABEL_SIZE, 0.0);
                glyphs::write_text(
                    svg,
                    REGULAR,
                    &count,
                    grec.x as f32 + grec.w as f32 - cw - 1.0,
                    ly,
                    GROUP_LABEL_SIZE,
                    pal.group_count,
                    0.0,
                );
            }
        }
        let cells = unit_cells(gunits.len(), cells_rect);
        for (unit, cell) in gunits.iter().zip(&cells) {
            debug_assert!(cells_rect.contains(*cell));
            draw_cell(svg, pal, metric, unit, status.state(&unit.id), *cell);
        }
        svg.push_str("</g>");
    }
    svg.push_str("</g></g>");
}

pub fn render(
    heading: &Heading,
    units: &[&Unit],
    status: &Status,
    metric: Metric,
    width: f64,
    theme: Theme,
) -> String {
    let pal = theme.palette();
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
    let metric_pct = pct(metric_done, total);

    let chrome_h = match heading {
        Heading::Status => MARGIN + BADGE_H + 12.0 + TITLE_H + 8.0,
        Heading::Title(_) => MARGIN + TITLE_H + 8.0,
    };
    let counts: Vec<f64> = components.iter().map(|(_, m)| m.len() as f64).collect();
    let body_x = MARGIN;
    let body_w = (width - 2.0 * MARGIN).max(80.0);
    let body_y = chrome_h;
    let body_h = body_height(total, body_w);
    let body = Rect {
        x: body_x,
        y: body_y,
        w: body_w,
        h: body_h,
    };
    let mut rects = squarify(&counts, body);
    apply_gutters(&mut rects, GUTTER);
    let height = body_y + body_h + LEGEND_H + MARGIN;

    let aria = match &heading {
        Heading::Status => {
            format!("Supabase components: {conformant} of {total} units conformant")
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
    let _ = write!(
        svg,
        r#"<rect width="100%" height="100%" fill="{}"/>"#,
        pal.bg
    );

    let mut cursor_x = MARGIN as f32;
    let badge_y = MARGIN as f32;
    if matches!(heading, Heading::Status) {
        cursor_x += draw_badge(
            &mut svg,
            &pal,
            cursor_x,
            badge_y,
            "coverage",
            &format!("{coverage_pct:.1}%"),
            metric_badge_color(&pal, coverage_pct),
        ) + 8.0;
        // Decision 0032: no standalone conformance percentage here. The
        // README shields badge (live Judge score) is that number.
        draw_badge(
            &mut svg,
            &pal,
            cursor_x,
            badge_y,
            "units",
            &format!("{implemented} / {total}"),
            pal.not_started,
        );
    }

    let title_y = match heading {
        Heading::Status => MARGIN + BADGE_H + 12.0,
        Heading::Title(_) => MARGIN,
    };
    let title_size = 14.0_f32;
    let title_text = match &heading {
        Heading::Status => {
            format!("Supabase components: {conformant} of {total} units conformant")
        }
        Heading::Title(t) => format!("{t}: {metric_pct:.1}% ({metric_done}/{total})"),
    };
    let title_max = body_w as f32;
    let title_draw = glyphs::truncate(REGULAR, &title_text, title_size, 0.0, title_max);
    glyphs::write_text(
        &mut svg,
        REGULAR,
        &title_draw,
        body_x as f32,
        title_y as f32 + (TITLE_H as f32 - title_size) / 2.0,
        title_size,
        pal.title,
        0.0,
    );

    for (i, ((component, members), rect)) in components.iter().zip(&rects).enumerate() {
        if rect.w < 1.0 || rect.h < 1.0 {
            continue;
        }
        let name = component_label(component).to_ascii_uppercase();
        draw_card(
            &mut svg,
            &pal,
            &format!("mb-c{i}"),
            *rect,
            &name,
            members,
            status,
            metric,
        );
    }

    let legend: &[(&str, &str)] = match metric {
        Metric::State => &[
            (pal.not_started, "not started"),
            (pal.implemented, "implemented"),
            (pal.tested, "tested"),
            (pal.conformant, "conformant (matches real Supabase)"),
        ],
        Metric::Coverage => &[
            (pal.conformant, "implemented"),
            (pal.not_started, "not implemented"),
        ],
        Metric::Conformance => &[
            (pal.conformant, "conformant"),
            (pal.not_started, "not conformant"),
        ],
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
            pal.muted,
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
    fn gutters_are_ten_px_on_shared_edges() {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 100.0,
        };
        let mut rects = squarify(&[1.0, 1.0], rect);
        apply_gutters(&mut rects, GUTTER);
        let dx = (rects[0].x - rects[1].right())
            .abs()
            .min((rects[1].x - rects[0].right()).abs());
        let dy = (rects[0].y - rects[1].bottom())
            .abs()
            .min((rects[1].y - rects[0].bottom()).abs());
        let gap = dx.min(dy);
        assert!(
            (gap - GUTTER).abs() < 0.05,
            "gap={gap} dx={dx} dy={dy} a={:?} b={:?}",
            rects[0],
            rects[1]
        );
    }

    #[test]
    fn unit_cells_fill_the_group() {
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
            assert!(inner.contains(*c), "cell clipped: {c:?} in {inner:?}");
            assert!(c.w > 8.0 && c.h > 8.0);
        }
        let min_x = cells.iter().map(|c| c.x).fold(f64::MAX, f64::min);
        let max_x = cells.iter().map(|c| c.right()).fold(f64::MIN, f64::max);
        let min_y = cells.iter().map(|c| c.y).fold(f64::MAX, f64::min);
        let max_y = cells.iter().map(|c| c.bottom()).fold(f64::MIN, f64::max);
        assert!((min_x - inner.x).abs() < 0.05);
        assert!((max_x - inner.right()).abs() < 0.05);
        assert!((min_y - inner.y).abs() < 0.05);
        assert!((max_y - inner.bottom()).abs() < 0.05);
    }

    #[test]
    fn last_row_may_use_wider_cells() {
        let inner = Rect {
            x: 0.0,
            y: 0.0,
            w: 238.8,
            h: 144.8,
        };
        let cells = unit_cells(22, inner);
        assert_eq!(cells.len(), 22);
        let last_y = cells.iter().map(|c| c.y).fold(f64::MIN, f64::max);
        let last: Vec<_> = cells
            .iter()
            .filter(|c| (c.y - last_y).abs() < 1e-6)
            .collect();
        let first: Vec<_> = cells.iter().filter(|c| c.y.abs() < 1e-6).collect();
        assert!(
            last.len() < first.len(),
            "last={} first={}",
            last.len(),
            first.len()
        );
        assert!(last[0].w > first[0].w);
    }

    #[test]
    fn group_label_truncates_then_drops_count() {
        let (name, count) = fit_group_label("ENDPOINTS", 0, 28, 400.0).unwrap();
        assert_eq!(name, "ENDPOINTS");
        assert_eq!(count.as_deref(), Some("0/28"));
        let (name, count) = fit_group_label("ADMIN", 0, 12, 50.0).unwrap();
        assert!(name.starts_with('A') || name == "ADMIN", "{name}");
        if glyphs::measure(
            REGULAR,
            "ADMIN",
            GROUP_LABEL_SIZE,
            glyphs::tracking_for_px(REGULAR, GROUP_LABEL_SIZE, GROUP_LABEL_TRACKING_PX),
        ) + glyphs::measure(REGULAR, "0/12", GROUP_LABEL_SIZE, 0.0)
            + 6.0
            > 50.0
        {
            assert!(count.is_none() || name.contains('…'), "{name} {count:?}");
        }
        assert!(fit_group_label("ENDPOINTS", 0, 28, 4.0).is_none());
    }

    #[test]
    fn status_svg_uses_live_totals_and_brand_colors() {
        let rest: Vec<Unit> = (0..5)
            .map(|i| unit(&format!("rest:x:{i}"), "rest", "filtering"))
            .collect();
        let pooler: Vec<Unit> = (0..2)
            .map(|i| unit(&format!("pooler:x:{i}"), "pooler", "modes"))
            .collect();
        let all: Vec<&Unit> = rest.iter().chain(pooler.iter()).collect();
        let svg = render(
            &Heading::Status,
            &all,
            &empty_status(),
            Metric::State,
            800.0,
            Theme::Dark,
        );
        assert!(svg.contains("Supabase components: 0 of 7 units conformant"));
        assert!(svg.contains("aria-label=\"Supabase components: 0 of 7 units conformant\""));
        assert!(!svg.contains("1,024"));
        assert!(!svg.contains("1024"));
        assert!(!svg.contains("334"));
        assert_eq!(svg.matches("data-unit=\"").count(), 7);
        assert!(!svg.contains("width=\"26\" height=\"26\""));
        for c in [
            "#0B0E12", "#14171B", "#2A2C2F", "#005441", "#009366", "#00D892", "#F7F7F7", "#BABABB",
            "#ABACAE",
        ] {
            assert!(svg.contains(c), "missing {c}");
        }
        assert!(!svg.contains("#1FCB5B"));
        assert!(!svg.contains("#3ECF8E"));
        assert!(!svg.contains("#24B47E"));
        assert!(svg.contains("not started"));
        assert!(svg.contains("conformant (matches real Supabase)"));
        assert!(svg.contains("rx=\"3\""));
        assert!(svg.contains("FILTERING") || svg.contains("filtering"));
    }

    #[test]
    fn light_theme_uses_kite_light_palette() {
        let rest: Vec<Unit> = (0..3)
            .map(|i| unit(&format!("rest:x:{i}"), "rest", "rpc"))
            .collect();
        let all: Vec<&Unit> = rest.iter().collect();
        let svg = render(
            &Heading::Status,
            &all,
            &empty_status(),
            Metric::State,
            800.0,
            Theme::Light,
        );
        for c in ["#FFFFFF", "#F7F7F7", "#DCDDDE", "#0B0E12", "#303235"] {
            assert!(svg.contains(c), "missing {c}");
        }
        assert_eq!(svg.matches("data-unit=\"").count(), 3);
    }

    #[test]
    fn implemented_cells_get_inset_outline() {
        let rest: Vec<Unit> = (0..4)
            .map(|i| unit(&format!("rest:x:{i}"), "rest", "g"))
            .collect();
        let mut states = BTreeMap::new();
        states.insert("rest:x:0".into(), State::Implemented);
        states.insert("rest:x:1".into(), State::Tested);
        states.insert("rest:x:2".into(), State::Conformant);
        let status = Status {
            states,
            cases_total: 0,
            cases_passing: 0,
        };
        let all: Vec<&Unit> = rest.iter().collect();
        let svg = render(
            &Heading::Status,
            &all,
            &status,
            Metric::State,
            800.0,
            Theme::Dark,
        );
        assert!(svg.contains(&format!("stroke=\"{IMPLEMENTED_OUTLINE}\"")));
        assert_eq!(
            svg.matches(&format!("stroke=\"{IMPLEMENTED_OUTLINE}\""))
                .count(),
            1
        );
        assert!(svg.contains("fill=\"#005441\""));
        assert!(svg.contains("fill=\"#009366\""));
        assert!(svg.contains("fill=\"#00D892\""));
    }

    #[test]
    fn production_component_sizes_draw_one_cell_each() {
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
            &Heading::Status,
            &all,
            &empty_status(),
            Metric::State,
            1280.0,
            Theme::Dark,
        );
        let n: usize = spec.iter().map(|(_, n)| n).sum();
        assert_eq!(svg.matches("data-unit=\"").count(), n);
        assert!(svg.contains(&format!("0 of {n} units conformant")));
        assert!(svg.contains("rx=\"3\""));
    }
}
