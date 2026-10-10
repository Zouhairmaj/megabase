# site/: megabase.sh

Extends the root `AGENTS.md`; it does not relax it.

- A standalone Cargo project, outside the workspace:
  ```bash
  cargo run --manifest-path site/Cargo.toml --release -- --repo-root . --out _site
  cargo fmt --manifest-path site/Cargo.toml -- --check
  cargo clippy --manifest-path site/Cargo.toml --all-targets --locked -- -D warnings
  cargo audit --file site/Cargo.lock
  ```
  `[lints.rust]` forbids `unsafe_code` here because this crate cannot inherit
  `[workspace.lints]`. Scorecard/OSV,
  `just audit`, and the CI `cargo-audit` matrix all scan
  `site/Cargo.lock`. The CI matrix still audits that lockfile when a
  pull request changes a site-generator input; the workspace lockfile
  leg is skipped on a site-only diff. Push to `main` audits both.
  Keep `resvg` on the harfrust/skrifa stack (0.48+);
  do not regress to rustybuzz or ttf-parser (RUSTSEC-2026-0206,
  RUSTSEC-2026-0192).
- `pages.yml` lints, tests, and builds the site on pull requests that
  change a generator input, and on push to `main` and `workflow_dispatch`.
  A pull request that does not change those paths reports **Generate site**
  as success without building. The workflow does not deploy that build. It checks out
  the event SHA only, does not download Judge output, and saves the
  default-branch Rust cache on pushes to `main`. Deploying that
  checkout would publish the regression baseline over the live score.
  After a successful Judge run on `main`,
  `.github/workflows/pages-badges.yml` (`workflow_run` only, no cache
  action) checks out the default branch and applies the Judge JSON as
  data when that commit is the checkout or an ancestor of it, then
  publishes shields JSON, `judge-history.json`, the README treemap
  PNGs to `gh-pages`, and deploys the site. It does not set
  `actions/checkout` `ref` from the triggering run. The generator
  writes the JSON and those PNGs to `_site/coverage/` (never treemap
  SVGs). `pages-badges.yml` uses the `pages` concurrency group.
  Neither workflow pushes to `main`.
- Root design gate: the PR links the approved Kite frame and its committee
  review.
- Treemaps are generated in Rust (`src/treemap.rs`) from
  `coverage/summary.json` and `coverage/units.json` and inlined as dark SVGs.
  Do not embed `coverage/treemap.svg` or `coverage/treemap-light.svg`. OG
  images are generated in Rust (`src/og.rs`).
- The disclaimer "Not affiliated with or endorsed by Supabase, Inc." stays
  word-for-word identical everywhere.
