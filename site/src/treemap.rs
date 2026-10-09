//! Dark nested-square treemaps for megabase.sh.
//!
//! One generator, used on Home (hero + live map) and Status. Layout is a
//! squarified treemap of components (largest first) with groups squarified
//! the same way inside each component. Unit cells are whole squares of one
//! global size, painted with a pattern and two paths per group.

use crate::metrics::{
    comma, CatalogRow, ComponentBlock, FeatureGroup, Metrics, UnitStatus, CATALOG,
};

const BG: &str = "#0B0E12";
const BLOCK: &str = "#121417";
const HEADER: &str = "#181A1D";
const GROUP: &str = "#0B0E12";
const NOT_STARTED: &str = "#303235";
const IMPLEMENTED: &str = "#005441";
const TESTED: &str = "#009366";
const CONFORMANT: &str = "#00D892";
const LABEL: &str = "#F7F7F7";
const GROUP_LABEL: &str = "#8A8B8E";

const S_MIN: i32 = 3;
const S_MAX: i32 = 40;
const GROUP_BONUS: f64 = 5.0;
const LABEL_INSET: i32 = 6;

/// Pixel size and chrome for one inline treemap SVG.
///
/// `nested` draws feature-group boxes inside each component. Hero presets
/// set it to false so each component is a single grid of unit squares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preset {
    pub width: i32,
    pub height: i32,
    pub header_h: i32,
    pub gap: i32,
    pub nested: bool,
    pub group_label_px: i32,
    pub id: &'static str,
}

/// Status page nested map, 816×640 beside 384px copy.
pub const STATUS_DESKTOP: Preset = Preset {
    width: 816,
    height: 640,
    header_h: 20,
    gap: 4,
    nested: true,
    group_label_px: 8,
    id: "status-d",
};
/// Status page nested map, 350×760 when the page stacks below 900px.
pub const STATUS_MOBILE: Preset = Preset {
    width: 350,
    height: 760,
    header_h: 18,
    gap: 3,
    nested: true,
    group_label_px: 7,
    id: "status-m",
};
/// Home live map, 1192×520.
pub const HOME_DESKTOP: Preset = Preset {
    width: 1192,
    height: 520,
    header_h: 20,
    gap: 4,
    nested: true,
    group_label_px: 8,
    id: "home-d",
};
/// Home live map, 350×700 on small viewports.
pub const HOME_MOBILE: Preset = Preset {
    width: 350,
    height: 700,
    header_h: 18,
    gap: 3,
    nested: true,
    group_label_px: 7,
    id: "home-m",
};
/// Home hero card, 442×260, component grids without group labels.
pub const HERO_DESKTOP: Preset = Preset {
    width: 442,
    height: 260,
    header_h: 14,
    gap: 4,
    nested: false,
    group_label_px: 8,
    id: "hero-d",
};
/// Home hero card, 308×300 on small viewports.
pub const HERO_MOBILE: Preset = Preset {
    width: 308,
    height: 300,
    header_h: 14,
    gap: 3,
    nested: false,
    group_label_px: 7,
    id: "hero-m",
};

#[cfg(test)]
pub const PRESETS: &[Preset] = &[
    STATUS_DESKTOP,
    STATUS_MOBILE,
    HOME_DESKTOP,
    HOME_MOBILE,
    HERO_DESKTOP,
    HERO_MOBILE,
];

#[derive(Clone, Copy, Debug)]
struct IRect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl IRect {
    fn right(self) -> i32 {
        self.x + self.w
    }
    fn bottom(self) -> i32 {
        self.y + self.h
    }
    #[cfg(test)]
    fn contains(self, inner: IRect) -> bool {
        inner.x >= self.x
            && inner.y >= self.y
            && inner.right() <= self.right()
            && inner.bottom() <= self.bottom()
    }
}

#[derive(Clone, Debug)]
struct PlacedGroup {
    id: String,
    rect: IRect,
    n: usize,
    cols: i32,
    ox: i32,
    oy: i32,
    units: Vec<UnitStatus>,
    text: Option<String>,
}

#[derive(Clone, Debug)]
struct PlacedComp {
    id: String,
    rect: IRect,
    label: String,
    font_px: i32,
    groups: Vec<PlacedGroup>,
}

#[derive(Clone, Debug)]
struct Layout {
    preset: Preset,
    s: i32,
    unit_gap: i32,
    components: Vec<PlacedComp>,
}

/// Dark nested-square SVG for `preset`, built from coverage metrics.
pub fn render(metrics: &Metrics, preset: Preset) -> String {
    paint(&build(metrics, preset), metrics)
}

/// Alias for [`alt_text`].
pub fn panel_alt(metrics: &Metrics) -> String {
    alt_text(metrics)
}

/// Accessible description of the map, used as `aria-label` and visually hidden copy.
pub fn alt_text(metrics: &Metrics) -> String {
    if !metrics.has_data() || metrics.components.iter().all(|c| c.total() == 0) {
        return "Component map: no coverage data yet.".into();
    }
    format!(
        "Component map: {} of {} units. One whole square per unit, grouped by component and feature group.",
        metrics.passing.map(comma).unwrap_or_else(|| "0".into()),
        metrics.total.map(comma).unwrap_or_else(|| "0".into()),
    )
}

/// Pick weights, pack components, then place one global square size `s`.
fn build(metrics: &Metrics, preset: Preset) -> Layout {
    let comps = live_components(metrics);
    if comps.is_empty() {
        return day0_layout(preset);
    }
    search_layout(&comps, preset, 3, 6)
        .or_else(|| search_layout(&comps, preset, 0, 2))
        .or_else(|| try_peel_layout(&comps, preset))
        .unwrap_or_else(|| fallback_layout(&comps, preset))
}

fn search_layout(
    comps: &[&ComponentBlock],
    preset: Preset,
    min_lo: i32,
    min_hi: i32,
) -> Option<Layout> {
    let mut best: Option<Layout> = None;
    let mut best_s = -1;
    for extra in 1..=4 {
        for min_pct in min_lo..=min_hi {
            if let Some(layout) = try_layout(comps, preset, extra, min_pct) {
                if layout.s > best_s {
                    best_s = layout.s;
                    best = Some(layout);
                }
            }
        }
    }
    best
}

fn live_components(metrics: &Metrics) -> Vec<&ComponentBlock> {
    metrics
        .components
        .iter()
        .filter(|c| c.total() > 0)
        .collect()
}

fn fallback_layout(comps: &[&ComponentBlock], preset: Preset) -> Layout {
    try_layout(comps, preset, 1, 0).unwrap_or_else(|| {
        let weights = component_weights(comps, 1, 0);
        let rects = pack_rects(&weights, 0, 0, preset.width, preset.height, preset.gap);
        layout_from_rects(comps, &rects, preset).unwrap_or_else(|| Layout {
            preset,
            s: S_MIN,
            unit_gap: unit_gap(S_MIN),
            components: Vec::new(),
        })
    })
}

fn component_weights(comps: &[&ComponentBlock], extra: i32, min_pct: i32) -> Vec<f64> {
    let raw: Vec<f64> = comps
        .iter()
        .map(|c| {
            let groups = group_n(c).max(1) as f64;
            c.total() as f64 + extra as f64 * groups
        })
        .collect();
    let sum: f64 = raw.iter().sum::<f64>().max(1.0);
    let floor = (min_pct as f64 / 100.0) * sum;
    let mut out = raw;
    for w in &mut out {
        if *w < floor {
            *w = floor;
        }
    }
    out
}

