# Human Intervention Log

This file records every human action taken on the Megabase repository or running agents.

Per the manifesto: "Any action a human takes on the repository or the running agents (a fix, a nudge, a reverted commit, a changed prompt) is recorded publicly in `HUMAN_LOG.md`, with the reason. The count is part of the result."

## Format

Each entry should include:
- **Date**: YYYY-MM-DD
- **Action**: What was done
- **Reason**: Why it was necessary
- **Files affected**: List of files changed

## Pending

- **Date**: 2026-10-09
- **Action**: (pending) Repository Settings → Pages → Build and deployment → Source: **GitHub Actions**.
- **Reason**: The placeholder site deploys with `actions/configure-pages` (`enablement: true`), `actions/upload-pages-artifact`, and `actions/deploy-pages` on push to `main`. Pages is not enabled yet (`GET /pages` is 404). `enablement: true` cannot turn Pages on with `GITHUB_TOKEN` alone (it needs a PAT or GitHub App token with Pages write). After the source is set to GitHub Actions, re-run **Deploy placeholder site**.
- **Files affected**: GitHub Pages settings (not in git)

- **Date**: 2026-10-09
- **Action**: (pending) Create repository secret `RELEASE_PLEASE_TOKEN`. Settings → Secrets and variables → Actions → New repository secret. Name: `RELEASE_PLEASE_TOKEN`. Value: a fine-grained PAT **or** GitHub App installation token for `Zouhairmaj/megabase` with `contents: write` and `pull-requests: write`. The token must be a user/app credential whose **pushes start GitHub Actions** (the default `GITHUB_TOKEN` does not). Classic PAT equivalent: `repo` scope. After saving, re-run the **Release** workflow on `main` (Actions → Release → Run workflow) so lockfile commits on `release-please--branches--*` pick up the token.
- **Reason**: Preferred path so release-please and `cargo update -w` lockfile commits trigger required checks natively. Release.yml now `workflow_dispatch`es CI when the secret is unset (GITHUB_TOKEN can start `workflow_dispatch`), but pull_request-only checks (for example Conventional Commits title) and a release branch that does not yet contain those `workflow_dispatch` triggers still need this token. Run 37980727582 failed on Sync Cargo.lock waiting for Build, Codecov, Bencher, Protected paths, and Judge on SHA `b8c1e454…`.
- **Files affected**: GitHub Actions repository secrets (not in git)

- **Date**: 2026-10-09
- **Action**: (pending) After the Release workflow publishes `ghcr.io/zouhairmaj/megabase`, set that package to public if GitHub created it private. Then dispatch **Release** from tag `v0.1.0` (Use workflow from = `v0.1.0`, input tag `v0.1.0`) so the first GitHub Release gains signed binaries (it shipped without assets).
- **Reason**: OpenSSF Scorecard Signed-Releases inspects assets on the last five GitHub Releases. Packaging also wants a public package. `attest-build-provenance` records the run SHA, so the backfill must run on that tag. Agents cannot change package visibility or start that dispatch from this environment.
- **Files affected**: GitHub Packages and Actions (not in git)

- **Date**: 2026-10-10
- **Action**: (pending) Replace repository secret `RELEASE_PLEASE_TOKEN`. The current value is a fine-grained PAT owned by `megabase-agent`. Fine-grained PATs cannot write a public repository owned by another personal account, even for a collaborator. Create a classic PAT with scopes `repo` and `workflow` for an account that can write `Zouhairmaj/megabase`, save it as `RELEASE_PLEASE_TOKEN`, and re-run **Release** on `main`. This supersedes the 2026-10-09 pending item that allowed a fine-grained PAT for this secret.
- **Reason**: Run 38007606007 Sync Cargo.lock failed with HTTP 403 `Permission to Zouhairmaj/megabase.git denied to megabase-agent`. GitHub's personal access token documentation says only a classic PAT has write access for a public repository you do not own. Granting Contents write on the fine-grained token does not fix that. `workflow` is included so the token can push workflow-file changes; the failed commit changed only `Cargo.lock`.
- **Files affected**: GitHub Actions repository secrets (not in git)

---

## Completed

- **Date**: 2026-10-09
- **Action**: Enabled “Allow GitHub Actions to create and approve pull requests” (Settings → Actions → General → Workflow permissions). Confirmed via API: `can_approve_pull_request_reviews=true`.
- **Reason**: Release run 37961663758 failed with “GitHub Actions is not permitted to create or approve pull requests” before this setting was on. The pending item recorded that failure; the setting is now enabled. Lockfile-head checks still need `RELEASE_PLEASE_TOKEN` (`GITHUB_TOKEN` pushes do not trigger workflows).
- **Files affected**: GitHub Actions settings (not in git)

- **Date**: 2026-10-09
- **Action**: Created repo secret `SCORECARD_TOKEN` (fine-grained PAT, read-only: Administration, Contents, Metadata, Pull requests).
- **Reason**: OpenSSF Scorecard's Branch-Protection check cannot read classic branch protection rules with `GITHUB_TOKEN` alone. The workflow now passes `repo_token` from secret `SCORECARD_TOKEN` and keeps `publish_results: true`.
- **Files affected**: GitHub Actions secrets (not in git)

- **Date**: 2026-10-09
- **Action**: Zouhair created a Docker Hub account and repository secrets `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN` (read-only PAT).
- **Reason**: Unauthenticated Docker Hub pulls were returning HTTP 429 and failing Judge, the container image build, and cargo-deny. CI logs in with these secrets before image pulls and builds, and skips login when the token is empty (fork pull requests). Digest-pinned `public.ecr.aws` and `ghcr.io` mirrors stay in place.
- **Files affected**: GitHub Actions repository secrets (not in git)

- **Date**: 2026-10-10
- **Action**: Zouhair completed the Phase 0 owner review. Decision: validée avec modifications (validated with modifications). Validated: vendor pins 1.1–1.9 except 1.7; 2.2–2.6; 2.7 (the judge fails on a regression, and level gates use totals); 2.8–2.10; 2.13; 3.1–3.4. Modifications, left as issues and not implemented in the log pull request: 1.7 pin the Studio image to the one named by `vendor/supabase/docker/docker-compose.yml` at supabase `v1.26.08` (`supabase/studio:2026.08.03-sha-022b374`) — #198; 2.1 add a minimum number of judge cases per unit and require the hidden suite (#114) before Level 1 can be validated — #199; 2.11 and 2.12 set cargo-deny `[bans] wildcards` to `deny` — #200; 3.5 JWT normalisation must decode tokens and compare claims (`role`, `aud`, `sub`, `aal`, `amr`, and the rest), ignoring only `iat`, `exp` and identifiers — #201; 3.6 database side-effect checks add ACLs, column defaults and RLS policies — #202; 4.x the public display emphasises progress per level rather than a global percentage — #203. Item 3.9 is done in this review: “Require review from Code Owners” is enabled with admin enforcement (owner, 2026-10-10). Corrections recorded with the review: `vendor.toml` was created on 2026-10-09 during Phase 0 bootstrap and has not been modified since; `CODEOWNERS` exists at the repository root and was not enforced when pull request #1 merged.
- **Reason**: GOAL.md section 6 requires one human review of Phase 0, recorded in this file. The bootstrap is accepted with the follow-ups above. Those follow-ups stay open; this entry does not implement them. The log pull request still needs the code owner’s approval.
- **Files affected**: `HUMAN_LOG.md`, `PROGRESS.md`. GitHub issues #198, #199, #200, #201, #202, #203. Branch protection (not in git): code-owner reviews with admin enforcement.
