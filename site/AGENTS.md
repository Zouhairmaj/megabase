# site/: megabase.sh

Extends the root `AGENTS.md`; it does not relax it.

- A standalone Cargo project, outside the workspace:
  ```bash
  cargo run --manifest-path site/Cargo.toml --release -- --repo-root . --out _site
  cargo fmt --manifest-path site/Cargo.toml -- --check
  cargo clippy --manifest-path site/Cargo.toml -- -D warnings
  ```
  CI does not lint it, so run these yourself.
- Deployed to GitHub Pages by `.github/workflows/pages.yml` on push to `main`.
- Root design gate: the PR links the approved Kite frame and its committee
  review.
- The Status section embeds `coverage/treemap.svg` and
  `coverage/treemap-light.svg`; never copy them. OG images are generated in
  Rust (`src/og.rs`).
- The disclaimer "Not affiliated with or endorsed by Supabase, Inc." stays
  word-for-word identical everywhere.
