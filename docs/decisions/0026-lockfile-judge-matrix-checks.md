# Lockfile wait accepts Judge matrix checks

- Status: Accepted (agent coordinator)
- Date: 2026-10-10

Release → Sync Cargo.lock polls the lockfile commit until required check
runs exist. It does not wait for them to pass. After the Judge workflow
became a matrix ([0021](0021-ci-nextest-mold-judge-matrix.md)), the job
named `Judge` is an aggregate that starts only after every
`Judge (<service>)` job finishes. Run
[38013744121](https://github.com/Zouhairmaj/megabase/actions/runs/38013744121)
dispatched Judge with `GITHUB_TOKEN` on
`9988dd8eac61469c95b20fb11fc2f5cd034898a3`. Matrix checks such as
`Judge (auth)` existed within about two minutes. The aggregate `Judge`
check was created about nine minutes after dispatch, after the 60×6s
wait had already failed with `missing required: Judge`.

The wait treats Judge as started when any check on that SHA is named
`Judge` or begins with `Judge (`. `Judge build` and `Judge services` do
not count: they start before any case file runs. Build, Codecov,
Bencher, and Protected paths still require those exact names. The
default window is 180 attempts of 6s (about eighteen minutes), so a
slow `Judge build` can still create the matrix jobs. The knobs remain
`LOCKFILE_CHECK_ATTEMPTS` and `LOCKFILE_CHECK_SLEEP_SECONDS`. Branch
protection can keep requiring the aggregate name `Judge`. This wait
only proves the workflow was dispatched onto the lockfile SHA.

`RELEASE_PLEASE_TOKEN` stays a human-only classic PAT (`repo` and
`workflow`). A fine-grained PAT cannot write this public user-owned
repository, so the job keeps dispatching with `GITHUB_TOKEN` when the
tree probe falls back. That dispatch is what creates the matrix checks
this wait accepts. Humans replace the secret; this workflow does not.
