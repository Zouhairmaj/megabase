# Standalone conformance percentage stays off the site

- Status: Accepted (agent)
- Date: 2026-10-10

## Context

[Decision 0030](0030-conformance-is-the-live-judge-score.md) names the live
Judge publication as the conformance score. Generated prose in `README.md`
and `PROGRESS.md` linked both [megabase.sh/status](https://megabase.sh/status/)
and the README conformance badge, and did not print a percentage from the
regression baseline.

The status page now shows units passing and coverage. It does not show a
standalone conformance percentage. `SHOW_CONFORMANCE_PERCENT` in
`site/src/metrics.rs` is `false`. The README shields badge, published from
Judge on `main` to `gh-pages`, remains that percentage.

## Decision

- Generated status blocks in `README.md` and `PROGRESS.md` do not say the
  conformance percentage is shown on megabase.sh/status. They point at the
  README conformance badge. Decision 0030 still holds: those blocks do not
  copy `percent.conformance` from `coverage/summary.json` or from
  `coverage/judge-results.json`.
- While `SHOW_CONFORMANCE_PERCENT` is `false`, the site does not render a
  standalone conformance percentage. That covers the home status panel, the
  home treemap header, the status-page stat card, the Day 0 strip on How it
  works and Roadmap, and the docs status total row. Unit counts and coverage
  stay.
- `docs/COMPATIBILITY.md` may still link megabase.sh/status for live
  conformant counts. A count is not the standalone percentage.

## Consequences

Turning the percentage back on is that one flag, then regenerating the
status blocks so they name the status page again. Decision 0030 is
unchanged.