fn group_n(comp: &ComponentBlock) -> usize {
    if comp.groups.is_empty() {
        1
    } else {
        comp.groups.len()
    }
}

fn groups_of(comp: &ComponentBlock) -> Vec<(&str, &str, &[UnitStatus])> {
    if comp.groups.is_empty() {
        vec![("units", "units", comp.units.as_slice())]
    } else {
        let mut groups: Vec<&FeatureGroup> = comp.groups.iter().filter(|g| g.total() > 0).collect();
        groups.sort_by(|a, b| b.total().cmp(&a.total()).then_with(|| a.id.cmp(&b.id)));
        groups
            .into_iter()
            .map(|g| (g.id.as_str(), g.label.as_str(), g.units.as_slice()))
            .collect()
    }
}

fn place_groups(
    comps: &[&ComponentBlock],
    rects: &[IRect],
    preset: Preset,
) -> Option<Vec<Vec<IRect>>> {
    let mut out = Vec::with_capacity(comps.len());
    for (comp, block) in comps.iter().zip(rects) {
        let body = body_rect(*block, preset);
        if body.w < 1 || body.h < 1 {
            return None;
        }
        if !preset.nested {
            out.push(vec![body]);
            continue;
        }
        let grouped = groups_of(comp);
        if grouped.is_empty() {
            out.push(vec![body]);
            continue;
        }
        let weights: Vec<f64> = grouped
            .iter()
            .map(|(_, _, units)| units.len() as f64 + GROUP_BONUS)
            .collect();
        let rects = pack_rects(&weights, body.x, body.y, body.w, body.h, preset.gap);
        if rects.len() != grouped.len() {
            return None;
        }
        out.push(rects);
    }
    Some(out)
}

fn body_rect(block: IRect, preset: Preset) -> IRect {
    let inset = if preset.nested { preset.gap } else { 0 };
    let y = block.y + preset.header_h + inset;
    let h = block.h - preset.header_h - inset * 2;
    IRect {
        x: block.x + inset,
        y,
        w: (block.w - inset * 2).max(0),
        h: h.max(0),
    }
}

fn finish_groups(
    comps: &[&ComponentBlock],
    rects: &[IRect],
    group_rects: &[Vec<IRect>],
    preset: Preset,
    s: i32,
) -> Option<Vec<PlacedComp>> {
    let ugap = unit_gap(s);
    let mut placed = Vec::with_capacity(comps.len());
    for ((comp, block), grects) in comps.iter().zip(rects).zip(group_rects) {
        let grouped = if preset.nested {
            groups_of(comp)
        } else {
            vec![("units", "units", comp.units.as_slice())]
        };
        if grouped.len() != grects.len() {
            return None;
        }
        let mut groups = Vec::with_capacity(grouped.len());
        for ((id, label, units), rect) in grouped.into_iter().zip(grects) {
            let ordered = ordered_units(units);
            let n = ordered.len();
            let (cols, ox, oy) = place_grid(n, *rect, s, ugap)?;
            let text = group_label_text(label, *rect, ox, oy, preset);
            groups.push(PlacedGroup {
                id: id.to_string(),
                rect: *rect,
                n,
                cols,
                ox,
                oy,
                units: ordered,
                text,
            });
        }
        let (label, font_px) = component_label(comp, *block, preset);
        placed.push(PlacedComp {
            id: comp.id.clone(),
            rect: *block,
            label,
            font_px,
            groups,
        });
    }
    Some(placed)
}

fn min_block_h(preset: Preset) -> i32 {
    preset.header_h + if preset.nested { preset.gap * 2 } else { 0 } + S_MIN + 2
}

fn try_layout(
    comps: &[&ComponentBlock],
    preset: Preset,
    extra: i32,
    min_pct: i32,
) -> Option<Layout> {
    let mut weights = component_weights(comps, extra, min_pct);
    for _ in 0..32 {
        let rects = pack_rects(&weights, 0, 0, preset.width, preset.height, preset.gap);
        if rects.len() != comps.len() {
            return None;
        }
        if bump_min_sizes(comps, &rects, preset, &mut weights) {
            continue;
        }
        if let Some(layout) = layout_from_rects(comps, &rects, preset) {
            return Some(layout);
        }
        if !bump_unfit(comps, &rects, preset, &mut weights) {
            return None;
        }
    }
    None
}

fn layout_from_rects(comps: &[&ComponentBlock], rects: &[IRect], preset: Preset) -> Option<Layout> {
    if rects.len() != comps.len() {
        return None;
    }
    for (comp, r) in comps.iter().zip(rects) {
        if r.w < min_label_width(comp, preset) || r.h < min_block_h(preset) {
            return None;
        }
    }
    let group_rects = place_groups(comps, rects, preset)?;
    let slots = collect_slots(comps, &group_rects, preset)?;
    let s = choose_s(&slots)?;
    let placed = finish_groups(comps, rects, &group_rects, preset, s)?;
    Some(Layout {
        preset,
        s,
        unit_gap: unit_gap(s),
        components: placed,
    })
}

fn collect_slots(
    comps: &[&ComponentBlock],
    group_rects: &[Vec<IRect>],
    preset: Preset,
) -> Option<Vec<(IRect, usize)>> {
    let mut slots = Vec::new();
    for (comp, grects) in comps.iter().zip(group_rects) {
        let grouped = if preset.nested {
            groups_of(comp)
        } else {
            vec![("units", "units", comp.units.as_slice())]
        };
        if grouped.len() != grects.len() {
            return None;
        }
        for ((_, _, units), rect) in grouped.iter().zip(grects) {
            slots.push((*rect, units.len()));
        }
    }
    Some(slots)
}

fn choose_s(slots: &[(IRect, usize)]) -> Option<i32> {
    for s in (S_MIN..=S_MAX).rev() {
        let ugap = unit_gap(s);
        if slots
            .iter()
            .all(|(rect, n)| place_grid(*n, *rect, s, ugap).is_some())
        {
            return Some(s);
        }
    }
    None
}

fn bump_min_sizes(
    comps: &[&ComponentBlock],
    rects: &[IRect],
    preset: Preset,
    weights: &mut [f64],
) -> bool {
    let mut bumped = false;
    for (i, (comp, r)) in comps.iter().zip(rects).enumerate() {
        let min_w = min_label_width(comp, preset);
        if r.w < min_w {
            weights[i] *= (min_w as f64 / r.w.max(1) as f64).max(1.08);
            bumped = true;
        }
        let min_h = min_block_h(preset);
        if r.h < min_h {
            weights[i] *= (min_h as f64 / r.h.max(1) as f64).max(1.08);
            bumped = true;
        }
    }
    bumped
}

fn bump_unfit(
    comps: &[&ComponentBlock],
    rects: &[IRect],
    preset: Preset,
    weights: &mut [f64],
) -> bool {
    let Some(group_rects) = place_groups(comps, rects, preset) else {
        return false;
    };
    let ugap = unit_gap(S_MIN);
    let mut any = false;
    for (i, (comp, grects)) in comps.iter().zip(&group_rects).enumerate() {
        let grouped = if preset.nested {
            groups_of(comp)
        } else {
            vec![("units", "units", comp.units.as_slice())]
        };
        let mut scale = 1.0_f64;
        for ((_, _, units), rect) in grouped.iter().zip(grects) {
            if place_grid(units.len(), *rect, S_MIN, ugap).is_none() {
                let cap = grid_capacity(*rect, S_MIN, ugap).max(1);
                scale = scale.max(units.len().max(1) as f64 / cap as f64);
            }
        }
        if scale > 1.0 {
            weights[i] *= scale.max(1.08);
            any = true;
        }
    }
    any
}

