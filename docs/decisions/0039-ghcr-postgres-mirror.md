# Reference Postgres image from a ghcr.io mirror, ECR fallback

- Status: Accepted
- Date: 2026-10-14

Issue [#350](https://github.com/Zouhairmaj/megabase/issues/350). Judge,
held-out Judge, the lockfile Judge job and Real app pull
`public.ecr.aws/supabase/postgres`. It is the largest image in the
stack, and public ECR answers bursts with `toomanyrequests`, so jobs
failed and had to be rerun.

**Decision.** `.github/workflows/mirror-postgres-image.yml` copies the
image to `ghcr.io/zouhairmaj/supabase-postgres` at the same tag with
`skopeo copy --all --preserve-digests`. It copies every platform,
refuses to change the digest, and then checks the pushed manifest's
digest. It runs on `workflow_dispatch` and on pushes to `main` that
change `judge/compose.override.yml`. The package is public.

`judge/compose.override.yml` stays the one place the reference is
written:
`${MEGABASE_JUDGE_DB_IMAGE:-ghcr.io/zouhairmaj/supabase-postgres:<tag>@<digest>}`.
The mirror workflow and `.github/actions/judge-db-image` read that line.
The action pulls the mirror and, if that fails, pulls
`public.ecr.aws/supabase/postgres:<tag>@<digest>` and exports
`MEGABASE_JUDGE_DB_IMAGE` so Compose and Real app use it. The digest is
the same on both registries, so the Judge runs the same bytes either way.
Locally, `just judge-up` pulls the mirror. To use ECR instead, set
`MEGABASE_JUDGE_DB_IMAGE` to the ECR reference.

**Bumping the image.** Change the tag and digest in the overlay only.
The push to `main` runs the mirror. Until it finishes, CI uses the ECR
fallback.

**Not chosen.**
- `crane` through `imjasonh/setup-crane`: every action here is pinned by
  commit SHA, and `skopeo` already ships on `ubuntu-latest`, so it adds
  no third-party action.
- Mirroring every reference image: only Postgres was named in the
  failures, and the per-image pull loop already retries the others.
