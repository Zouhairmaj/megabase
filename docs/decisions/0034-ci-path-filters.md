# CI path filters for site-only pull requests

- Status: Accepted
- Date: 2026-10-10

A pull request that only changes the static site was running the full
server suite (build, judge, image, fuzz, supply-chain, Codecov). The
workflows now classify the diff in `.github/actions/pr-paths` and skip
that work inside the existing jobs. The behavior is described in
[Pull request checks](../contributing.md#pull-request-checks).

`paths` / `paths-ignore` on the workflow trigger is not used. A workflow
that does not start leaves a required status check at "Expected —
waiting". Each job keeps its current name. When the diff does not apply,
the job succeeds without the expensive steps, so branch protection sees
success. Push to `main`, schedules, and `workflow_dispatch` (including
release lockfile sync) are not filtered. If the path-filter job
fails, dependent jobs run the full suite. A pull request that changes
a site-generator input still audits `site/Cargo.lock`. A pull request
that only changes one component crate runs that Judge shard; shared
and unclassifiable paths run every shard. The crate-to-shard map is
`COMPONENT_SHARDS` in `.github/actions/pr-paths`.

CodeQL stays on GitHub's default code-scanning setup. There is no
`codeql.yml` to filter. Switching to advanced setup disables default
setup. Default setup may leave an existing CodeQL workflow disabled.
