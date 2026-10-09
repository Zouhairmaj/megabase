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

---

## Completed

- **Date**: 2026-10-09
- **Action**: Enabled “Allow GitHub Actions to create and approve pull requests” (Settings → Actions → General → Workflow permissions). Confirmed via API: `can_approve_pull_request_reviews=true`.
- **Reason**: Release run 37961663758 failed with “GitHub Actions is not permitted to create or approve pull requests” before this setting was on. The pending item recorded that failure; the setting is now enabled. Lockfile-head checks still need `RELEASE_PLEASE_TOKEN` (`GITHUB_TOKEN` pushes do not trigger workflows).
- **Files affected**: GitHub Actions settings (not in git)
