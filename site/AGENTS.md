# site/: megabase.sh

Extends the root `AGENTS.md`; it does not relax it.

- A standalone Cargo project, outside the workspace:
  ```bash
  cargo run --manifest-path site/Cargo.toml --release -- --repo-root . --out _site
  cargo fmt --manifest-path site/Cargo.toml -- --check
  cargo clippy --manifest-path site/Cargo.toml -- -D warnings
  cargo audit --file site/Cargo.lock
  ```
  CI does not lint it, so run fmt/clippy yourself. Scorecard/OSV,
  `just audit`, and the CI `cargo-audit` matrix all scan
  `site/Cargo.lock`. Keep `resvg` on the harfrust/skrifa stack (0.48+);
  do not regress to rustybuzz or ttf-parser (RUSTSEC-2026-0206,
  RUSTSEC-2026-0192).
- Deployed to GitHub Pages by `.github/workflows/pages.yml` on push to `main`.
- Root design gate: the PR links the approved Kite frame and its committee
  review.
- Treemaps are generated in Rust (`src/treemap.rs`) from
  `coverage/summary.json` and `coverage/units.json` and inlined as dark SVGs.
  Do not embed `coverage/treemap.svg` or `coverage/treemap-light.svg`. OG
  images are generated in Rust (`src/og.rs`).
- The disclaimer "Not affiliated with or endorsed by Supabase, Inc." stays
  word-for-word identical everywhere.
