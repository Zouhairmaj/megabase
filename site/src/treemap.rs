use crate::metrics::{comma, ComponentBlock, Metrics};

const BG: &str = "#0B0E12";
const NOT_STARTED: &str = "#303235";
const IMPLEMENTED: &str = "#005441";
const TESTED: &str = "#009366";
const CONFORMANT: &str = "#00D892";
const LABEL: &str = "#F7F7F7";
const LABEL_ON_ACCENT: &str = "#0B0E12";
const GAP: f64 = 3.0;

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
            "Experiment treemap: {} of {} units passing the judge. One block per component.",
            metrics.passing.map(comma).unwrap_or_else(|| "—".into()),
            metrics.total.map(comma).unwrap_or_else(|| "—".into())
        )
    };

    let mut out = String::new();
    out.push_str(&format!(
        r##"<svg class="treemap-svg" viewBox="0 0 {width} {height}" width="100%" height="100%" role="img" aria-label="{label}" xmlns="http://www.w3.org/2000/svg"><title>{label}</title><rect width="{width}" height="{height}" fill="{BG}"/>"##
    ));

    if blocks.is_empty() {
        out.push_str(&format!(
            r##"<text x="{cx}" y="{cy}" text-anchor="middle" dominant-baseline="middle" font-family="JetBrains Mono, ui-monospace, monospace" font-size="18" fill="#8E9094">—</text>"##,
            cx = width / 2.0,
            cy = height / 2.0
        ));
        out.push_str("</svg>");
        return out;
    }

    let weights: Vec<f64> = blocks.iter().map(|c| c.total() as f64).collect();
    let cells = squarify(&weights, 0.0, 0.0, width, height);
    for (block, [x, y, w, h]) in blocks.iter().zip(cells.into_iter()) {
        let (x, y, w, h) = inset(x, y, w, h, GAP / 2.0);
        if w <= 1.0 || h <= 1.0 {
            continue;
        }
        paint_block(&mut out, block, x, y, w, h);
    }

    out.push_str("</svg>");
    out
}

fn paint_block(out: &mut String, block: &ComponentBlock, x: f64, y: f64, w: f64, h: f64) {
    let total = block.total() as f64;
    let layers = [
        (block.conformant, CONFORMANT),
        (block.tested, TESTED),
        (block.implemented, IMPLEMENTED),
        (block.not_started, NOT_STARTED),
    ];
    let present: Vec<(usize, &str)> = layers.into_iter().filter(|(n, _)| *n > 0).collect();
    if present.is_empty() {
        return;
    }

    if present.len() == 1 {
        let color = present[0].1;
        out.push_str(&format!(
            r##"<g data-component="{id}"><rect x="{x:.2}" y="{y:.2}" width="{w:.2}" height="{h:.2}" fill="{color}"/>"##,
            id = xml_esc(&block.id)
        ));
        label_block(out, block, x, y, w, h, color);
        out.push_str("</g>");
        return;
    }

    // Mixed status: stack from the bottom (conformant → not started).
    out.push_str(&format!(
        r##"<g data-component="{id}">"##,
        id = xml_esc(&block.id)
    ));
    let mut y_cursor = y + h;
    for (n, color) in present {
        let hh = h * (n as f64 / total);
        y_cursor -= hh;
        out.push_str(&format!(
            r##"<rect x="{x:.2}" y="{y_cursor:.2}" width="{w:.2}" height="{hh:.2}" fill="{color}"/>"##
        ));
    }
    label_block(out, block, x, y, w, h, NOT_STARTED);
    out.push_str("</g>");
}

fn label_block(
    out: &mut String,
    block: &ComponentBlock,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    fill: &str,
) {
    if w < 40.0 || h < 18.0 {
        return;
    }
    let fill_color = if fill == CONFORMANT {
        LABEL_ON_ACCENT
    } else {
        LABEL
    };
    let cx = x + w / 2.0;
    let two_line = h >= 34.0 && w >= 52.0;
    let name_size = if w >= 90.0 { 12.0 } else { 10.0 };
    let name_y = if two_line {
        y + h / 2.0 - 7.0
    } else {
        y + h / 2.0
    };
    out.push_str(&format!(
        r##"<text x="{cx:.2}" y="{name_y:.2}" text-anchor="middle" dominant-baseline="middle" font-family="JetBrains Mono, ui-monospace, monospace" font-size="{name_size}" font-weight="700" fill="{fill_color}">{name}</text>"##,
        name = xml_esc(block.label)
    ));
    if two_line {
        let count_y = y + h / 2.0 + 9.0;
        out.push_str(&format!(
            r##"<text x="{cx:.2}" y="{count_y:.2}" text-anchor="middle" dominant-baseline="middle" font-family="JetBrains Mono, ui-monospace, monospace" font-size="10" fill="#BABABB">{count}</text>"##,
            count = comma(block.total())
        ));
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
    use super::squarify;

    #[test]
    fn areas_sum_to_canvas() {
        let sizes = [81.0, 57.0, 51.0, 38.0, 34.0, 33.0, 27.0, 13.0];
        let rects = squarify(&sizes, 0.0, 0.0, 462.0, 200.0);
        assert_eq!(rects.len(), sizes.len());
        let area: f64 = rects.iter().map(|r| r[2] * r[3]).sum();
        assert!((area - 462.0 * 200.0).abs() < 1.0);
    }

    #[test]
    fn brand_colors_only_and_one_block_per_component() {
        use crate::metrics::{ComponentBlock, Metrics};

        let mut metrics = Metrics::placeholder();
        metrics.total = Some(10);
        metrics.passing = Some(2);
        metrics.components = vec![
            ComponentBlock {
                id: "rest".into(),
                label: "REST",
                not_started: 4,
                implemented: 0,
                tested: 0,
                conformant: 0,
            },
            ComponentBlock {
                id: "auth".into(),
                label: "Auth",
                not_started: 1,
                implemented: 1,
                tested: 1,
                conformant: 2,
            },
        ];
        let svg = super::svg(&metrics);
        assert!(svg.contains("data-component=\"rest\""));
        assert!(svg.contains("data-component=\"auth\""));
        assert!(svg.contains("#0B0E12"));
        assert!(svg.contains("#303235"));
        assert!(svg.contains("#005441"));
        assert!(svg.contains("#009366"));
        assert!(svg.contains("#00D892"));
        assert!(!svg.contains("#22c55e"));
        assert!(!svg.contains("#eab308"));
        assert!(!svg.contains("1024"));
    }
}
