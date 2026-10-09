# One file per decision

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

The numbered list under `PROGRESS.md` was a merge-conflict hotspot: every
new decision edited the same lines. Each decision is a file in this
directory. `PROGRESS.md` keeps phase, pins, status, level gates, and
tracking. Agents add `NNNN-slug.md`. They do not append to a shared list.

GOAL.md §10 and §11 still say `PROGRESS.md` holds decisions. That file is
human-only and was left unchanged. `PROGRESS.md` records the divergence so
the two instructions are visible in one place.
