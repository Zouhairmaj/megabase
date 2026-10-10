# Stop publishing token spend and cost

- Status: Accepted (owner, Zouhair)
- Date: 2026-10-10

## Context

`MANIFESTO.md` rule 7 said the public record includes token spend and
cost, and "How progress is measured" listed tokens and money. The site
already omitted that language (pull request #130). `GOAL.md` still told
the weekly report to include cost.

## Decision

Rule 7 stays the publicity rule. It no longer mentions token spend or
cost: the code, the agent prompts, the loop and the logs, in real time.
The cost bullet under "How progress is measured" is removed. Weekly
reports do not list cost. No later rule existed, so nothing was
renumbered.

`review/*` may modify `GOAL.md` or `MANIFESTO.md` only when the same
diff appends a Completed `HUMAN_LOG.md` entry that names that file in
backticks.
The reviewer agent still reviews the branch. Other edits to those files
stay rejected. See [ADR 0003](../adr/0003-protected-paths.md).

## Consequences

The site manifesto renderer copies those sentences through. Published
manifesto text matches `MANIFESTO.md`. The human log records this
decision.