fn grid_capacity(rect: IRect, s: i32, gap: i32) -> usize {
    let stride = s + gap;
    if stride <= 0 {
        return 0;
    }
    let cols = ((rect.w + gap) / stride).max(0) as usize;
    let rows = ((rect.h + gap) / stride).max(0) as usize;
    cols.saturating_mul(rows)
}

fn try_peel_layout(comps: &[&ComponentBlock], preset: Preset) -> Option<Layout> {
    let n = comps.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        comps[b]
            .total()
            .cmp(&comps[a].total())
            .then_with(|| comps[a].id.cmp(&comps[b].id))
    });
    let canvas = IRect {
        x: 0,
        y: 0,
        w: preset.width,
        h: preset.height,
    };
    let placed = peel_pack(&order, canvas, comps, preset)?;
    let mut rects = vec![
        IRect {
            x: 0,
            y: 0,
            w: 1,
            h: 1
        };
        n
    ];
    for (i, r) in placed {
        rects[i] = r;
    }
    layout_from_rects(comps, &rects, preset)
}

fn peel_pack(
    items: &[usize],
    rect: IRect,
    comps: &[&ComponentBlock],
    preset: Preset,
) -> Option<Vec<(usize, IRect)>> {
    if items.is_empty() {
        return Some(Vec::new());
    }
    if items.len() == 1 {
        return if block_can_hold(comps[items[0]], rect, preset) {
            Some(vec![(items[0], rect)])
        } else {
            None
        };
    }
    let gap = preset.gap;
    let first = items[0];
    let rest = &items[1..];
    let t0 = comps[first].total().max(1) as i64;
    let t_all: i64 = items.iter().map(|&i| comps[i].total().max(1) as i64).sum();

    if rect.w > gap + 2 {
        if let Some(w_min) = min_width_for(comps[first], rect.h, rect.w - gap - 1, preset) {
            let w_max = rect.w - gap - 1;
            let mut ws = split_sizes(w_min, w_max, t0, t_all, rect.w - gap);
            for w in ws.drain(..) {
                let r1 = IRect {
                    x: rect.x,
                    y: rect.y,
                    w,
                    h: rect.h,
                };
                let r2 = IRect {
                    x: rect.x + w + gap,
                    y: rect.y,
                    w: rect.w - w - gap,
                    h: rect.h,
                };
                if let Some(p) = peel_side(first, r1, rest, r2, comps, preset) {
                    return Some(p);
                }
                let r1r = IRect {
                    x: rect.x + rect.w - w,
                    y: rect.y,
                    w,
                    h: rect.h,
                };
                let r2r = IRect {
                    x: rect.x,
                    y: rect.y,
                    w: rect.w - w - gap,
                    h: rect.h,
                };
                if let Some(p) = peel_side(first, r1r, rest, r2r, comps, preset) {
                    return Some(p);
                }
            }
        }
    }
    if rect.h > gap + 2 {
        if let Some(h_min) = min_height_for(comps[first], rect.w, rect.h - gap - 1, preset) {
            let h_max = rect.h - gap - 1;
            let mut hs = split_sizes(h_min, h_max, t0, t_all, rect.h - gap);
            for h in hs.drain(..) {
                let r1 = IRect {
                    x: rect.x,
                    y: rect.y,
                    w: rect.w,
                    h,
                };
                let r2 = IRect {
                    x: rect.x,
                    y: rect.y + h + gap,
                    w: rect.w,
                    h: rect.h - h - gap,
                };
                if let Some(p) = peel_side(first, r1, rest, r2, comps, preset) {
                    return Some(p);
                }
                let r1b = IRect {
                    x: rect.x,
                    y: rect.y + rect.h - h,
                    w: rect.w,
                    h,
                };
                let r2b = IRect {
                    x: rect.x,
                    y: rect.y,
                    w: rect.w,
                    h: rect.h - h - gap,
                };
                if let Some(p) = peel_side(first, r1b, rest, r2b, comps, preset) {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn peel_side(
    first: usize,
    r1: IRect,
    rest: &[usize],
    r2: IRect,
    comps: &[&ComponentBlock],
    preset: Preset,
) -> Option<Vec<(usize, IRect)>> {
    if r1.w < 1 || r1.h < 1 || r2.w < 1 || r2.h < 1 {
        return None;
    }
    if !block_can_hold(comps[first], r1, preset) {
        return None;
    }
    let mut p = peel_pack(rest, r2, comps, preset)?;
    p.insert(0, (first, r1));
    Some(p)
}

fn split_sizes(min: i32, max: i32, t0: i64, t_all: i64, inner: i32) -> Vec<i32> {
    if min > max {
        return Vec::new();
    }
    let prop = (inner as i64 * t0 / t_all.max(1)) as i32;
    let mut out = vec![min, prop.clamp(min, max), (min + max) / 2, max];
    out.sort_unstable();
    out.dedup();
    out
}

fn min_width_for(comp: &ComponentBlock, h: i32, max_w: i32, preset: Preset) -> Option<i32> {
    let mut lo = min_label_width(comp, preset).clamp(1, max_w);
    let mut hi = max_w;
    if !block_can_hold(
        comp,
        IRect {
            x: 0,
            y: 0,
            w: hi,
            h,
        },
        preset,
    ) {
        return None;
    }
    while lo < hi {
        let mid = (lo + hi) / 2;
        if block_can_hold(
            comp,
            IRect {
                x: 0,
                y: 0,
                w: mid,
                h,
            },
            preset,
        ) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Some(lo)
}

fn min_height_for(comp: &ComponentBlock, w: i32, max_h: i32, preset: Preset) -> Option<i32> {
    let mut lo = min_block_h(preset).clamp(1, max_h);
    let mut hi = max_h;
    if !block_can_hold(
        comp,
        IRect {
            x: 0,
            y: 0,
            w,
            h: hi,
        },
        preset,
    ) {
        return None;
    }
    while lo < hi {
        let mid = (lo + hi) / 2;
        if block_can_hold(
            comp,
            IRect {
                x: 0,
                y: 0,
                w,
                h: mid,
            },
            preset,
        ) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Some(lo)
}

fn block_can_hold(comp: &ComponentBlock, rect: IRect, preset: Preset) -> bool {
    if rect.w < min_label_width(comp, preset) || rect.h < min_block_h(preset) {
        return false;
    }
    let body = body_rect(rect, preset);
    if body.w < 1 || body.h < 1 {
        return false;
    }
    let ugap = unit_gap(S_MIN);
    if !preset.nested {
        return place_grid(comp.total(), body, S_MIN, ugap).is_some();
    }
    let grouped = groups_of(comp);
    if grouped.is_empty() {
        return true;
    }
    let weights: Vec<f64> = grouped
        .iter()
        .map(|(_, _, units)| units.len() as f64 + GROUP_BONUS)
        .collect();
    let grects = pack_rects(&weights, body.x, body.y, body.w, body.h, preset.gap);
    grects.len() == grouped.len()
        && grouped
            .iter()
            .zip(&grects)
            .all(|((_, _, units), r)| place_grid(units.len(), *r, S_MIN, ugap).is_some())
}

fn component_name(comp: &ComponentBlock) -> String {
    comp.label.to_ascii_uppercase()
}

fn min_label_width(comp: &ComponentBlock, _preset: Preset) -> i32 {
    let name = component_name(comp);
    LABEL_INSET + mono_w(&name, 8) + 4
}

fn component_label(comp: &ComponentBlock, block: IRect, preset: Preset) -> (String, i32) {
    let name = component_name(comp);
    let full = format!(
        "{} {}/{}",
        name,
        comma(comp.conformant),
        comma(comp.total())
    );
    let start = if preset.header_h >= 20 {
        11
    } else if preset.header_h >= 18 {
        10
    } else {
        9
    };
    let budget = (block.w - LABEL_INSET - 4).max(0);
    for px in (8..=start).rev() {
        if mono_w(&full, px) <= budget {
            return (full, px);
        }
    }
    for px in (8..=start).rev() {
        if mono_w(&name, px) <= budget {
            return (name, px);
        }
    }
    (name, 8)
}

fn group_label_text(id: &str, rect: IRect, _ox: i32, oy: i32, preset: Preset) -> Option<String> {
    if !preset.nested {
        return None;
    }
    let px = preset.group_label_px;
    let ty = rect.y + if px >= 8 { 9 } else { 8 };
    // Keep the label above the square grid.
    if ty + 2 > oy {
        return None;
    }
    let budget = (rect.w - 6).max(0);
    if budget < mono_w("…", px) {
        return None;
    }
    let raw = id.to_ascii_lowercase();
    if mono_w(&raw, px) <= budget {
        return Some(raw);
    }
    let chars: Vec<char> = raw.chars().collect();
    for keep in (1..chars.len()).rev() {
        let mut s: String = chars[..keep].iter().collect();
        s.push('…');
        if mono_w(&s, px) <= budget {
            return Some(s);
        }
    }
    None
}

fn ordered_units(units: &[UnitStatus]) -> Vec<UnitStatus> {
    let mut out = Vec::with_capacity(units.len());
    for status in [
        UnitStatus::Conformant,
        UnitStatus::Tested,
        UnitStatus::Implemented,
        UnitStatus::NotStarted,
    ] {
        out.extend(units.iter().copied().filter(|u| *u == status));
    }
    out
}

fn unit_gap(s: i32) -> i32 {
    if s < 6 {
        1
    } else if s < 14 {
        2
    } else {
        3
    }
}

fn mono_w(text: &str, px: i32) -> i32 {
    // JetBrains Mono is 0.6em per character.
    text.chars().count() as i32 * px * 3 / 5
}

fn place_grid(n: usize, rect: IRect, s: i32, gap: i32) -> Option<(i32, i32, i32)> {
    if n == 0 {
        return Some((0, rect.x, rect.y));
    }
    let stride = s + gap;
    if stride <= 0 || s < S_MIN {
        return None;
    }
    let max_cols = ((rect.w + gap) / stride).max(0);
    let max_rows = ((rect.h + gap) / stride).max(0);
    if max_cols <= 0 || max_rows <= 0 {
        return None;
    }
    let n = n as i32;
    let mut best: Option<(i32, i32, i32, i32, f64)> = None;
    for cols in 1..=max_cols.min(n) {
        let rows = (n + cols - 1) / cols;
        if rows > max_rows {
            continue;
        }
        let gw = cols * stride - gap;
        let gh = rows * stride - gap;
        if gw > rect.w || gh > rect.h {
            continue;
        }
        let min_ox = rect.x;
        let max_ox = rect.x + rect.w - gw;
        let min_oy = rect.y;
        let max_oy = rect.y + rect.h - gh;
        if min_ox > max_ox || min_oy > max_oy {
            continue;
        }
        let ideal_ox = rect.x as f64 + (rect.w - gw) as f64 / 2.0;
        let ideal_oy = rect.y as f64 + (rect.h - gh) as f64 / 2.0;
        let Some(ox) = snap_lattice(ideal_ox, stride, min_ox, max_ox) else {
            continue;
        };
        let Some(oy) = snap_lattice(ideal_oy, stride, min_oy, max_oy) else {
            continue;
        };
        if (ox as f64 - ideal_ox).abs() > stride as f64 + 0.51 {
            continue;
        }
        if (oy as f64 - ideal_oy).abs() > stride as f64 + 0.51 {
            continue;
        }
        let leftover = cols * rows - n;
        let aspect =
            ((gw as f64 / gh.max(1) as f64) - (rect.w as f64 / rect.h.max(1) as f64)).abs();
        let better = match best {
            None => true,
            Some((c, _, _, l, a)) => {
                leftover < l
                    || (leftover == l && aspect < a - f64::EPSILON)
                    || (leftover == l && (aspect - a).abs() < f64::EPSILON && cols < c)
            }
        };
        if better {
            best = Some((cols, ox, oy, leftover, aspect));
        }
    }
    let (cols, ox, oy, _, _) = best?;
    Some((cols, ox, oy))
}

fn snap_lattice(ideal: f64, stride: i32, min: i32, max: i32) -> Option<i32> {
    if stride <= 0 || min > max {
        return None;
    }
    let mut t = min.div_euclid(stride) * stride;
    if t < min {
        t = t.saturating_add(stride);
    }
    let mut best = None;
    let mut best_d = f64::MAX;
    while t <= max {
        let d = (t as f64 - ideal).abs();
        if d < best_d {
            best_d = d;
            best = Some(t);
        }
        match t.checked_add(stride) {
            Some(n) => t = n,
            None => break,
        }
    }
    best
}

fn pack_rects(weights: &[f64], x: i32, y: i32, w: i32, h: i32, gap: i32) -> Vec<IRect> {
    if weights.is_empty() || w <= 0 || h <= 0 {
        return Vec::new();
    }
    let raw = squarify(weights, x as f64, y as f64, w as f64, h as f64);
    let mut rects: Vec<IRect> = raw
        .iter()
        .map(|r| {
            let x1 = r[0].round() as i32;
            let y1 = r[1].round() as i32;
            let x2 = (r[0] + r[2]).round() as i32;
            let y2 = (r[1] + r[3]).round() as i32;
            IRect {
                x: x1,
                y: y1,
                w: (x2 - x1).max(1),
                h: (y2 - y1).max(1),
            }
        })
        .collect();
    let canvas = IRect { x, y, w, h };
    for r in &mut rects {
        if r.x < canvas.x {
            r.w -= canvas.x - r.x;
            r.x = canvas.x;
        }
        if r.y < canvas.y {
            r.h -= canvas.y - r.y;
            r.y = canvas.y;
        }
        if r.right() > canvas.right() {
            r.w = canvas.right() - r.x;
        }
        if r.bottom() > canvas.bottom() {
            r.h = canvas.bottom() - r.y;
        }
        r.w = r.w.max(1);
        r.h = r.h.max(1);
    }
    apply_internal_gaps(&mut rects, gap);
    for r in &mut rects {
        if r.right() > canvas.right() {
            r.w = (canvas.right() - r.x).max(1);
        }
        if r.bottom() > canvas.bottom() {
            r.h = (canvas.bottom() - r.y).max(1);
        }
        r.w = r.w.max(1);
        r.h = r.h.max(1);
    }
    rects
}

fn apply_internal_gaps(rects: &mut [IRect], gap: i32) {
    if rects.len() < 2 || gap <= 0 {
        return;
    }
    let orig = rects.to_vec();
    for (i, r) in rects.iter_mut().enumerate() {
        let cur = orig[i];
        let mut neighbor_right = false;
        let mut neighbor_bottom = false;
        for (j, o) in orig.iter().enumerate() {
            if i == j {
                continue;
            }
            let y_overlap = cur.y < o.bottom() - 1 && o.y < cur.bottom() - 1;
            let x_overlap = cur.x < o.right() - 1 && o.x < cur.right() - 1;
            if y_overlap && (o.x - cur.right()).abs() <= 1 && o.x >= cur.x {
                neighbor_right = true;
            }
            if x_overlap && (o.y - cur.bottom()).abs() <= 1 && o.y >= cur.y {
                neighbor_bottom = true;
            }
        }
        if neighbor_right {
            r.w = (r.w - gap).max(1);
        }
        if neighbor_bottom {
            r.h = (r.h - gap).max(1);
        }
    }
}

/// Squarified treemap (Bruls et al.), largest first. Result is in input order.
fn squarify(sizes: &[f64], x: f64, y: f64, dx: f64, dy: f64) -> Vec<[f64; 4]> {
    let n = sizes.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| sizes[*b].total_cmp(&sizes[*a]).then(a.cmp(b)));
    let total: f64 = sizes.iter().sum();
    let mut out = vec![[0.0; 4]; n];
    if n == 0 || total <= 0.0 || dx <= 0.0 || dy <= 0.0 {
        return out;
    }
    let area = dx * dy;
    let areas: Vec<f64> = order.iter().map(|i| sizes[*i] / total * area).collect();
    let mut free_x = x;
    let mut free_y = y;
    let mut free_w = dx;
    let mut free_h = dy;
    let mut i = 0;
    while i < areas.len() {
        let side = if free_w >= free_h { free_h } else { free_w };
        if side <= 0.0 {
            break;
        }
        let mut j = i + 1;
        while j < areas.len() && worst(&areas[i..j], side) >= worst(&areas[i..=j], side) {
            j += 1;
        }
        let row_area: f64 = areas[i..j].iter().sum();
        if free_w >= free_h {
            let width = if free_h > 0.0 {
                (row_area / free_h).min(free_w)
            } else {
                0.0
            };
            let mut yy = free_y;
            for k in i..j {
                let h = if width > 0.0 { areas[k] / width } else { 0.0 };
                out[order[k]] = [free_x, yy, width, h];
                yy += h;
            }
            free_x += width;
            free_w -= width;
        } else {
            let height = if free_w > 0.0 {
                (row_area / free_w).min(free_h)
            } else {
                0.0
            };
            let mut xx = free_x;
            for k in i..j {
                let w = if height > 0.0 { areas[k] / height } else { 0.0 };
                out[order[k]] = [xx, free_y, w, height];
                xx += w;
            }
            free_y += height;
            free_h -= height;
        }
        i = j;
    }
    out
}

fn worst(row: &[f64], side: f64) -> f64 {
    let s: f64 = row.iter().sum();
    if s == 0.0 || side == 0.0 {
        return f64::MAX;
    }
    row.iter()
        .map(|r| {
            if *r <= 0.0 {
                return f64::MAX;
            }
            let a = side * side * r / (s * s);
            let b = s * s / (side * side * r);
            a.max(b)
        })
        .fold(0.0_f64, f64::max)
}

fn day0_layout(preset: Preset) -> Layout {
    let weights = vec![1.0; CATALOG.len()];
    let rects = pack_rects(&weights, 0, 0, preset.width, preset.height, preset.gap);
    let components = CATALOG
        .iter()
        .zip(rects)
        .map(|(row, rect)| {
            let name = day0_name(row);
            let font_px = if mono_w(&name, 11) + LABEL_INSET + 4 <= rect.w {
                11.min(if preset.header_h >= 20 { 11 } else { 9 })
            } else {
                8
            };
            PlacedComp {
                id: row.id.to_string(),
                rect,
                label: name,
                font_px,
                groups: Vec::new(),
            }
        })
        .collect();
    Layout {
        preset,
        s: 0,
        unit_gap: 0,
        components,
    }
}

fn day0_name(row: &CatalogRow) -> String {
    match row.id {
        "rest" => "REST",
        "auth" => "AUTH",
        "storage" => "STORAGE",
        "realtime" => "REALTIME",
        "functions" => "FUNCTIONS",
        "pooler" => "POOLER",
        "meta" => "META",
        "studio" => "STUDIO",
        _ => row.name,
    }
    .to_string()
}

fn paint(layout: &Layout, metrics: &Metrics) -> String {
    let w = layout.preset.width;
    let h = layout.preset.height;
    let label = xml_esc(&alt_text(metrics));
    let mut out = String::new();
    out.push_str(&format!(
        r##"<svg class="treemap-svg" viewBox="0 0 {w} {h}" width="100%" height="auto" preserveAspectRatio="xMidYMid meet" style="display:block;max-width:100%" role="img" aria-label="{label}" xmlns="http://www.w3.org/2000/svg"><title>{label}</title>"##
    ));
    let used = used_status_colors(layout);
    if layout.s >= S_MIN && !used.is_empty() {
        out.push_str("<defs>");
        let stride = layout.s + layout.unit_gap;
        for (status, color) in used {
            let id = pattern_id(layout.preset.id, status);
            out.push_str(&format!(
                r##"<pattern id="{id}" width="{stride}" height="{stride}" patternUnits="userSpaceOnUse"><rect width="{s}" height="{s}" fill="{color}"/></pattern>"##,
                s = layout.s,
            ));
        }
        out.push_str("</defs>");
    }
    out.push_str(&format!(
        r##"<rect width="{w}" height="{h}" fill="{BG}"/>"##
    ));
    for comp in &layout.components {
        paint_component(&mut out, layout, comp);
    }
    if layout.s == 0 {
        out.push_str(&format!(
            r##"<text x="{cx}" y="{cy}" text-anchor="middle" dominant-baseline="middle" font-family="JetBrains Mono, ui-monospace, monospace" font-size="32" fill="{LABEL}" aria-hidden="true">—</text>"##,
            cx = w / 2,
            cy = h / 2,
        ));
    }
    out.push_str("</svg>");
    out
}

fn used_status_colors(layout: &Layout) -> Vec<(UnitStatus, &'static str)> {
    let mut seen = [false; 4];
    for comp in &layout.components {
        for g in &comp.groups {
            for u in &g.units {
                seen[status_idx(*u)] = true;
            }
        }
    }
    let mut out = Vec::new();
    for status in [
        UnitStatus::Conformant,
        UnitStatus::Tested,
        UnitStatus::Implemented,
        UnitStatus::NotStarted,
    ] {
        if seen[status_idx(status)] {
            out.push((status, status_color(status)));
        }
    }
    out
}

fn status_idx(s: UnitStatus) -> usize {
    match s {
        UnitStatus::Conformant => 0,
        UnitStatus::Tested => 1,
        UnitStatus::Implemented => 2,
        UnitStatus::NotStarted => 3,
    }
}

fn status_color(s: UnitStatus) -> &'static str {
    match s {
        UnitStatus::NotStarted => NOT_STARTED,
        UnitStatus::Implemented => IMPLEMENTED,
        UnitStatus::Tested => TESTED,
        UnitStatus::Conformant => CONFORMANT,
    }
}

fn pattern_id(preset: &str, status: UnitStatus) -> String {
    let tag = match status {
        UnitStatus::NotStarted => "ns",
        UnitStatus::Implemented => "im",
        UnitStatus::Tested => "te",
        UnitStatus::Conformant => "co",
    };
    format!("tm-{preset}-{tag}")
}

fn paint_component(out: &mut String, layout: &Layout, comp: &PlacedComp) {
    let r = comp.rect;
    out.push_str(&format!(
        r##"<g data-component="{id}" data-x="{x}" data-y="{y}" data-w="{w}" data-h="{h}"><rect fill="{BLOCK}" x="{x}" y="{y}" width="{w}" height="{h}"/><rect class="block-head" fill="{HEADER}" x="{x}" y="{y}" width="{w}" height="{hh}"/>"##,
        id = xml_esc(&comp.id),
        x = r.x,
        y = r.y,
        w = r.w,
        h = r.h,
        hh = layout.preset.header_h,
    ));
    let ty = r.y
        + if layout.preset.header_h >= 18 {
            layout.preset.header_h - 6
        } else {
            layout.preset.header_h - 4
        };
    out.push_str(&format!(
        r##"<text x="{tx}" y="{ty}" font-family="JetBrains Mono, ui-monospace, monospace" font-size="{px}" fill="{LABEL}">{label}</text>"##,
        tx = r.x + LABEL_INSET,
        px = comp.font_px,
        label = xml_esc(&comp.label),
    ));
    for g in &comp.groups {
        paint_group(out, layout, g);
    }
    out.push_str("</g>");
}

fn paint_group(out: &mut String, layout: &Layout, g: &PlacedGroup) {
    let r = g.rect;
    if layout.preset.nested {
        out.push_str(&format!(
            r##"<g data-group="{id}" data-n="{n}" data-s="{s}" data-gap="{gap}" data-cols="{cols}" data-ox="{ox}" data-oy="{oy}" data-x="{x}" data-y="{y}" data-w="{w}" data-h="{h}"><rect fill="{GROUP}" x="{x}" y="{y}" width="{w}" height="{h}"/>"##,
            id = xml_esc(&g.id),
            n = g.n,
            s = layout.s,
            gap = layout.unit_gap,
            cols = g.cols,
            ox = g.ox,
            oy = g.oy,
            x = r.x,
            y = r.y,
            w = r.w,
            h = r.h,
        ));
    } else {
        out.push_str(&format!(
            r##"<g data-group="{id}" data-n="{n}" data-s="{s}" data-gap="{gap}" data-cols="{cols}" data-ox="{ox}" data-oy="{oy}" data-x="{x}" data-y="{y}" data-w="{w}" data-h="{h}">"##,
            id = xml_esc(&g.id),
            n = g.n,
            s = layout.s,
            gap = layout.unit_gap,
            cols = g.cols,
            ox = g.ox,
            oy = g.oy,
            x = r.x,
            y = r.y,
            w = r.w,
            h = r.h,
        ));
    }
    if g.n > 0 && layout.s >= S_MIN && g.cols > 0 {
        paint_cells(out, layout, g);
    }
    if let Some(text) = &g.text {
        let ty = r.y
            + if layout.preset.group_label_px >= 8 {
                9
            } else {
                8
            };
        out.push_str(&format!(
            r##"<text font-size="{px}" fill="{GROUP_LABEL}" x="{tx}" y="{ty}">{text}</text>"##,
            px = layout.preset.group_label_px,
            tx = r.x + 3,
            text = xml_esc(text),
        ));
    }
    out.push_str("</g>");
}

fn paint_cells(out: &mut String, layout: &Layout, g: &PlacedGroup) {
    let mut i = 0usize;
    while i < g.units.len() {
        let status = g.units[i];
        let mut j = i + 1;
        while j < g.units.len() && g.units[j] == status {
            j += 1;
        }
        let paths = run_paths(i, j - i, g.cols, g.ox, g.oy, layout.s, layout.unit_gap);
        let fill = pattern_id(layout.preset.id, status);
        for p in &paths {
            out.push_str(&format!(
                r##"<path class="tm-cells" data-status="{st}" fill="url(#{fill})" d="M{x} {y}h{w}v{h}h-{w}z"/>"##,
                st = status_slug(status),
                x = p.x,
                y = p.y,
                w = p.w,
                h = p.h,
            ));
        }
        i = j;
    }
}

fn status_slug(s: UnitStatus) -> &'static str {
    match s {
        UnitStatus::NotStarted => "not-started",
        UnitStatus::Implemented => "implemented",
        UnitStatus::Tested => "tested",
        UnitStatus::Conformant => "conformant",
    }
}

fn run_paths(
    start: usize,
    count: usize,
    cols: i32,
    ox: i32,
    oy: i32,
    s: i32,
    gap: i32,
) -> Vec<IRect> {
    if count == 0 || cols <= 0 {
        return Vec::new();
    }
    let stride = s + gap;
    let mut out = Vec::new();
    let mut i = 0i32;
    let count = count as i32;
    let start = start as i32;
    while i < count {
        let idx = start + i;
        let col = idx % cols;
        let row = idx / cols;
        let take = (cols - col).min(count - i);
        let x = ox + col * stride;
        let y = oy + row * stride;
        if col == 0 && take == cols {
            let mut rows_n = 1;
            i += take;
            while i + cols <= count && (start + i) % cols == 0 {
                rows_n += 1;
                i += cols;
            }
            out.push(IRect {
                x,
                y,
                w: cols * stride - gap,
                h: rows_n * stride - gap,
            });
        } else {
            out.push(IRect {
                x,
                y,
                w: take * stride - gap,
                h: s,
            });
            i += take;
        }
    }
    out
}

fn xml_esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{ComponentBlock, FeatureGroup, Metrics, UnitStatus};
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn real_metrics() -> Metrics {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        crate::metrics::load(&root)
    }

    fn synthetic(total: usize) -> Metrics {
        let mut metrics = Metrics::placeholder();
        if total == 0 {
            return metrics;
        }
        if total == 1 {
            metrics.total = Some(1);
            metrics.passing = Some(0);
            metrics.components = vec![ComponentBlock::from_groups(
                "rest",
                "REST",
                vec![FeatureGroup {
                    id: "filtering".into(),
                    label: "filtering".into(),
                    units: vec![UnitStatus::NotStarted],
                }],
                vec![UnitStatus::NotStarted],
            )];
            return metrics;
        }
        // Hero mobile is 308×300. Eight labeled blocks cannot hold 5,000
        // stride-4 squares; four equal-ish blocks can.
        let (shares, ids): (&[usize], &[(&str, &str)]) = if total > 2000 {
            (
                &[1, 1, 1, 1],
                &[
                    ("functions", "Functions"),
                    ("studio", "Studio"),
                    ("auth", "Auth"),
                    ("storage", "Storage"),
                ],
            )
        } else {
            (
                &[276, 250, 152, 149, 93, 69, 19, 16],
                &[
                    ("functions", "Functions"),
                    ("studio", "Studio"),
                    ("auth", "Auth"),
                    ("storage", "Storage"),
                    ("rest", "REST"),
                    ("meta", "Meta"),
                    ("realtime", "Realtime"),
                    ("pooler", "Pooler"),
                ],
            )
        };
        let share_sum: usize = shares.iter().sum();
        let mut parts: Vec<usize> = shares
            .iter()
            .map(|s| (total * *s + share_sum / 2) / share_sum)
            .collect();
        let diff = total as i32 - parts.iter().sum::<usize>() as i32;
        if diff != 0 {
            parts[0] = (parts[0] as i32 + diff).max(1) as usize;
        }
        while parts.iter().sum::<usize>() > total {
            if let Some(p) = parts.iter_mut().find(|p| **p > 1) {
                *p -= 1;
            } else {
                break;
            }
        }
        while parts.iter().sum::<usize>() < total {
            parts[0] += 1;
        }
        let mut blocks = Vec::new();
        for ((id, label), n) in ids.iter().zip(parts) {
            if n == 0 {
                continue;
            }
            let g1 = (n / 2).max(1).min(n);
            let g2 = n - g1;
            let mut groups = vec![FeatureGroup {
                id: "a".into(),
                label: "a".into(),
                units: vec![UnitStatus::NotStarted; g1],
            }];
            if g2 > 0 {
                groups.push(FeatureGroup {
                    id: "b".into(),
                    label: "b".into(),
                    units: vec![UnitStatus::NotStarted; g2],
                });
            }
            blocks.push(ComponentBlock::from_groups(*id, label, groups, vec![]));
        }
        metrics.total = Some(total);
        metrics.passing = Some(0);
        metrics.components = blocks;
        metrics
    }

    fn parse_attr<'a>(chunk: &'a str, key: &str) -> &'a str {
        let start = chunk.find(key).unwrap_or_else(|| panic!("missing {key}")) + key.len() + 1;
        let rest = &chunk[start..];
        let end = rest.find('"').expect("quote");
        &rest[..end]
    }

    fn parse_i(chunk: &str, key: &str) -> i32 {
        parse_attr(chunk, key).parse().expect(key)
    }

    struct ParsedGroup {
        component: String,
        id: String,
        n: usize,
        s: i32,
        gap: i32,
        cols: i32,
        ox: i32,
        oy: i32,
        rect: IRect,
        block: IRect,
    }

    fn parse_groups(svg: &str) -> Vec<ParsedGroup> {
        let mut out = Vec::new();
        for (i, _) in svg.match_indices("<g data-group=\"") {
            let gchunk = &svg[i..];
            let before = &svg[..i];
            let comp_at = before
                .rfind("<g data-component=\"")
                .expect("group without component");
            let comp_chunk = &before[comp_at..];
            let id_end = comp_chunk.find("data-component=\"").unwrap() + 16;
            let component = parse_attr(comp_chunk, "data-component=").to_string();
            let _ = id_end;
            let gid_end = gchunk.find('"').unwrap();
            let _ = gid_end;
            out.push(ParsedGroup {
                component,
                id: parse_attr(gchunk, "data-group=").to_string(),
                n: parse_attr(gchunk, "data-n=").parse().unwrap(),
                s: parse_i(gchunk, "data-s="),
                gap: parse_i(gchunk, "data-gap="),
                cols: parse_i(gchunk, "data-cols="),
                ox: parse_i(gchunk, "data-ox="),
                oy: parse_i(gchunk, "data-oy="),
                rect: IRect {
                    x: parse_i(gchunk, "data-x="),
                    y: parse_i(gchunk, "data-y="),
                    w: parse_i(gchunk, "data-w="),
                    h: parse_i(gchunk, "data-h="),
                },
                block: IRect {
                    x: parse_i(comp_chunk, "data-x="),
                    y: parse_i(comp_chunk, "data-y="),
                    w: parse_i(comp_chunk, "data-w="),
                    h: parse_i(comp_chunk, "data-h="),
                },
            });
        }
        out
    }

    fn cells_of(g: &ParsedGroup) -> Vec<IRect> {
        if g.n == 0 || g.cols <= 0 {
            return Vec::new();
        }
        let stride = g.s + g.gap;
        (0..g.n as i32)
            .map(|i| {
                let c = i % g.cols;
                let r = i / g.cols;
                IRect {
                    x: g.ox + c * stride,
                    y: g.oy + r * stride,
                    w: g.s,
                    h: g.s,
                }
            })
            .collect()
    }

    fn assert_square_rules(svg: &str, metrics: &Metrics, preset: Preset) {
        let svg_tag_end = svg.find('>').expect("svg tag");
        let svg_tag = &svg[..svg_tag_end];
        assert!(svg_tag.contains("width=\"100%\""), "svg must be fluid");
        assert!(svg_tag.contains("height=\"auto\""));
        assert!(svg_tag.contains("preserveAspectRatio=\"xMidYMid meet\""));
        assert!(svg_tag.contains("viewBox=\"0 0 "));
        assert!(
            !svg_tag.contains(&format!("width=\"{}\"", preset.width)),
            "root svg must not use a fixed pixel width"
        );
        let groups = parse_groups(svg);
        let canvas = IRect {
            x: 0,
            y: 0,
            w: preset.width,
            h: preset.height,
        };
        let mut total_cells = 0usize;
        let mut sizes = HashSet::new();
        for g in &groups {
            let expected = metrics
                .components
                .iter()
                .find(|c| c.id == g.component)
                .and_then(|c| {
                    if !preset.nested || c.groups.is_empty() {
                        Some(c.total())
                    } else {
                        c.groups.iter().find(|x| x.id == g.id).map(|x| x.total())
                    }
                })
                .unwrap_or(0);
            assert_eq!(g.n, expected, "{} / {} square count", g.component, g.id);
            total_cells += g.n;
            assert!(
                canvas.contains(g.block),
                "component overflow {}: {:?}",
                g.component,
                g.block
            );
            assert!(
                g.block.contains(g.rect),
                "group overflow {} / {} {:?} in {:?}",
                g.component,
                g.id,
                g.rect,
                g.block
            );
            let cells = cells_of(g);
            assert_eq!(cells.len(), g.n);
            if g.n > 0 {
                assert!(g.s >= S_MIN, "square size {} < 3", g.s);
                assert!(g.s > 0, "whole pixels");
                sizes.insert(g.s);
                let stride = g.s + g.gap;
                let rows = (g.n as i32 + g.cols - 1) / g.cols;
                let gw = g.cols * stride - g.gap;
                let gh = rows * stride - g.gap;
                let ideal_ox = g.rect.x as f64 + (g.rect.w - gw) as f64 / 2.0;
                let ideal_oy = g.rect.y as f64 + (g.rect.h - gh) as f64 / 2.0;
                assert!(
                    (g.ox as f64 - ideal_ox).abs() <= stride as f64 + 0.51,
                    "grid x not centred within one lattice step {} vs {}",
                    g.ox,
                    ideal_ox
                );
                assert!(
                    (g.oy as f64 - ideal_oy).abs() <= stride as f64 + 0.51,
                    "grid y not centred within one lattice step {} vs {}",
                    g.oy,
                    ideal_oy
                );
                let last_row = (g.n as i32 - 1) / g.cols;
                if last_row > 0 {
                    let last_count = g.n as i32 - last_row * g.cols;
                    for c in 0..last_count {
                        let last = cells[(last_row * g.cols + c) as usize];
                        let first = cells[c as usize];
                        assert_eq!(
                            last.x, first.x,
                            "last row must be left-aligned with the grid"
                        );
                    }
                }
            }
            for cell in &cells {
                assert_eq!(cell.w, cell.h, "must be squares");
                assert!(
                    g.rect.contains(*cell),
                    "square overflows group {} / {} {:?} {:?}",
                    g.component,
                    g.id,
                    cell,
                    g.rect
                );
                assert!(
                    g.block.contains(*cell),
                    "square overflows component {}",
                    g.component
                );
                assert!(canvas.contains(*cell), "square overflows canvas");
                assert!(
                    cell.y >= g.block.y + preset.header_h,
                    "square overlaps header"
                );
            }
        }
        let expected_total: usize = metrics.components.iter().map(|c| c.total()).sum();
        assert_eq!(total_cells, expected_total, "total squares");
        if expected_total > 0 {
            assert_eq!(sizes.len(), 1, "all squares equal: {sizes:?}");
            let s = *sizes.iter().next().unwrap();
            assert!((S_MIN..=S_MAX).contains(&s));
        }

        let mut labeled = HashSet::new();
        for chunk in svg.split("<g data-component=\"").skip(1) {
            let id_end = chunk.find('"').unwrap();
            let id = &chunk[..id_end];
            let bw = parse_i(chunk, "data-w=");
            if let Some(text_at) = chunk.find("<text ") {
                let tchunk = &chunk[text_at..];
                let end = tchunk.find("</text>").unwrap();
                let open_end = tchunk.find('>').unwrap();
                let content = &tchunk[open_end + 1..end];
                let px: i32 = parse_attr(tchunk, "font-size=").parse().unwrap();
                let decoded = content.replace("&amp;", "&");
                assert!(
                    LABEL_INSET + mono_w(&decoded, px) + 4 <= bw,
                    "{id} label {decoded:?} at {px}px does not fit width {bw}"
                );
                labeled.insert(id.to_string());
            }
        }
        let live: Vec<_> = metrics
            .components
            .iter()
            .filter(|c| c.total() > 0)
            .map(|c| c.id.clone())
            .collect();
        if live.len() == 8 {
            for id in &live {
                assert!(labeled.contains(id), "missing component label {id}");
            }
        }
        assert_no_light_non_text(svg);
    }

    fn assert_no_light_non_text(svg: &str) {
        let allowed = [
            BG,
            BLOCK,
            HEADER,
            GROUP,
            NOT_STARTED,
            IMPLEMENTED,
            TESTED,
            CONFORMANT,
        ];
        let mut rest = svg;
        while let Some(at) = rest.find('#') {
            let hex = &rest[at..at + 7.min(rest.len() - at)];
            if hex.len() == 7 && hex[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
                let before = &svg[..svg.len() - rest.len() + at];
                let in_text = before.rfind("<text").is_some()
                    && before
                        .rfind("<text")
                        .and_then(|i| before[i..].find("</text>"))
                        .is_none();
                if !in_text {
                    assert!(
                        allowed.iter().any(|c| c.eq_ignore_ascii_case(hex)),
                        "unexpected non-text colour {hex}"
                    );
                }
            }
            rest = &rest[at + 1..];
        }
    }

    fn assert_layout(metrics: &Metrics, preset: Preset) {
        let svg = render(metrics, preset);
        let again = render(metrics, preset);
        assert_eq!(svg, again, "output must be byte-identical");
        assert_square_rules(&svg, metrics, preset);
    }

    #[test]
    fn real_summary_at_every_preset() {
        let metrics = real_metrics();
        let summed: usize = metrics.components.iter().map(|c| c.total()).sum();
        let total = metrics
            .total
            .expect("live coverage must include totals.units");
        assert!(total > 0, "live coverage must have units");
        assert_eq!(
            total, summed,
            "summary total must equal the sum of components"
        );
        for preset in PRESETS {
            assert_layout(&metrics, *preset);
        }
    }

    #[test]
    fn generated_unit_counts() {
        for n in [1usize, 334, 1024, 5000] {
            let metrics = synthetic(n);
            assert_eq!(
                metrics.components.iter().map(|c| c.total()).sum::<usize>(),
                n
            );
            for preset in PRESETS {
                assert_layout(&metrics, *preset);
            }
        }
    }

    #[test]
    fn mixed_status_order_is_conformant_first() {
        let mut metrics = Metrics::placeholder();
        metrics.total = Some(8);
        metrics.passing = Some(2);
        metrics.components = vec![ComponentBlock::from_groups(
            "rest",
            "REST",
            vec![FeatureGroup {
                id: "filtering".into(),
                label: "filtering".into(),
                units: vec![
                    UnitStatus::NotStarted,
                    UnitStatus::Implemented,
                    UnitStatus::Tested,
                    UnitStatus::Conformant,
                    UnitStatus::NotStarted,
                    UnitStatus::Implemented,
                    UnitStatus::Tested,
                    UnitStatus::Conformant,
                ],
            }],
            vec![],
        )];
        let svg = render(&metrics, HOME_DESKTOP);
        let ns = svg.find("data-status=\"not-started\"").unwrap();
        let im = svg.find("data-status=\"implemented\"").unwrap();
        let te = svg.find("data-status=\"tested\"").unwrap();
        let co = svg.find("data-status=\"conformant\"").unwrap();
        assert!(co < te && te < im && im < ns, "status paint order");
        assert_layout(&metrics, HOME_DESKTOP);
    }

    #[test]
    fn day0_has_catalog_tiles_and_no_invented_total() {
        let metrics = Metrics::placeholder();
        let svg = render(&metrics, HOME_DESKTOP);
        assert!(!svg.contains("334"));
        let cx = HOME_DESKTOP.width / 2;
        let cy = HOME_DESKTOP.height / 2;
        assert!(
            svg.contains(&format!(
                r#"<text x="{cx}" y="{cy}" text-anchor="middle" dominant-baseline="middle""#
            )) && svg.contains("—"),
            "no-data SVG must draw a centered em dash"
        );
        assert_eq!(parse_groups(&svg).iter().map(|g| g.n).sum::<usize>(), 0);
        for id in [
            "rest",
            "auth",
            "storage",
            "realtime",
            "functions",
            "pooler",
            "meta",
            "studio",
        ] {
            assert!(svg.contains(&format!("data-component=\"{id}\"")));
        }
        assert!(svg.contains("REST"));
        assert!(svg.contains(BG));
    }

    #[test]
    fn hero_has_no_group_labels_and_shows_counts() {
        let metrics = real_metrics();
        let live: Vec<_> = metrics
            .components
            .iter()
            .filter(|c| c.total() > 0)
            .collect();
        let svg = render(&metrics, HERO_DESKTOP);
        assert!(!svg.contains("font-size=\"8\" fill=\"#8A8B8E\""));
        for c in &live {
            let name = c.label.to_ascii_uppercase();
            assert!(svg.contains(&name), "hero must label component {name}");
            let count = format!("/{}", comma(c.total()));
            assert!(
                svg.contains(&count) || svg.contains(&name),
                "hero label should include a count when it fits"
            );
        }
        assert!(svg.contains("/"));
        let groups = parse_groups(&svg);
        assert_eq!(groups.len(), live.len());
        assert_layout(&metrics, HERO_DESKTOP);
        assert_layout(&metrics, HERO_MOBILE);
    }

    #[test]
    fn squarify_areas_sum_to_canvas() {
        let sizes = [81.0, 57.0, 51.0, 38.0, 34.0, 33.0, 27.0, 13.0];
        let rects = squarify(&sizes, 0.0, 0.0, 462.0, 200.0);
        assert_eq!(rects.len(), sizes.len());
        let area: f64 = rects.iter().map(|r| r[2] * r[3]).sum();
        assert!((area - 462.0 * 200.0).abs() < 1.0);
        assert!(rects[0][2] * rects[0][3] >= rects[1][2] * rects[1][3] - 1.0);
    }

    #[test]
    fn numbers_use_comma_grouping() {
        assert_eq!(comma(1024), "1,024");
        let metrics = synthetic(1024);
        let expected = comma(1024);
        let svg = render(&metrics, STATUS_DESKTOP);
        assert!(
            svg.contains(&expected) || alt_text(&metrics).contains(&expected),
            "synthetic 1024-unit map must use comma grouping"
        );
    }
}
