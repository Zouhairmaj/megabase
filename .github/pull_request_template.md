## Summary

## Coverage / conformance

- coverage:
- conformance:

PR titles follow Conventional Commits as specified in [`AGENTS.md`](AGENTS.md)
(allowed types and scopes are those in `.github/workflows/semantic-pr.yml`).
PRs are squash-merged; the title is the changelog entry. The squash
commit subject must be that Conventional Commits title, not a
`[component] unit:` subject. The repository still has
`squash_merge_commit_title=COMMIT_OR_PR_TITLE`, so a one-commit squash
keeps the commit subject until a human sets `PR_TITLE`
([`docs/ROADMAP.md`](docs/ROADMAP.md#versioning-and-releases)).
