#!/usr/bin/env python3
"""
Generate treemap SVGs from coverage/units.json.

Creates visual representations of implementation coverage per component,
similar to the anyps5 project's status display. Each unit is a square,
colored by status:
- Green (#22c55e): Implemented and conformant
- Yellow (#eab308): Implemented but not yet conformant
- Grey (#6b7280): Not implemented

Licensed under Apache-2.0
"""

import json
import math
from pathlib import Path
from typing import Any

COVERAGE_DIR = Path(__file__).parent.parent / "coverage"
UNITS_FILE = COVERAGE_DIR / "units.json"

# Colors
COLOR_DONE = "#22c55e"      # Green - implemented and conformant
COLOR_PARTIAL = "#eab308"   # Yellow - implemented but not conformant
COLOR_TODO = "#6b7280"      # Grey - not implemented
COLOR_TEXT = "#ffffff"      # White text
COLOR_TEXT_DARK = "#1f2937" # Dark text for light backgrounds
COLOR_BG = "#111827"        # Dark background


def load_units() -> dict[str, Any]:
    """Load units from JSON file."""
    with open(UNITS_FILE) as f:
        return json.load(f)


def get_unit_color(unit: dict) -> str:
    """Get color for a unit based on its status."""
    status = unit.get("status", "not_implemented")
    if status == "conformant":
        return COLOR_DONE
    elif status == "implemented":
        return COLOR_PARTIAL
    return COLOR_TODO


def squarify(values: list[float], x: float, y: float, width: float, height: float) -> list[dict]:
    """
    Simple squarified treemap layout algorithm.
    Returns list of rectangles: {x, y, width, height, index}
    """
    if not values:
        return []
    
    total = sum(values)
    if total == 0:
        return []
    
    rects = []
    remaining = list(enumerate(values))
    
    def layout_row(items: list, row_x: float, row_y: float, row_width: float, row_height: float, horizontal: bool):
        """Layout a single row of items."""
        if not items:
            return []
        
        row_rects = []
        row_total = sum(v for _, v in items)
        offset = 0
        
        for idx, val in items:
            ratio = val / row_total if row_total > 0 else 0
            if horizontal:
                rect_width = row_width * ratio
                row_rects.append({
                    "x": row_x + offset,
                    "y": row_y,
                    "width": rect_width,
                    "height": row_height,
                    "index": idx
                })
                offset += rect_width
            else:
                rect_height = row_height * ratio
                row_rects.append({
                    "x": row_x,
                    "y": row_y + offset,
                    "width": row_width,
                    "height": rect_height,
                    "index": idx
                })
                offset += rect_height
        
        return row_rects
    
    # Simple row-based layout
    curr_x, curr_y = x, y
    curr_width, curr_height = width, height
    
    while remaining:
        horizontal = curr_width >= curr_height
        
        # Take items for this row
        row_items = []
        row_total = 0
        target = total * (0.4 if len(remaining) > 4 else 1.0)
        
        for item in remaining[:]:
            if row_total < target or len(row_items) < 1:
                row_items.append(item)
                row_total += item[1]
                remaining.remove(item)
            else:
                break
        
        if not row_items:
            break
        
        # Calculate row size
        row_ratio = row_total / total if total > 0 else 0
        
        if horizontal:
            row_height = curr_height * row_ratio
            row_rects = layout_row(row_items, curr_x, curr_y, curr_width, row_height, True)
            curr_y += row_height
            curr_height -= row_height
        else:
            row_width = curr_width * row_ratio
            row_rects = layout_row(row_items, curr_x, curr_y, row_width, curr_height, False)
            curr_x += row_width
            curr_width -= row_width
        
        rects.extend(row_rects)
        total -= row_total
    
    return rects


