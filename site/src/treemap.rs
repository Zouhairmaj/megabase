use crate::metrics::{
    comma, CatalogRow, ComponentBlock, FeatureGroup, Metrics, UnitStatus, CATALOG,
};

const BG: &str = "#0B0E12";
const NOT_STARTED: &str = "#2A2C2F";
const GROUP_STROKE: &str = "#303235";
const IMPLEMENTED: &str = "#005441";
const TESTED: &str = "#009366";
const CONFORMANT: &str = "#00D892";
const LABEL: &str = "#BABABB";
const BLOCK_GAP: f64 = 4.0;
const BLOCK_PAD: f64 = 4.0;
const UNIT_GAP: f64 = 1.5;
const LABEL_H: f64 = 14.0;
const MIN_CELL: f64 = 2.0;

pub fn svg(metrics: &Metrics) -> String {
    svg_size(metrics, 462.0, 200.0)
}

pub fn svg_size(metrics: &Metrics, width: f64, height: f64) -> String {
    let blocks: Vec<&ComponentBlock> = metrics
        .components
        .iter()
        .filter(|c| c.total() > 0)
        .collect();

    let label = if blocks.is_empty() {
        "Experiment treemap: no coverage data yet.".to_string()
    } else {
        format!(
            "Experiment treemap: {} of {} units. One whole square per unit, grouped by component.",
            metrics.passing.map(comma).unwrap_or_else(|| "—".into()),
            metrics.total.map(comma).unwrap_or_else(|| "—".into())
        )
    };

    let mut out = String::new();
    out.push_str(&format!(
        r##"<svg class="treemap-svg" viewBox="0 0 {width} {height}" width="100%" height="100%" role="img" aria-label="{label}" xmlns="http://www.w3.org/2000/svg"><title>{label}</title><rect width="{width}" height="{height}" fill="{BG}"/>"##
    ));

    if blocks.is_empty() {
        paint_day0_catalog(&mut out, width, height);
        out.push_str("</svg>");
        return out;
    }

    let weights: Vec<f64> = blocks.iter().map(|c| c.total() as f64).collect();
    let cells = squarify(&weights, 0.0, 0.0, width, height);
    let packed: Vec<(&ComponentBlock, [f64; 4])> = blocks
        .iter()
        .copied()
        .zip(cells)
        .map(|(block, rect)| {
            let (x, y, w, h) = inset(rect[0], rect[1], rect[2], rect[3], BLOCK_GAP / 2.0);
            (block, [x, y, w, h])
        })
        .filter(|(_, r)| r[2] > 1.0 && r[3] > 1.0)
        .collect();

    let mut cell = global_cell(&packed);
    while cell > MIN_CELL && !grids_fit(&packed, cell) {
        cell = (cell - 0.05).max(MIN_CELL);
        cell = (cell * 100.0).floor() / 100.0;
    }
    for (block, [x, y, w, h]) in packed {
        paint_block(&mut out, block, x, y, w, h, cell);
    }

    out.push_str("</svg>");
    out
}

fn grids_fit(packed: &[(&ComponentBlock, [f64; 4])], cell: f64) -> bool {
    packed.iter().all(|(block, [_, _, w, h])| {
        let (inner_w, inner_h, _) = inner_area(*w, *h);
        if block.groups.len() > 1 {
            let weights: Vec<f64> = block.groups.iter().map(|g| g.total() as f64).collect();
            let cells = squarify(&weights, 0.0, 0.0, inner_w, inner_h);
            block.groups.iter().zip(cells).all(|(group, rect)| {
                let (gw, gh) = (rect[2], rect[3]);
                let pad = 1.0;
                let (cols, rows) = grid_for(
                    group.total(),
                    (gw - pad).max(0.0),
                    (gh - pad).max(0.0),
                    cell,
                );
                let grid_w = cols as f64 * cell + (cols.saturating_sub(1) as f64) * UNIT_GAP;
                let grid_h = rows as f64 * cell + (rows.saturating_sub(1) as f64) * UNIT_GAP;
                grid_w <= gw + 0.05 && grid_h <= gh + 0.05
            })
        } else {
            let (cols, rows) = grid_for(block.total(), inner_w, inner_h, cell);
            let grid_w = cols as f64 * cell + (cols.saturating_sub(1) as f64) * UNIT_GAP;
            let grid_h = rows as f64 * cell + (rows.saturating_sub(1) as f64) * UNIT_GAP;
            grid_w <= inner_w + 0.01 && grid_h <= inner_h + 0.01
        }
    })
}

