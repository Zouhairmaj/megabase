# Level 1 validation requirements

- Status: Accepted
- Date: 2026-10-10

Issue [#199](https://github.com/Zouhairmaj/megabase/issues/199). Level 1
(REST, Auth email/password) is marked validated only when all of these
hold:

- **Minimum cases per unit.** Every in-scope Level 1 unit has at least
  **3** linked judge cases. Level 1 cannot be marked validated while any
  in-scope unit has fewer.
- **Hidden suite.** The hidden suite
  ([#114](https://github.com/Zouhairmaj/megabase/issues/114)) has
  published a pass/fail count. Level 1 cannot be marked validated before
  that.
- **Totals and regressions.** The ≥95% threshold uses total conformance.
  Total conformance must not decrease (GOAL.md §3 rule 4), so Level 1
  cannot be marked validated if the total fell, even when it is still
  ≥95% and no baseline-passing case regressed. A regression still fails
  the judge on a pull request.

This adds to the level gates accepted in
[proposal 0001](../proposals/0001-external-review.md); it does not loosen
them. `PROGRESS.md` records only the current validation state.
