# <Component>: <unit or group> (issue <n>)

<!--
Copy this file to specs/<component>/<unit>.md (GOAL.md section 5, step 3).
Write it from vendor/ before any code. Source beats docs; the reference
stack beats source. Replace every <placeholder> and delete these comments.
Status line values: stub | draft | complete.
-->

Status: stub
Unit ids (`coverage/units.json`): `<component>:<kind>:<name>`, ...
Level: <1-5>

One paragraph: what this unit is and what clients rely on it for.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| <behavior> | [`vendor/<repo>/<path>:<line>`](<permalink at the commit pinned in vendor.toml>) |

Pins: `<repo> <tag>` (`vendor.toml`). Ported code carries the credit header
required by AGENTS.md.

## Inputs

Method, path, query parameters, headers, body, SQL arguments.

## Outputs

Status codes, headers that matter, body shape, side effects in the database.

## Errors

| When | Status | Code / message (upstream) |
|---|---|---|

## Edge cases

Empty input, duplicates, encodings, ordering, concurrency, auth/RLS role.

## Out of scope

What this unit deliberately leaves to other units or levels. Anything not
served returns 501 via `megabase_core::MegabaseNotImplemented`.

## Judge cases

Case files (separate `review/issue-<n>-cases` PR) that exercise this spec.
