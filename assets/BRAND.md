# Megabase brand

## Mark

A 3×3 grid of square tiles with even gaps. Eight charcoal tiles surround one
green center tile. The grid is the treemap of units the project is measured
by; the green tile is a unit that is done: implemented and conformant.

The wordmark `MEGABASE` is drawn as SVG paths in a squared geometric
monospace style (every letter the same width, chamfered corners, wide
tracking), so it renders identically everywhere and needs no font.

The definitive reference for proportions is the brand sheet in
[`docs/brand/reference.png`](../docs/brand/reference.png). The sheet shows an
orange center tile; the accent has since changed to Megabase green (below), and
everything else on the sheet still applies.

The mark is original. The green accent is a nod to the project Megabase is
compatible with, but it is deliberately a different green from Supabase's
brand greens (`#3ECF8E`, `#24B47E`): a purer, more saturated green with less blue, and it is
never used with a lightning bolt or any Supabase shape.

## Files

All files are hand-written SVG in this directory.

| File | Use |
|---|---|
| `logo-icon.svg` / `logo-icon-dark.svg` | Icon only (avatars, small spaces) |
| `logo-horizontal.svg` / `logo-horizontal-dark.svg` | Icon + wordmark side by side (README header, docs) |
| `logo-stacked.svg` / `logo-stacked-dark.svg` | Icon above wordmark (square-ish placements) |
| `wordmark.svg` / `wordmark-dark.svg` | Wordmark only |
| `app-icon.svg` / `app-icon-dark.svg` | Rounded-square app icon (off-white card, or charcoal card with off-white tiles) |
| `favicon.svg` | Favicon; switches to the dark palette with `prefers-color-scheme` |

The plain files are for light backgrounds; `-dark` files are for dark
backgrounds. In Markdown, pick automatically with `<picture>`:

```html
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/logo-horizontal-dark.svg">
  <img alt="Megabase" src="assets/logo-horizontal.svg" width="360">
</picture>
```

## Palette

| Name | Hex | Use |
|---|---|---|
| Charcoal | `#1F1F1F` | Tiles and wordmark on light backgrounds |
| Off-white | `#F5F2EA` | Tiles and wordmark on dark backgrounds |
| Megabase green | `#1FCB5B` | Center tile; the only accent color |

Do not substitute Supabase's greens (`#3ECF8E`, `#24B47E`) for the accent.

The coverage treemaps use the same palette: charcoal for units not yet
implemented, lighter grey for implemented but untested, deep green for tested
but not yet passing, and Megabase green `#1FCB5B` for conformant.

## Geometry

- Icon: 96×96 units, tiles 30×30, gaps 3 (a tenth of a tile).
- Wordmark: cap height 100, every letter 96 wide, stroke 17, 45° chamfers of
  14, letter spacing 44. The full word is 1076×100.
- Stacked lockup: wordmark about twice the icon width, gap between icon and
  wordmark about a quarter of the icon height.
- Horizontal lockup: cap height 0.28 of the icon height, vertically centered,
  gap of 0.3 icon heights between icon and wordmark.
- App icon: 512×512 rounded square (corner radius 112), grid at 60% of the
  width, centered.
- Clear space around any lockup: at least one tile width.

## Rules

1. Use the variant that matches the background.
2. Scale proportionally; never stretch, recolor, rotate or add effects.
3. Never use the mark in a way that implies endorsement by Supabase.

## Non-affiliation

Megabase is independent and not affiliated with Supabase. "Supabase" is a
trademark of its owner and is used only to describe compatibility.

## License

Apache-2.0, like the rest of the project.