fn global_cell(packed: &[(&ComponentBlock, [f64; 4])]) -> f64 {
    let mut cell = f64::MAX;
    for (block, [_, _, w, h]) in packed {
        let (inner_w, inner_h, _) = inner_area(*w, *h);
        if block.groups.len() > 1 {
            let weights: Vec<f64> = block.groups.iter().map(|g| g.total() as f64).collect();
            let cells = squarify(&weights, 0.0, 0.0, inner_w, inner_h);
            for (group, rect) in block.groups.iter().zip(cells) {
                cell = cell.min(max_cell(group.total(), rect[2].max(1.0), rect[3].max(1.0)));
            }
        } else {
            cell = cell.min(max_cell(block.total(), inner_w, inner_h));
        }
    }
    if !cell.is_finite() || cell < MIN_CELL {
        MIN_CELL
    } else {
        (cell * 100.0).floor() / 100.0
    }
}

fn inner_area(w: f64, h: f64) -> (f64, f64, bool) {
    let inner_w = (w - BLOCK_PAD * 2.0).max(0.0);
    let labeled = h >= LABEL_H + MIN_CELL + BLOCK_PAD * 2.0 + 4.0;
    let inner_h = if labeled {
        (h - BLOCK_PAD * 2.0 - LABEL_H).max(0.0)
    } else {
        (h - BLOCK_PAD * 2.0).max(0.0)
    };
    (inner_w, inner_h, labeled)
}

fn max_cell(n: usize, w: f64, h: f64) -> f64 {
    if n == 0 || w <= 0.0 || h <= 0.0 {
        return 0.0;
    }
    let mut best = 0.0;
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let cell_w = (w - UNIT_GAP * (cols as f64 - 1.0)) / cols as f64;
        let cell_h = (h - UNIT_GAP * (rows as f64 - 1.0)) / rows as f64;
        let cell = cell_w.min(cell_h);
        if cell > best {
            best = cell;
        }
    }
    best
}

fn grid_for(n: usize, w: f64, h: f64, cell: f64) -> (usize, usize) {
    let max_cols = (((w + UNIT_GAP) / (cell + UNIT_GAP)).floor() as usize).clamp(1, n);
    let max_rows = (((h + UNIT_GAP) / (cell + UNIT_GAP)).floor() as usize).max(1);
    let mut best = (1usize, n);
    let mut best_score = f64::MAX;
    for cols in 1..=max_cols {
        let rows = n.div_ceil(cols);
        if rows > max_rows {
            continue;
        }
        let used_w = cols as f64 * cell + (cols.saturating_sub(1) as f64) * UNIT_GAP;
        let used_h = rows as f64 * cell + (rows.saturating_sub(1) as f64) * UNIT_GAP;
        if used_w > w + 0.01 || used_h > h + 0.01 {
            continue;
        }
        let aspect = (used_w / used_h.max(0.001) - w / h.max(0.001)).abs();
        let leftover = (cols * rows - n) as f64;
        let score = leftover * 2.0 + aspect;
        if score < best_score {
            best_score = score;
            best = (cols, rows);
        }
    }
    best
}

