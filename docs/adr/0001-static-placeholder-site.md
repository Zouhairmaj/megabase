# ADR 0001: Static placeholder website

- Status: Accepted
- Date: 2026-10-09
- Decision makers: coordinator (tech), design committee (Kite file `megabase-identity`, page “Website / Placeholder”)

## Context

`GOAL.md` rule 1 says all code we write is Rust, with three named exceptions: configuration, compatibility SQL, and Studio front-end assets when a Rust UI framework requires them.

The public experiment still needs a temporary site until a designed product site exists. That site is not a Megabase runtime component. It is documentation and status, the same way Studio’s static assets are UI rather than the Rust services in `crates/`.

A complete multi-page site (Status, Roadmap, How it works, Components, Devlog, Human log, FAQ, Docs, 404) is being designed in Kite. Home and Manifesto ship first; later pages must drop in without copying chrome.

Without an explicit decision, a strict reading of rule 1 would block HTML/CSS in the repository.

## Decision

Static site assets are allowed under `GOAL.md` rule 1 by the same rationale as Studio front-end assets: they are not a rewrite of a Supabase service, they are not served by the `megabase` binary, and they do not replace Rust for anything in scope.

Constraints:

1. Isolation. Everything for the public site lives under the top-level `site/` directory, plus `wrangler.toml` and one GitHub Actions workflow that publishes it to Cloudflare Workers static assets. No JavaScript framework, no Node bundler, no GitHub Pages.
2. Shared chrome. A tiny Rust generator (`site/`, package `megabase-site`) wraps every page in one layout, nav, footer and logo. New pages are a `Page` entry plus a template under `site/templates/pages/`. Forthcoming routes are listed in the generator and are not invented ahead of the Kite design.
3. Coverage is data, not a client fetch. When `coverage/units.json` (and optionally `coverage/summary.json`) exist, the generator inlines the unit total, passing count, treemap and Scope table at publish time. The denominator is always the length of `units` (currently 334 on Phase 0), never a hardcoded number. If those files are absent, Day-0 placeholders show an em dash (`—`) rather than a fake total, `Phase 0 · bootstrap` for the stage, and an empty treemap. The treemap is one labeled block per component. Inside each block, every unit is exactly one whole square; all squares are the same size, centered in the block, and never clipped. Colors are brand only: background `#0B0E12`, not started `#303235`, implemented `#005441`, tested `#009366`, conformant `#00D892`.
4. The manifesto body is rendered from `MANIFESTO.md` so the page stays in sync with the file.
5. Fonts are self-hosted OFL files (JetBrains Mono on every page; Inter for manifesto body). No Google Fonts CDN. The nav mark is one shared SVG (`site/static/logo.svg`): a 3x3 grid of squares, never a grid-character glyph.
6. The site must not import, wrap, or reimplement any Supabase service.

## Consequences

- Agents working on crates, `judge/`, or `coverage/` do not need to touch `site/`.
- Merging this work with Phase 0 is order-independent: the generator reads `coverage/` when present and otherwise keeps placeholders.
- Adding Status, Roadmap and the rest is a template plus a registry row, not a layout rewrite.
- Deploy is Cloudflare Workers static assets (`megabase-site`). The workflow skips instead of failing when `CLOUDFLARE_API_TOKEN` or `CLOUDFLARE_ACCOUNT_ID` are unset. Token setup is recorded as pending in `HUMAN_LOG.md`.
