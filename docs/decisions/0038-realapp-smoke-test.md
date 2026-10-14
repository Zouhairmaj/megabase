# Real-app smoke test with the official client, inline in a workflow

- Status: Accepted
- Date: 2026-10-14

Issue [#320](https://github.com/Zouhairmaj/megabase/issues/320). The Judge
reported 100 % on the Auth endpoints while the official
`@supabase/supabase-js` client failed against Megabase (null request
fields, CORS preflight, `.single()` on writes). The Judge compares one
request at a time; a client stacks them and depends on headers and
response shapes no single case asserted.

**Decision.** `.github/workflows/realapp.yml` (check `Real app
(supabase-js)`) builds Megabase from the pull request, starts the
reference stack's PostgreSQL image on its own, applies the
`nextjs-todo-list` example migration, and runs the 24-step scenario from
the issue through `@supabase/supabase-js` at the tag `vendor.toml` pins
for `vendor/supabase-js`. The two browser preflights the example sends
are checked with `fetch`. Every step is recorded in the job summary; the
process exits non-zero when any step fails.

**The script is inline.** GOAL.md rule 1 and the Protected paths guard
(`tools/megabase-guard`, `NON_RUST_CODE`) reject a `.js`, `.mjs`, `.ts`
or `.sh` file anywhere outside `vendor/`. The scenario therefore lives as
a heredoc in the workflow's `run:` block, the same way `judge.yml` and
`ci.yml` carry inline Python. The test is not Megabase code: it is the
upstream client exercising the server, and the only way to exercise the
real client is to run it. Carving a `tests/realapp/` exception into the
guard would widen a mission rule, which is a human decision.

**Not required yet.** The job reports success while `REALAPP_REQUIRED`
in the workflow is `"false"`; a failed scenario is a warning annotation
plus the summary table. Set it to `"true"` once the client bugs #320
lists are closed, so the check fails on any step failure.

**Database.** The job starts `public.ecr.aws/supabase/postgres` at the
digest `judge/compose.override.yml` pins instead of a stock library
image: that digest is already verified and Renovate-tracked, and no
verified digest for a stock image was at hand. Megabase bootstraps the
API roles on a stock PostgreSQL ([0037](0037-bootstrap-standard-roles.md));
on this image the roles exist and the idempotent bootstrap must still
succeed. Switching to a stock image is a one-line change once a digest
is pinned.

**Not chosen.** A Rust port of the scenario: it would not catch
client-side behaviour such as `.single()` headers or `x-client-info`.
Running the scenario inside the Judge: the harness compares two stacks
and does not import a JavaScript client. A `package-lock.json`: the
install is one exact version with scripts disabled, and a lockfile in
the tree would be a second dependency surface for Renovate and
Scorecard.
