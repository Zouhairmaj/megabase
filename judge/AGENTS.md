# judge/: the external arbiter

Extends the root `AGENTS.md` and does not relax it. **This directory is
protected.** The root file defines who may change it, including the
bootstrap exception. Architecture and commands are in
[`README.md`](README.md).

- **Cases** (`cases/<component>.toml`): write one `[[case]]` per behavior.
  Take `units = [...]` from `coverage/units.json` and run the steps against
  `fixtures/schema.sql`. A case may be HTTP-only, `[[case.db]]`-only
  (schema objects), or both. Use `absent = true` when the pinned
  reference dropped the object. Mutating HTTP cases snapshot `auth.users`
  and `public.todos` unless `snapshot` overrides that. Every case must
  pass on the reference stack. Expected behavior always comes from the
  reference, never from Megabase. The harness talks to Postgres itself
  and does not import Megabase crates.
- **Never** delete, loosen or skip a case, or add an `ignore` that hides a
  real difference.
- A new normalization rule must apply to both stacks equally and be
  documented in `NORMALIZATION.md` in the same PR.
- Harness tests: `cargo test -p megabase-judge`.
