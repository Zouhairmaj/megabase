# Board Status reflects reality

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

Agents claim an issue (assign + **In progress**, branch `issue-<n>-<slug>`,
PR `Closes #<n>`) before coding. `.github/workflows/board-sync.yml` mirrors
Status from those signals plus `blocked`. `GOAL.md` is human-owned; the
contract is in `AGENTS.md`. `tools/megabase-backlog` is the idempotent
source of truth for the board: match `<!-- megabase-id -->`, GraphQL Status
option ids (including Blocked), sub-issues and blocked-by.