/// Day-0 map: one labelled grey tile per known component, equal area.
/// No unit cells and no invented denominator — those arrive with coverage/.
fn paint_day0_catalog(out: &mut String, width: f64, height: f64) {
    let weights: Vec<f64> = CATALOG.iter().map(|_| 1.0).collect();
    let cells = squarify(&weights, 0.0, 0.0, width, height);
    for (row, rect) in CATALOG.iter().zip(cells) {
        let (x, y, w, h) = inset(rect[0], rect[1], rect[2], rect[3], BLOCK_GAP / 2.0);
        if w < 2.0 || h < 2.0 {
            continue;
        }
        out.push_str(&format!(
            r##"<g data-component="{id}" data-x="{x:.2}" data-y="{y:.2}" data-w="{w:.2}" data-h="{h:.2}"><rect x="{x:.2}" y="{y:.2}" width="{w:.2}" height="{h:.2}" fill="{NOT_STARTED}"/><text x="{tx:.2}" y="{ty:.2}" text-anchor="middle" dominant-baseline="middle" font-family="JetBrains Mono, ui-monospace, monospace" font-size="11" font-weight="700" fill="{LABEL}">{label}</text></g>"##,
            id = xml_esc(&row.id),
            tx = x + w / 2.0,
            ty = y + h / 2.0,
            label = xml_esc(day0_label(row)),
        ));
    }
}

fn day0_label(row: &CatalogRow) -> &'static str {
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
}

fn paint_block(
    out: &mut String,
    block: &ComponentBlock,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    cell: f64,
) {
    let n = block.total();
    if n == 0 {
        return;
    }
    let (inner_w, inner_h, labeled) = inner_area(w, h);
    let origin_x = x + BLOCK_PAD;
    let origin_y = y + BLOCK_PAD + if labeled { LABEL_H } else { 0.0 };

    out.push_str(&format!(
        r##"<g data-component="{id}" data-x="{x:.2}" data-y="{y:.2}" data-w="{w:.2}" data-h="{h:.2}">"##,
        id = xml_esc(&block.id)
    ));

    if labeled && w >= 36.0 {
        out.push_str(&format!(
            r##"<text x="{tx:.2}" y="{ty:.2}" text-anchor="middle" dominant-baseline="middle" font-family="JetBrains Mono, ui-monospace, monospace" font-size="10" font-weight="700" fill="{LABEL}">{name}</text>"##,
            tx = x + w / 2.0,
            ty = y + BLOCK_PAD + LABEL_H / 2.0,
            name = xml_esc(block.label)
        ));
    }

    if block.groups.len() > 1 {
        paint_groups(
            out,
            &block.groups,
            origin_x,
            origin_y,
            inner_w,
            inner_h,
            cell,
        );
    } else {
        let units = block.unit_statuses();
        paint_unit_grid(out, &units, origin_x, origin_y, inner_w, inner_h, cell);
    }
    out.push_str("</g>");
}

fn paint_groups(
    out: &mut String,
    groups: &[FeatureGroup],
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    cell: f64,
) {
    let weights: Vec<f64> = groups.iter().map(|g| g.total() as f64).collect();
    let cells = squarify(&weights, x, y, w, h);
    for (group, rect) in groups.iter().zip(cells) {
        let (gx, gy, gw, gh) = inset(rect[0], rect[1], rect[2], rect[3], 0.75);
        if gw < 1.0 || gh < 1.0 {
            continue;
        }
        out.push_str(&format!(
            r##"<rect data-group="{id}" x="{gx:.2}" y="{gy:.2}" width="{gw:.2}" height="{gh:.2}" fill="none" stroke="{GROUP_STROKE}" stroke-width="0.5"/>"##,
            id = xml_esc(&group.id)
        ));
        paint_unit_grid(out, &group.units, gx, gy, gw, gh, cell);
    }
}

fn paint_unit_grid(
    out: &mut String,
    units: &[UnitStatus],
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    cell: f64,
) {
    let n = units.len();
    if n == 0 {
        return;
    }
    let (cols, rows) = grid_for(n, w, h, cell);
    let grid_w = cols as f64 * cell + (cols.saturating_sub(1) as f64) * UNIT_GAP;
    let grid_h = rows as f64 * cell + (rows.saturating_sub(1) as f64) * UNIT_GAP;
    let origin_x = x + ((w - grid_w) / 2.0).max(0.0);
    let origin_y = y + ((h - grid_h) / 2.0).max(0.0);
    for (i, status) in units.iter().enumerate() {
        let c = i % cols;
        let r = i / cols;
        let sx = origin_x + c as f64 * (cell + UNIT_GAP);
        let sy = origin_y + r as f64 * (cell + UNIT_GAP);
        let fill = status_color(*status);
        out.push_str(&format!(
            r##"<rect class="unit" x="{sx:.2}" y="{sy:.2}" width="{cell:.2}" height="{cell:.2}" fill="{fill}"/>"##
        ));
    }
}