def generate_component_treemap(component: str, units: list[dict], width: int = 400, height: int = 200) -> str:
    """Generate SVG treemap for a single component."""
    if not units:
        return ""
    
    # Count statuses
    total = len(units)
    implemented = sum(1 for u in units if u.get("status") in ["implemented", "conformant"])
    conformant = sum(1 for u in units if u.get("status") == "conformant")
    
    pct = (conformant / total * 100) if total > 0 else 0
    
    # Create values (all equal for now, could weight by complexity later)
    values = [1.0] * len(units)
    
    # Generate layout
    padding = 2
    header_height = 30
    treemap_y = header_height + padding
    treemap_height = height - treemap_y - padding
    
    rects = squarify(values, padding, treemap_y, width - 2*padding, treemap_height)
    
    # Build SVG
    svg_parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}">',
        f'  <rect width="{width}" height="{height}" fill="{COLOR_BG}"/>',
        f'  <text x="{padding}" y="20" font-family="system-ui, sans-serif" font-size="14" font-weight="600" fill="{COLOR_TEXT}">',
        f'    {component.upper()}: {pct:.1f}% ({conformant}/{total})',
        f'  </text>',
    ]
    
    # Add rectangles
    for rect in rects:
        unit = units[rect["index"]]
        color = get_unit_color(unit)
        rx, ry = rect["x"], rect["y"]
        rw, rh = max(rect["width"] - 1, 1), max(rect["height"] - 1, 1)
        
        svg_parts.append(
            f'  <rect x="{rx:.1f}" y="{ry:.1f}" width="{rw:.1f}" height="{rh:.1f}" '
            f'fill="{color}" rx="1">'
            f'<title>{unit["name"]}</title></rect>'
        )
    
    svg_parts.append('</svg>')
    return '\n'.join(svg_parts)


def generate_combined_treemap(data: dict, width: int = 900, height: int = 500) -> str:
    """Generate combined treemap SVG with all components."""
    units = data.get("units", [])
    summary = data.get("summary", {})
    
    # Group units by component
    components = {}
    for unit in units:
        comp = unit["component"]
        if comp not in components:
            components[comp] = []
        components[comp].append(unit)
    
    # Component order and labels
    comp_order = ["rest", "auth", "realtime", "storage", "functions", "pooler", "meta", "studio"]
    comp_labels = {
        "rest": "REST API",
        "auth": "Auth",
        "realtime": "Realtime",
        "storage": "Storage",
        "functions": "Functions",
        "pooler": "Pooler",
        "meta": "Meta",
        "studio": "Studio"
    }
    
    # Layout components in grid
    padding = 10
    header_height = 40
    cols = 4
    rows = 2
    
    cell_width = (width - padding * (cols + 1)) / cols
    cell_height = (height - header_height - padding * (rows + 1)) / rows
    
    total_units = len(units)
    total_conformant = sum(1 for u in units if u.get("status") == "conformant")
    overall_pct = (total_conformant / total_units * 100) if total_units > 0 else 0
    
    svg_parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}">',
        f'  <rect width="{width}" height="{height}" fill="{COLOR_BG}"/>',
        f'  <text x="{width/2}" y="28" font-family="system-ui, sans-serif" font-size="18" font-weight="700" fill="{COLOR_TEXT}" text-anchor="middle">',
        f'    Megabase Coverage: {overall_pct:.1f}% ({total_conformant}/{total_units} units)',
        f'  </text>',
    ]
    
    # Add each component
    for i, comp in enumerate(comp_order):
        if comp not in components:
            continue
        
        col = i % cols
        row = i // cols
        
        x = padding + col * (cell_width + padding)
        y = header_height + padding + row * (cell_height + padding)
        
        comp_units = components[comp]
        total = len(comp_units)
        conformant = sum(1 for u in comp_units if u.get("status") == "conformant")
        pct = (conformant / total * 100) if total > 0 else 0
        
        # Component box
        svg_parts.append(f'  <rect x="{x}" y="{y}" width="{cell_width}" height="{cell_height}" fill="#1f2937" rx="4"/>')
        
        # Component label
        label = comp_labels.get(comp, comp.title())
        svg_parts.append(
            f'  <text x="{x + 5}" y="{y + 16}" font-family="system-ui, sans-serif" '
            f'font-size="11" font-weight="600" fill="{COLOR_TEXT}">{label}: {pct:.1f}% ({conformant}/{total})</text>'
        )
        
        # Mini treemap inside
        inner_padding = 4
        inner_y = y + 22
        inner_height = cell_height - 26
        inner_width = cell_width - 2 * inner_padding
        
        values = [1.0] * len(comp_units)
        rects = squarify(values, x + inner_padding, inner_y, inner_width, inner_height)
        
        for rect in rects:
            unit = comp_units[rect["index"]]
            color = get_unit_color(unit)
            rx, ry = rect["x"], rect["y"]
            rw, rh = max(rect["width"] - 0.5, 0.5), max(rect["height"] - 0.5, 0.5)
            
            svg_parts.append(
                f'  <rect x="{rx:.1f}" y="{ry:.1f}" width="{rw:.1f}" height="{rh:.1f}" '
                f'fill="{color}" rx="0.5"><title>{unit["name"]}</title></rect>'
            )
    
    svg_parts.append('</svg>')
    return '\n'.join(svg_parts)


