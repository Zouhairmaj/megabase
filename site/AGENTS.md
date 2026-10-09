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
- Deployed to GitHub Pages by `.github/workflows/pages.yml` on push to
  `main` (and `workflow_dispatch`). That workflow checks out the event
  SHA only and does not download Judge output. It saves the
  default-branch Rust cache on pushes to `main`. After a successful
  Judge run on `main`, `.github/workflows/pages-badges.yml`
  (`workflow_run` only, no cache action) checks out the default branch
  and applies the Judge JSON as data when that commit is the checkout
  or an ancestor of it, then publishes shields JSON and the README
  treemap PNGs to `gh-pages` and redeploys the site. It does not set
  `actions/checkout` `ref` from the triggering run. The generator
  writes the JSON and those PNGs to `_site/coverage/` (never treemap
  SVGs). Both workflows share the `pages` concurrency group. Neither
  pushes to `main`.
- Root design gate: the PR links the approved Kite frame and its committee
  review.
- Treemaps are generated in Rust (`src/treemap.rs`) from
  `coverage/summary.json` and `coverage/units.json` and inlined as dark SVGs.
  Do not embed `coverage/treemap.svg` or `coverage/treemap-light.svg`. OG
  images are generated in Rust (`src/og.rs`).
- The disclaimer "Not affiliated with or endorsed by Supabase, Inc." stays
  word-for-word identical everywhere.
