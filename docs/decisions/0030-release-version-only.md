# Release commits only bump versions

- Status: Accepted (agent coordinator)
- Date: 2026-10-10

Release pull request #209 (`e80025f`, parent `846b842`) deleted workspace
dependencies `chacha20` and `hkdf` from the root `Cargo.toml`.
`cargo metadata --locked --no-deps` then failed, and the lockfile job
failed before it could commit: `dependency.chacha20 was not found in
workspace.dependencies`. That job only stages `Cargo.lock`. It did not
rewrite `Cargo.toml`.

release-please 17.3.0 (action `v4.4.0`) reads files through
`RepositoryFileCache`, which keys the git tree by the branch name
`main`. Workflow run 38031312690 was the push of `434cc3d`. At
06:32:55Z it fetched `release-please-config.json` from `main` and cached
that tree. `846b842` (which adds the two dependencies) reached `main` at
06:32:58Z. At 06:33:00Z the same run created `e80025f` with parent
`846b842` and the cached `Cargo.toml`, version bumped to `0.1.5`.
`replaceTomlValue` only replaces the version span. The deletion is the
stale file written onto the newer tree.

The lockfile job now rebuilds `Cargo.toml` from the merge-base with the
default branch and changes only `[workspace.package] version` to the
manifest version, then runs `cargo update -w`. The push uses
`RELEASE_PLEASE_CLASSIC_TOKEN` when that secret is set (classic PAT,
scopes `repo` and `workflow`). If it is absent, or that probe returns
HTTP 403 or 404 that is not a rate limit, the job probes
`RELEASE_PLEASE_TOKEN`, then `GITHUB_TOKEN`. A classic-token push to the
release branch starts pull_request workflows. `megabase-guard` rejects
a release-please diff that changes anything else in `Cargo.toml` or
`Cargo.lock` (changelog and `.release-please-manifest.json` stay
allowed). See [ADR 0003](../adr/0003-protected-paths.md).

#207 was squash-merged as `[rest] filtering: ...` because
`squash_merge_commit_title` is `COMMIT_OR_PR_TITLE` and the pull request
had one commit. conventional-commits-parser rejects that subject, so
release-please omitted it from 0.1.5. The repository setting has to
become `PR_TITLE` (human-only; the token got HTTP 403). #207's body
contains `BEGIN_COMMIT_OVERRIDE` /
`feat(rest): add imatch, in, is, like, lt, and not filters (#207)` /
`END_COMMIT_OVERRIDE`. The `(#207)` is the issue reference
release-please prints beside the commit. The 0.1.5 changelog uses that
subject.
