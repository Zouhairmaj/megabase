# README status is generated

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

PRs run `megabase-coverage update`; `check` is a required CI job. Agents
keep it current. CI does not commit generated files to `main` (branch
protection requires a PR). After Judge on `main`,
`.github/workflows/pages-badges.yml` publishes shields JSON and the
README treemap PNGs to the `gh-pages` branch and redeploys megabase.sh
from those results. `coverage/judge-results.json` in git is the
regression baseline; feature PRs restore it from `origin/main`. The
committed treemap SVGs are that baseline. Totals come from
`coverage/units.json`.
