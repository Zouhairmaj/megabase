# ADR 0003: Protected-path policy and the Phase 0 bootstrap exception

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

## Context

GOAL.md forbids changing `vendor/` after bootstrap, and forbids changing
`judge/` outside a `review/*` branch. Humans own `GOAL.md`, `MANIFESTO.md`
and `HUMAN_LOG.md`. The Phase 0 bootstrap PR must *create* `vendor/`,
`judge/` and `HUMAN_LOG.md`, so a naive "reject any touch of these paths"
check would block the one PR that is allowed to introduce them.

## Decision

`tools/megabase-guard` is the policy. It is a pure function of the change
set plus a small amount of git context (base tree, head branch, pin
agreement). CI runs it on every pull request.

### Always rejected

- Edits to `GOAL.md` or `MANIFESTO.md`, except the lead-approved Design
  and Documentation sections on the Phase 0 bootstrap branch, and except
  a logged owner edit on `review/*` (below).
- Edits to `HUMAN_LOG.md` except the bootstrap case and the `review/*`
  pending/completed exception below.
- Edits to `vendor/`, `vendor.toml` or `.gitmodules` after bootstrap.
- Source files in languages other than Rust (`.py`, `.js`, `.ts`, `.go`,
  `.ex`, `.hs`, `.rb`, `.lua`, `.sh`, …) outside `vendor/`. SQL, TOML,
  YAML, Markdown and SVG stay allowed. The one exception is the exact
  path `site/static/db-dither.js` (below).

### Allowed only on `review/*` branches

- `judge/`
- `.github/`
- `tools/megabase-guard/`
- `CODEOWNERS`

The reviewer agent reviews those PRs. The orchestrator merges them.

### Bootstrap exception (self-expiring)

The exception is active **only if both** are true:

1. The merge base has none of `vendor.toml`, `judge/` or vendor gitlinks
   (the tree is still Day 0).
2. The head branch is exactly `cursor/phase-0-bootstrap-121c`.

Then the PR may add the protected tree, provided:

- `MANIFESTO.md` is unchanged.
- `GOAL.md` may receive the lead-approved Design (Kite) and
  Documentation sections only; after this PR merges, `GOAL.md` is
  human-owned again.
- `HUMAN_LOG.md` is **created empty** (zero bytes).
- Every `vendor/` gitlink matches `vendor.toml`.

The exception cannot apply to a later PR: after this lands, the base has
`vendor.toml` and condition 1 is false. It cannot apply to a different
branch name even before this lands. It is not a label anyone can add.

### Release-please exception

Branches named `release-please--branches--*` may change only the version
bump set:

- `CHANGELOG.md`
- `.release-please-manifest.json`
- `Cargo.toml`, and only the `[workspace.package]` `version` string.
  Dependency lines, comments, and every other key must match the base.
- `Cargo.lock`, and only quoted `version = "..."` lines (the workspace
  member versions `cargo update -w` writes). The unquoted lockfile
  format `version`, checksums, and package entries must match the base.
- `release-please-config.json` only when the sole change is deleting
  `release-as` (required before the v0.1.0 PR merges). Any other edit
  to that file is rejected.

Any other path on those branches is rejected, including reviewed paths
and product code. A release diff that deletes a workspace dependency
fails this check. This is narrower than a `review/*` exception: the bot
cannot land CI or judge changes through a release PR.

`review/*` may append items under `## Pending` in `HUMAN_LOG.md`
(a grow whose added suffix starts a new `- ` item) and may drop whole
Pending items that start with `- **Date**`, including an earlier item
when the items that stay are unchanged, in the same order, and still
separated by the same blank lines. It may
also shrink a suffix that ends at a complete Pending item whose removed
suffix starts with `- **Date**`. It may grow the Completed section
(replace the empty-log placeholder, or append after existing completed
entries). A shrink does not have to add a Completed entry. Format text
and earlier completed entries stay unchanged. Mid-item Pending
truncations, edited items, and continuation-line growth are rejected.

### Logged owner edits

On a `review/*` branch, a modification of `GOAL.md` or `MANIFESTO.md` is
allowed only when that same diff also passes the `HUMAN_LOG.md` rule
above and the new Completed suffix contains the file name in backticks
(`` `GOAL.md` `` or `` `MANIFESTO.md` ``). A longer path, the same
name without backticks, or a continuation of an earlier entry does not
count. Deletions stay rejected. Any
other branch cannot use this exception. This is the logged human
approval `CODEOWNERS` describes. The reviewer agent still reviews the
pull request.

### Decorative hero canvas (issue #228)

Owner Zouhair approved one client-side canvas, `site/static/db-dither.js`,
as the site hero illustration. Product code stays Rust. The guard allows
that exact path on any branch. A different extension, a different file,
or a capitalised `.JS` name stays rejected.

The branch `cursor/hero-dither-db-13b4` may also edit
`tools/megabase-guard/` and append one Completed `HUMAN_LOG.md` entry,
and only when that new entry names `` `site/static/db-dither.js` ``.
`judge/`, `.github/`, `CODEOWNERS`, `GOAL.md` and `MANIFESTO.md` stay
closed on that branch. After it merges, further guard edits still need
`review/*`.

## Consequences

A required status check named **Protected paths** should be set on `main`
(human-only: branch protection). Agents keep the policy in this ADR and in
`docs/decisions/`; they do not weaken it for convenience.