fn status_color(status: UnitStatus) -> &'static str {
    match status {
        UnitStatus::NotStarted => NOT_STARTED,
        UnitStatus::Implemented => IMPLEMENTED,
        UnitStatus::Tested => TESTED,
        UnitStatus::Conformant => CONFORMANT,
    }
}

fn inset(x: f64, y: f64, w: f64, h: f64, pad: f64) -> (f64, f64, f64, f64) {
    (
        x + pad,
        y + pad,
        (w - pad * 2.0).max(0.0),
        (h - pad * 2.0).max(0.0),
    )
}

fn xml_esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Squarified treemap. `sizes` are relative weights; result rects are `[x,y,w,h]`.
fn squarify(sizes: &[f64], x: f64, y: f64, dx: f64, dy: f64) -> Vec<[f64; 4]> {
    let total: f64 = sizes.iter().sum();
    let area = dx * dy;
    if sizes.is_empty() || total <= 0.0 || area <= 0.0 {
        return Vec::new();
    }
    let mut remaining: Vec<f64> = sizes.iter().map(|s| s / total * area).collect();
    let mut x = x;
    let mut y = y;
    let mut dx = dx;
    let mut dy = dy;
    let mut out = Vec::with_capacity(sizes.len());

    while !remaining.is_empty() {
        let side = if dx >= dy { dy } else { dx };
        if side <= 0.0 {
            break;
        }
        let mut row = vec![remaining.remove(0)];
        while !remaining.is_empty() {
            let mut trial = row.clone();
            trial.push(remaining[0]);
            if worst_aspect(&row, side) >= worst_aspect(&trial, side) {
                row.push(remaining.remove(0));
            } else {
                break;
            }
        }
        let row_area: f64 = row.iter().sum();
        if dx >= dy {
            let width = (row_area / dy).min(dx);
            let mut yy = y;
            for a in &row {
                let h = if width > 0.0 { a / width } else { 0.0 };
                out.push([x, yy, width, h]);
                yy += h;
            }
            x += width;
            dx -= width;
        } else {
            let height = (row_area / dx).min(dy);
            let mut xx = x;
            for a in &row {
                let w = if height > 0.0 { a / height } else { 0.0 };
                out.push([xx, y, w, height]);
                xx += w;
            }
            y += height;
            dy -= height;
        }
    }
    out
}

