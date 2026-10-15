# judge/: the external arbiter

Extends the root `AGENTS.md` and does not relax it. **This directory is
protected.** The root file defines who may change it, including the
bootstrap exception. Architecture and commands are in
[`README.md`](README.md).

- **Cases** (`cases/<component>.toml`): write one `[[case]]` per behavior.
  Take `units = [...]` from `coverage/units.json` and run the steps against
  `fixtures/schema.sql`. A case may be HTTP-only, `[[case.db]]`-only
  (schema objects), or both.   Use `absent = true` when the pinned
  reference dropped the object. A required object missing on both
  databases fails. Mutating HTTP cases compare the before/after row
  delta of `auth.users` and `public.todos` unless `snapshot` overrides
  that. Every case must
  pass on the reference stack. Expected behavior always comes from the
  reference, never from Megabase. The harness talks to Postgres itself
  and does not import Megabase crates.
- **Never** delete, loosen or skip a case, or add an `ignore` that hides a
  real difference.
- **Held-out suite** (`megabase-judge hidden`): cases are not files in
  this tree. Do not log, upload, or commit a seed, a sealed blob's
  plaintext, or a per-case diff. The weekly workflow publishes counts
  only. `hidden-open` stays out of CI.
- A new normalization rule must apply to both stacks equally and be
  documented in `NORMALIZATION.md` in the same PR.
- Harness tests: `cargo test -p megabase-judge`.
- `compose.override.yml` must pin every reference-stack `image:` to
  `ghcr.io/…@sha256:…`. Do not pull from Docker Hub (anonymous 429s
  fail CI) or from `public.ecr.aws` (its data quota fails CI) except as
  the fallback. Every image published on ECR is
  `${MEGABASE_JUDGE_<NAME>_IMAGE:-ghcr.io/zouhairmaj/<repo>:…}`, a
  mirror of the ECR image at the same digest, listed in
  `.github/image-mirrors.txt`
  ([decisions 0039](../docs/decisions/0039-ghcr-postgres-mirror.md),
  [0040](../docs/decisions/0040-ghcr-library-image-mirrors.md) and
  [0041](../docs/decisions/0041-ghcr-supabase-image-mirrors.md)).
  The tag and digest live in `judge/compose.override.yml` and in the
  `Dockerfile` `FROM` lines; the table holds only
  the repository mappings. To bump one, change the tag and digest in
  those files, then let the mirror workflow run on `main`. The Kong service uses
  `entrypoint: !override` so Compose replaces the vendor script
  instead of appending it. The wrapper points that script at
  `/docker-entrypoint.sh` on the Docker library image.
