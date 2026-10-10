# No coverage push to main; judge images off Docker Hub

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

The `Update coverage on main` job only rewrote `coverage/judge-results.json`
(PRs already commit regenerated coverage). It is removed. Live badge JSON
is published to the `gh-pages` branch and `_site/coverage/` for shields.io.
Judge compose pulls digest-pinned images from `public.ecr.aws` / `ghcr.io`
so anonymous Docker Hub 429s do not fail CI. Root `Dockerfile` `FROM` lines
use `public.ecr.aws/docker/library` at the same digests. `cargo deny` runs
on the runner (`taiki-e/install-action`), not via
`EmbarkStudios/cargo-deny-action` (that action builds
`docker.io/library/rust` and hits the same anonymous 429). When
`DOCKERHUB_TOKEN` is set, Judge, the container image job, cargo-deny, and
the lockfile compose job log in to Docker Hub first (`docker/login-action`
v4.6.0, via `.github/actions/dockerhub-login`) and skip login when the
secret is empty. A timeout talking to `auth.docker.io` is retried, then
ignored, so the digest-pinned mirrors still pull. The ECR library Kong
image has no `/entrypoint.sh`. The compose override uses
`entrypoint: !override` (Compose would otherwise append the vendor
entrypoint, which execs the missing path and leaves `supabase-kong`
unhealthy) and rewrites that one path to `/docker-entrypoint.sh`.
Pages publication of live Judge results is
`.github/workflows/pages-badges.yml` (`workflow_run` only, no cache
action). It checks out the default branch and does not set
`actions/checkout` `ref` from the triggering run (Scorecard
Dangerous-Workflow treats that ref as an untrusted checkout). It
applies the Judge JSON as data when that commit is the checkout or an
ancestor of it, because `main` usually moves during the Judge run. An
equality check was dropping every result. It then publishes shields
JSON, the README treemap PNGs, and the site. A Judge commit that is
not contained in the checkout is skipped. The JSON stays data: the
cached Pages job never downloads it. `.github/workflows/pages.yml` is
push and `workflow_dispatch` only, checks out the event SHA, and does
not download `gh-pages` or a Judge artifact. It saves the
default-branch Rust cache on pushes to `main` and does not deploy
that build. Publication, `judge-history.json`, and the `pages`
concurrency group belong to `pages-badges.yml`
([decision 0030](0030-conformance-is-the-live-judge-score.md)).
