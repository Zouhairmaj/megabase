# Megabase brand

Dark-first and technical: a monospace wordmark, a grid of tiles, one green
accent.

## Mark

A 3×3 grid of square tiles with even gaps. Eight neutral tiles surround one
green center tile. The grid is the treemap of units the project is measured
by; the green tile is a unit that is done.

[`reference.png`](reference.png) is the reference for proportions and layout
(icon, stacked lockup, dark card, rounded app icon). Its colors are
superseded by the Megabase palette below.

## Megabase palette

### Green (accent)

| Step | Hex | Use |
|---|---|---|
| 200 | `#002923` | LED-grid tile fill |
| 400 | `#005441` | LED-grid tile outline; treemap "implemented" |
| 600 | `#009366` | Center tile on light backgrounds; treemap "tested" |
| 800 | `#00D892` | **Primary accent.** Center tile on dark; treemap "conformant" |
| 900 | `#37E6A8` | Hover/highlight on dark |
| 1100 | `#A3EFCB` | Subtle tints, text on deep green |

### Neutrals

| Step | Hex | Use |
|---|---|---|
| 0 | `#0B0E12` | Dark background |
| 100 | `#181A1D` | Raised surface; tiles on light backgrounds |
| 300 | `#303235` | Borders; empty tiles; tiles on dark backgrounds |
| 500 | `#5D5E61` | Muted text |
| 800 | `#BABABB` | Body text on dark |
| 1200 | `#F7F7F7` | Light background; wordmark on dark |

## Logo files

All files are SVG in [`assets/`](../../assets). Default files are for light
backgrounds; `-dark` and `-led` files are for dark backgrounds.

| File | Tiles | Center | Wordmark |
|---|---|---|---|
| `logo-icon.svg`, `logo-horizontal.svg`, `logo-stacked.svg`, `wordmark.svg` | `#181A1D` | `#009366` | `#181A1D` |
| `logo-icon-dark.svg`, `logo-horizontal-dark.svg`, `logo-stacked-dark.svg`, `wordmark-dark.svg` | `#303235` | `#00D892` | `#F7F7F7` |
| `logo-icon-led.svg`, `logo-horizontal-led.svg`, `logo-stacked-led.svg` | `#002923` outlined `#005441` | `#00D892` | `#F7F7F7` |
| `app-icon.svg` | `#181A1D` on a `#F7F7F7` rounded card | `#009366` | — |
| `app-icon-dark.svg` | `#303235` on a `#0B0E12` rounded card | `#00D892` | — |
| `favicon.svg` | light or dark set, via `prefers-color-scheme` | | — |

In Markdown, pick the variant automatically:

```html
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/logo-horizontal-dark.svg">
  <img alt="Megabase" src="assets/logo-horizontal.svg" width="360">
</picture>
```

## Wordmark

`MEGABASE` set in JetBrains Mono Bold with wide tracking (150/1000 em),
converted to SVG paths so it renders without the font installed. JetBrains
Mono is licensed under the SIL Open Font License 1.1; see
[`LICENSES/OFL-1.1-JetBrains-Mono.txt`](../../LICENSES/OFL-1.1-JetBrains-Mono.txt).
Running text in generated graphics uses the stack
`JetBrains Mono, IBM Plex Mono, SFMono-Regular, Menlo, Consolas, monospace`.

## Geometry

- Icon: 96×96, tiles 30×30, gaps 3 (a tenth of a tile).
- Stacked lockup: wordmark twice the icon width, gap of 22 (about a quarter of
  the icon height) between icon and wordmark.
- Horizontal lockup: cap height 27 for a 96 icon, vertically centered, gap of
  30 between icon and wordmark.
- App icon: 512×512 rounded square, corner radius 112, grid at 60% width.
- Clear space around any lockup: at least one tile width.

## Generated graphics

Treemaps and badges in `coverage/` are drawn by `tools/megabase-coverage` in
this palette, on a `#0B0E12` background with monospace labels:

| State | Color |
|---|---|
| conformant | `#00D892` |
| tested | `#009366` |
| implemented | `#005441` |
| not done | `#303235` |

Badges use a `#181A1D` label with a green value: `#00D892` from 90%,
`#009366` from 50%, `#005441` above 0%, `#303235` at 0%.

## Rules

1. Use the variant that matches the background.
2. Scale proportionally; never stretch, recolor, rotate or add effects.
3. Never use the mark in a way that implies endorsement by any other project.

## License

The Megabase mark and assets are Apache-2.0, like the rest of the project.
The wordmark outlines derive from JetBrains Mono (OFL-1.1).