def generate_badge(label: str, value: str, color: str) -> str:
    """Generate a shields.io-style badge SVG."""
    label_width = len(label) * 7 + 10
    value_width = len(value) * 7 + 10
    total_width = label_width + value_width
    
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{total_width}" height="20">
  <linearGradient id="smooth" x2="0" y2="100%">
    <stop offset="0" stop-color="#bbb" stop-opacity=".1"/>
    <stop offset="1" stop-opacity=".1"/>
  </linearGradient>
  <clipPath id="round"><rect width="{total_width}" height="20" rx="3" fill="#fff"/></clipPath>
  <g clip-path="url(#round)">
    <rect width="{label_width}" height="20" fill="#555"/>
    <rect x="{label_width}" width="{value_width}" height="20" fill="{color}"/>
    <rect width="{total_width}" height="20" fill="url(#smooth)"/>
  </g>
  <g fill="#fff" text-anchor="middle" font-family="DejaVu Sans,Verdana,Geneva,sans-serif" font-size="11">
    <text x="{label_width/2}" y="15" fill="#010101" fill-opacity=".3">{label}</text>
    <text x="{label_width/2}" y="14">{label}</text>
    <text x="{label_width + value_width/2}" y="15" fill="#010101" fill-opacity=".3">{value}</text>
    <text x="{label_width + value_width/2}" y="14">{value}</text>
  </g>
</svg>'''


def main():
    """Main entry point."""
    print("Generating treemap SVGs...")
    
    # Load units
    data = load_units()
    units = data.get("units", [])
    summary = data.get("summary", {})
    
    total = summary.get("total_units", len(units))
    conformant = summary.get("conformant", 0)
    implemented = summary.get("implemented", 0)
    
    coverage_pct = (implemented / total * 100) if total > 0 else 0
    conformance_pct = (conformant / total * 100) if total > 0 else 0
    
    # Generate combined treemap
    combined_svg = generate_combined_treemap(data)
    with open(COVERAGE_DIR / "treemap.svg", "w") as f:
        f.write(combined_svg)
    print(f"  Generated: coverage/treemap.svg")
    
    # Generate per-component treemaps
    components = {}
    for unit in units:
        comp = unit["component"]
        if comp not in components:
            components[comp] = []
        components[comp].append(unit)
    
    for comp, comp_units in components.items():
        svg = generate_component_treemap(comp, comp_units)
        with open(COVERAGE_DIR / f"treemap-{comp}.svg", "w") as f:
            f.write(svg)
        print(f"  Generated: coverage/treemap-{comp}.svg")
    
    # Generate badges
    badges = [
        ("coverage", f"{coverage_pct:.1f}%", "#007ec6"),
        ("conformance", f"{conformance_pct:.1f}%", "#22c55e" if conformance_pct > 50 else "#eab308" if conformance_pct > 10 else "#6b7280"),
        ("units", str(total), "#555"),
    ]
    
    for name, value, color in badges:
        badge_svg = generate_badge(name, value, color)
        with open(COVERAGE_DIR / f"badge-{name}.svg", "w") as f:
            f.write(badge_svg)
        print(f"  Generated: coverage/badge-{name}.svg")
    
    # Generate summary JSON for CI
    summary_out = {
        "total_units": total,
        "implemented": implemented,
        "conformant": conformant,
        "coverage_percent": round(coverage_pct, 2),
        "conformance_percent": round(conformance_pct, 2),
        "by_component": {}
    }
    
    for comp, comp_units in components.items():
        comp_total = len(comp_units)
        comp_impl = sum(1 for u in comp_units if u.get("status") in ["implemented", "conformant"])
        comp_conf = sum(1 for u in comp_units if u.get("status") == "conformant")
        summary_out["by_component"][comp] = {
            "total": comp_total,
            "implemented": comp_impl,
            "conformant": comp_conf,
            "coverage_percent": round((comp_impl / comp_total * 100) if comp_total > 0 else 0, 2),
            "conformance_percent": round((comp_conf / comp_total * 100) if comp_total > 0 else 0, 2),
        }
    
    with open(COVERAGE_DIR / "summary.json", "w") as f:
        json.dump(summary_out, f, indent=2)
    print(f"  Generated: coverage/summary.json")
    
    print(f"\nDone! Coverage: {coverage_pct:.1f}%, Conformance: {conformance_pct:.1f}%")


if __name__ == "__main__":
    main()
