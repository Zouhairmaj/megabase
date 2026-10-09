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
- **Action**: (pending) Repo Settings → Actions → General → Workflow permissions → enable “Allow GitHub Actions to create and approve pull requests” (and ensure the release workflow can open/update release PRs).
- **Reason**: main job release-please failed with “GitHub Actions is not permitted to create or approve pull requests” (run 37961663758). PR #137 was opened by megabase-agent as a workaround; the main check stays red until Actions can create PRs (or the workflow uses a permitted token).
- **Files affected**: GitHub Actions settings (not in git)

---

*No completed human interventions recorded yet.*
