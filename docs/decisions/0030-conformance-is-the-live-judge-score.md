# Conformance score is the live Judge publication

- Status: Accepted (agent coordinator)
- Date: 2026-10-10

This file is 0030. On `main`, 0029 is already
`0029-judge-hidden-suite.md` and `0029-stop-publishing-cost.md`.

## Context

`coverage/judge-results.json` on `main` is the regression baseline. A case
recorded there as passing fails the Judge job if it later fails. Decision
0022 stopped CI from committing Judge output back to `main` (branch
protection requires a pull request). Feature pull requests restore the file
from `origin/main` and do not change it.

Every case in that committed file is `pass: false`. `megabase-coverage`
therefore wrote `conformance 0% (0/94 judge cases)` into the generated
status block. The same zero is `percent.conformance` in the committed
`coverage/summary.json`, which release notes copied into the GitHub Release
body.

After a successful Judge run on `main`, `pages-badges.yml` applies the
artifact and publishes shields JSON, the README treemap, and the site.
Fetched on 2026-10-10, that publication was 70.2% (66/94 cases). The README
badge already reads it. The generated prose and the release notes did not.

Committing the latest results into the baseline was rejected. The next
Judge run would make the commit stale, and nothing on `main` is allowed to
rewrite the file without a pull request. A number that is true only until
the next green Judge run is not a source of truth.

## Decision

The live Judge publication is the only conformance score.

- Generated prose in `PROGRESS.md`, `README.md`, and the units table in
  `docs/COMPATIBILITY.md` does not print a conformance percentage or a
  conformant-unit count taken from the baseline. Those blocks link to
  [megabase.sh/status](https://megabase.sh/status/) and the README
  conformance badge.
- `coverage/judge-results.json` stays the regression baseline. It is not
  updated from Judge on `main`.
- Committed `coverage/summary.json` still stores baseline pass counts so
  `just coverage` stays deterministic. Do not quote `percent.conformance`
  from that file. The copy on `gh-pages` is produced after the live
  results are applied.
- `pages.yml` builds the site and does not deploy it. Deploying the
  checkout would replace the live score with the baseline. `pages-badges.yml`
  is the publisher. It appends `coverage/judge-history.json` on `gh-pages`
  (commit SHA, passing cases, case count, percent). `force_orphan` drops
  git history on that branch; the JSON file is the record.
- Release notes take coverage (implemented ÷ units) from the tag's
  `coverage/summary.json`. They take conformance from `judge-history.json`,
  or from the Judge `judge-results` artifact for that commit. If neither
  exists they say conformance was not measured. They do not subtract the
  baseline's zero.

## Consequences

The release job waits for Judge only when the release commit is the
push that opened the release. An older tag uses one lookup. Tags from
before the history file exists have no conformance delta until a later
release has a publication for both commits. The publisher asks the
contents API whether `coverage/judge-history.json` is on `gh-pages`.
HTTP 404 means that path is not in the tree, and only then is an empty
history written. Any other status or a transport failure fails the job
before `force_orphan`, so a transient error cannot replace earlier runs. Passes that exist only in the live run are not
regressions until a commit records them in the baseline. That recording
stays a deliberate change to `coverage/judge-results.json`, not a
side effect of generating the status block.
