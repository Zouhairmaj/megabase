# Every ECR reference image from a ghcr.io mirror, ECR fallback

- Status: Accepted
- Date: 2026-10-15

Issue [#371](https://github.com/Zouhairmaj/megabase/issues/371).
[Decision 0040](0040-ghcr-library-image-mirrors.md) mirrored Postgres,
Kong and the `Dockerfile` bases. The other Supabase images in the Judge
overlay were still pulled from `public.ecr.aws`, and Judge jobs kept
failing on its anonymous data quota (`toomanyrequests: Data limit
exceeded`).

**Decision.** Every Judge reference image that is published on
`public.ecr.aws` is pulled from a ghcr.io mirror at the same tag and
digest, with the 0040 mechanism unchanged: one line per repository in
`.github/image-mirrors.txt`, `mirror-images.yml` copies each pinned
reference (`skopeo copy --all --preserve-digests`), and
`.github/actions/judge-images` falls back to ECR. New mirrors:

| Judge service | Variable | Mirror | Upstream |
| --- | --- | --- | --- |
| `studio` | `MEGABASE_JUDGE_STUDIO_IMAGE` | `ghcr.io/zouhairmaj/supabase-studio` | `public.ecr.aws/supabase/studio` |
| `auth` | `MEGABASE_JUDGE_AUTH_IMAGE` | `ghcr.io/zouhairmaj/supabase-gotrue` | `public.ecr.aws/supabase/gotrue` |
| `realtime` | `MEGABASE_JUDGE_REALTIME_IMAGE` | `ghcr.io/zouhairmaj/supabase-realtime` | `public.ecr.aws/supabase/realtime` |
| `storage` | `MEGABASE_JUDGE_STORAGE_IMAGE` | `ghcr.io/zouhairmaj/supabase-storage-api` | `public.ecr.aws/supabase/storage-api` |
| `meta` | `MEGABASE_JUDGE_META_IMAGE` | `ghcr.io/zouhairmaj/supabase-postgres-meta` | `public.ecr.aws/supabase/postgres-meta` |
| `functions` | `MEGABASE_JUDGE_FUNCTIONS_IMAGE` | `ghcr.io/zouhairmaj/supabase-edge-runtime` | `public.ecr.aws/supabase/edge-runtime` |
| `supavisor` | `MEGABASE_JUDGE_SUPAVISOR_IMAGE` | `ghcr.io/zouhairmaj/supabase-supavisor` | `public.ecr.aws/supabase/supavisor` |

`rest` (`ghcr.io/postgrest/postgrest`) and `imgproxy`
(`ghcr.io/imgproxy/imgproxy`) are already published on ghcr.io by
their upstreams and are not mirrored.

- The tags and digests in `judge/compose.override.yml` did not change;
  each reference only moved into
  `${MEGABASE_JUDGE_<NAME>_IMAGE:-ghcr.io/zouhairmaj/…}`.
- The `megabase-judge` overlay test now rejects any default image that
  is not on ghcr.io, so ECR is used only as the CI fallback.
- **Retry budget.** `judge-images` keeps one 20-minute budget, but now
  resolves in two passes so a slow fallback cannot starve the images
  after it. Pass 1 pulls every mirror (3 attempts, at most 2 minutes
  each) and stops with 8 minutes left. Pass 2 pulls the upstream of
  each failed mirror, and each fallback leaves one minute per fallback
  still queued after it. Docker library images keep the 0040 order (ECR
  for 4 attempts, leaving 4 more minutes, then Docker Hub). This
  replaces the per-image budget described in 0040.

**Running the mirror once.** The push to `main` runs Mirror images.
Each new package starts private: open it under the owner's Packages,
Package settings, Change visibility, Public. Until then the workflow
warns and CI uses the ECR fallback.

**Not chosen.**
- Pulling images in parallel: ECR answers bursts with
  `toomanyrequests: Rate exceeded`, and ghcr pulls are fast enough one
  at a time.
- A second mirror table or workflow for the Supabase images: the 0040
  table and workflow are generic.
