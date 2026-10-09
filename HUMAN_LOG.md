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

Human actions that have not happened yet. The deploy workflow skips cleanly until these secrets exist.

- **Date**: 2026-10-09
- **Action**: (pending) Create a Cloudflare API token with **Account → Cloudflare Workers → Edit**, then add GitHub Actions repository secrets `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`. After the first successful deploy, record the `*.workers.dev` URL here.
- **Reason**: The repository is private, so GitHub Pages cannot host the placeholder site. Deploy is Cloudflare Workers static assets (`wrangler.toml`, worker name `megabase-site`). The token needs Workers Edit so Wrangler can upload the generated `./site` assets.
- **Files affected**: GitHub repository secrets (not committed)

---

*No completed human interventions recorded yet.*
