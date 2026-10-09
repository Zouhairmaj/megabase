# Megabase Brand Guidelines

## Logo

The Megabase logo represents a bold, ascending "M" formed by three database column pillars rising from a shared foundation. The center pillar reaches highest, symbolizing growth and the unified nature of the single-binary architecture.

### Design Elements

- **Three Pillars**: Represent the core services (REST, Auth, Realtime) unified into one
- **Connecting Beams**: Show how components work together seamlessly
- **Base Platform**: The PostgreSQL foundation everything builds upon

### Files

| File | Use Case |
|------|----------|
| `logo.svg` | Primary logo for light backgrounds |
| `logo-dark.svg` | Logo variant for dark backgrounds |
| `wordmark.svg` | Full wordmark for headers, light backgrounds |
| `wordmark-dark.svg` | Wordmark for dark backgrounds |

## Colors

The Megabase palette uses a blue-to-indigo gradient, deliberately distinct from Supabase's green branding.

### Primary Palette

| Name | Hex | Usage |
|------|-----|-------|
| Deep Navy | `#1e3a5f` | Primary dark, text on light |
| Royal Blue | `#3b5998` | Gradient midpoint |
| Indigo | `#6366f1` | Accent, links |
| Light Indigo | `#818cf8` | Highlights |

### Dark Mode Palette

| Name | Hex | Usage |
|------|-----|-------|
| Sky Blue | `#60a5fa` | Primary on dark |
| Soft Indigo | `#818cf8` | Gradient midpoint |
| Lavender | `#a78bfa` | Accent on dark |
| Light Lavender | `#c4b5fd` | Highlights on dark |

### Text Colors

- Light mode: `#1e3a5f` (Deep Navy)
- Dark mode: `#e2e8f0` (Slate 200)

## Typography

Use system fonts for maximum compatibility:

```css
font-family: system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif;
```

The wordmark uses **700 weight** (bold).

## Usage Guidelines

1. **Do** use the appropriate variant for the background (light/dark)
2. **Do** maintain aspect ratio when scaling
3. **Do** provide adequate padding around the logo
4. **Don't** modify the colors or gradient direction
5. **Don't** add effects like shadows or outlines
6. **Don't** use the logo in ways that imply Supabase endorsement

## Non-Affiliation Notice

Megabase is an independent project. "Supabase" is a trademark of Supabase, Inc. The Megabase logo and brand are original designs that intentionally differ from Supabase's visual identity.

## License

The Megabase logo and brand assets are released under Apache-2.0, the same license as the project.
