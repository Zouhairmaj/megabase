# Owned lockfiles

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

Scorecard/OSV flagged RUSTSEC-2026-0206 (`rustybuzz` unmaintained) and
RUSTSEC-2026-0192 (`ttf-parser` unmaintained) in `site/Cargo.lock`, not the
workspace lockfile and not `vendor/`. `site/` is excluded from the
workspace, so root `cargo audit` missed them. Fix: `resvg` 0.45 → 0.48
(harfrust + skrifa). `just audit` and the CI `cargo-audit` matrix (#157)
each pass `--file` for every owned lockfile (`Cargo.lock`,
`site/Cargo.lock`); a workspace-only run misses `site/`. `vendor/**`
lockfiles stay frozen; do not add an OSV ignore unless a finding exists
only there.
