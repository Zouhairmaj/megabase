# Docker library bases and Kong from ghcr.io mirrors, ECR fallback

- Status: Accepted
- Date: 2026-10-15

Issue [#358](https://github.com/Zouhairmaj/megabase/issues/358).
[Decision 0039](0039-ghcr-postgres-mirror.md) mirrored only the
reference Postgres image. The Container image job still failed on the
`Dockerfile` `FROM` lines, and a Judge run on `main` failed pulling
Kong. All three images come from `public.ecr.aws/docker/library`, and
public ECR answers the anonymous data quota with `toomanyrequests`
("Data limit exceeded").

**Decision.** Mirror these images to ghcr.io at the same tag and
digest, and pull the mirror first with ECR as the fallback:

| Consumer | Mirror | Upstream |
| --- | --- | --- |
| `Dockerfile` build stage | `ghcr.io/zouhairmaj/rust` | `public.ecr.aws/docker/library/rust` |
| `Dockerfile` runtime | `ghcr.io/zouhairmaj/debian` | `public.ecr.aws/docker/library/debian` |
| Judge `kong` service | `ghcr.io/zouhairmaj/kong` | `public.ecr.aws/docker/library/kong` |
| Judge `db` service (0039) | `ghcr.io/zouhairmaj/supabase-postgres` | `public.ecr.aws/supabase/postgres` |

- `.github/image-mirrors.txt` maps each mirror repository to its
  upstream. Tags and digests stay where they were written: the `FROM`
  lines and `judge/compose.override.yml`.
- `.github/workflows/mirror-images.yml` (replaces
  `mirror-postgres-image.yml`) copies every
  `ghcr.io/zouhairmaj/<repo>:<tag>@<digest>` reference in those two
  files with `skopeo copy --all --preserve-digests`, then checks the
  digest and whether an anonymous pull works. It runs on
  `workflow_dispatch` and on pushes to `main` that change either file,
  the table or the workflow.
- `.github/actions/judge-images` (replaces `judge-db-image`) pulls each
  `${MEGABASE_JUDGE_<NAME>_IMAGE:-ghcr.io/zouhairmaj/…}` image in the
  overlay, falls back to the upstream at the same digest, and exports
  the variable so Compose uses the pulled reference. Real app resolves
  only `MEGABASE_JUDGE_DB_IMAGE`.
- `.github/actions/dockerfile-bases` pulls the `FROM` mirrors. When one
  cannot be pulled it pulls the upstream and writes a copy of the
  `Dockerfile` under `$RUNNER_TEMP` with that `FROM` retargeted; the
  Container image job builds the file it returns. The `FROM` lines stay
  literal, so Scorecard Pinned-Dependencies still sees tag and digest.
- `megabase-judge` has a test that every mirror reference in the overlay
  and the `Dockerfile` is listed in the table, and that overlay mirrors
  go through a `MEGABASE_JUDGE_*_IMAGE` variable.

**Running the mirror once.** After this merges, the push to `main` runs
the workflow. To run it by hand: Actions, Mirror images, Run workflow
on `main` (or `gh workflow run mirror-images.yml -R Zouhairmaj/megabase
--ref main`). Each new package (`rust`, `debian`, `kong`) starts
private: open it under the owner's Packages, Package settings, Change
visibility, Public. The workflow warns until each one is public, and CI
keeps using the ECR fallback until then.

**Bumping an image.** Change the tag and digest in the `FROM` line or
the overlay only. Renovate looks up the Dockerfile bases on ECR
(`overridePackageName` in `renovate.json`), because the mirror holds
only pinned tags. The push to `main` runs the mirror; until it
finishes, CI uses the ECR fallback.

**Not chosen.**
- `ARG` base images with `--build-arg` overrides: a `FROM ${VAR}` line
  hides the pin from Scorecard Pinned-Dependencies and Renovate.
- BuildKit `--build-context` overrides keyed by the `FROM` reference:
  matching a reference that carries a digest is not documented, and a
  retargeted copy is plain text that any builder reads.
- Changing the release workflow: it logs in to ghcr.io and builds after
  the mirror has run on `main`, and a backfilled tag can predate the
  mirrored `FROM` lines.
