## Summary

## Coverage / conformance

- coverage:
- conformance:

PR titles follow Conventional Commits as specified in [`AGENTS.md`](AGENTS.md)
(allowed types and scopes are those in `.github/workflows/semantic-pr.yml`).
PRs are squash-merged; the title becomes the changelog entry. The squash
commit subject is that title (`squash_merge_commit_title=PR_TITLE` in
[`docs/ROADMAP.md`](docs/ROADMAP.md#versioning-and-releases)), not a
`[component] unit:` subject.