fn worst_aspect(row: &[f64], side: f64) -> f64 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{ComponentBlock, Metrics, UnitStatus};
    use std::collections::HashSet;

    fn sample_metrics() -> Metrics {
        let mut metrics = Metrics::placeholder();
        metrics.total = Some(10);
        metrics.passing = Some(2);
        metrics.components = vec![
            ComponentBlock::from_units(
                "rest",
                "REST",
                vec![
                    UnitStatus::NotStarted,
                    UnitStatus::NotStarted,
                    UnitStatus::NotStarted,
                    UnitStatus::NotStarted,
                ],
            ),
            ComponentBlock::from_units(
                "auth",
                "Auth",
                vec![
                    UnitStatus::NotStarted,
                    UnitStatus::Implemented,
                    UnitStatus::Tested,
                    UnitStatus::Conformant,
                    UnitStatus::Conformant,
                    UnitStatus::Implemented,
                ],
            ),
        ];
        metrics
    }

    #[test]
    fn day0_paints_catalog_tiles_without_inventing_units() {
        let metrics = Metrics::placeholder();
        let svg = super::svg(&metrics);
        assert!(!svg.contains("334"));
        assert!(!svg.contains("1024"));
        assert_eq!(parse_units(&svg).len(), 0, "no fake unit cells");
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
            assert!(
                svg.contains(&format!("data-component=\"{id}\"")),
                "missing {id}"
            );
        }
        assert!(svg.contains("REST"));
        assert!(svg.contains("#2A2C2F"));
    }

    #[test]
    fn areas_sum_to_canvas() {
        let sizes = [81.0, 57.0, 51.0, 38.0, 34.0, 33.0, 27.0, 13.0];
        let rects = squarify(&sizes, 0.0, 0.0, 462.0, 200.0);
        assert_eq!(rects.len(), sizes.len());
        let area: f64 = rects.iter().map(|r| r[2] * r[3]).sum();
        assert!((area - 462.0 * 200.0).abs() < 1.0);
    }

    #[test]
    fn phase0_334_units_all_drawn_equal_and_inside_viewbox() {
        let counts = [
            ("rest", "REST", 57),
            ("auth", "Auth", 81),
            ("realtime", "Realtime", 33),
            ("storage", "Storage", 34),
            ("functions", "Functions", 27),
            ("pooler", "Pooler", 13),
            ("meta", "Meta", 51),
            ("studio", "Studio", 38),
        ];
        let mut metrics = Metrics::placeholder();
        metrics.total = Some(334);
        metrics.passing = Some(0);
        metrics.components = counts
            .iter()
            .map(|(id, label, n)| {
                ComponentBlock::from_units(*id, label, vec![UnitStatus::NotStarted; *n])
            })
            .collect();
        let svg = super::svg(&metrics);
        let units = parse_units(&svg);
        assert_eq!(units.len(), 334);
        let sizes: HashSet<(i64, i64)> = units
            .iter()
            .map(|u| ((u.2 * 100.0).round() as i64, (u.3 * 100.0).round() as i64))
            .collect();
        assert_eq!(sizes.len(), 1, "all squares same size: {sizes:?}");
        assert!((units[0].2 - units[0].3).abs() < 0.011, "must be squares");
        for u in &units {
            assert!(u.0 >= -0.05 && u.1 >= -0.05, "clipped left/top {u:?}");
            assert!(u.0 + u.2 <= 462.05, "clipped right {u:?}");
            assert!(u.1 + u.3 <= 200.05, "clipped bottom {u:?}");
        }
    }

    #[test]
    fn one_equal_unclipped_square_per_unit() {
        let metrics = sample_metrics();
        let svg = super::svg(&metrics);
        let units = parse_units(&svg);
        assert_eq!(units.len(), 10, "one square per unit");
        let sizes: HashSet<(i64, i64)> = units
            .iter()
            .map(|u| ((u.2 * 100.0).round() as i64, (u.3 * 100.0).round() as i64))
            .collect();
        assert_eq!(sizes.len(), 1, "all unit squares the same size: {sizes:?}");
        assert!(units[0].2 > 0.0 && units[0].2 == units[0].3, "squares");
        for u in &units {
            assert!(u.0 >= -0.05 && u.1 >= -0.05);
            assert!(u.0 + u.2 <= 462.05);
            assert!(u.1 + u.3 <= 200.05);
        }
        assert!(svg.contains("data-component=\"rest\""));
        assert!(svg.contains("data-component=\"auth\""));
        assert!(svg.contains("#0B0E12"));
        assert!(svg.contains("#2A2C2F"));
        assert!(svg.contains("#005441"));
        assert!(svg.contains("#009366"));
        assert!(svg.contains("#00D892"));
        assert!(!svg.contains("1024"));
    }

    #[derive(Debug)]
    struct UnitBox(f64, f64, f64, f64);

    fn parse_units(svg: &str) -> Vec<UnitBox> {
        let mut out = Vec::new();
        for chunk in svg.split("<rect ").skip(1) {
            if !chunk.contains("class=\"unit\"") {
                continue;
            }
            let x = attr(chunk, "x=");
            let y = attr(chunk, "y=");
            let w = attr(chunk, "width=");
            let h = attr(chunk, "height=");
            out.push(UnitBox(x, y, w, h));
        }
        out
    }

    fn attr(chunk: &str, key: &str) -> f64 {
        let start = chunk.find(key).expect(key) + key.len() + 1;
        let rest = &chunk[start..];
        let end = rest.find('"').expect("quote");
        rest[..end].parse().expect("number")
    }
}
