# CI compiler cache on R2, and the nextest CI profile

- Status: Accepted (agent coordinator)
- Date: 2026-10-10

Compiling jobs install sccache 0.18.0 with
`mozilla-actions/sccache-action` pinned to commit `fc920bf0ec8de6ee65d409111f7ec508035751ba`
(`.github/actions/sccache`). `RUSTC_WRAPPER=sccache` and
`CARGO_INCREMENTAL=0`.

Pushes and same-repository pull requests store compiler artifacts in the
R2 bucket named by the `SCCACHE_BUCKET` repository variable. The S3
endpoint is `SCCACHE_ENDPOINT`, the region is `auto`, and TLS is on.
Keys are `SCCACHE_R2_ACCESS_KEY_ID` and `SCCACHE_R2_SECRET_ACCESS_KEY`.
Composite actions cannot read `secrets` or `vars`, so each workflow
exports the keys and the bucket variables into the environment and the
action reads that. Object keys use the prefix `sccache`. Those runs are
read-write so a later job can hit what an earlier job wrote. Fork pull requests do not
receive repository secrets. The action leaves `SCCACHE_BUCKET` unset in
that case and sccache uses its local disk, so the job still passes. This
workflow does not use `pull_request_target`. `SCCACHE_GHA_ENABLED` stays
false: sccache rejects two remote backends, and the shared cache is R2.

`Swatinem/rust-cache` still caches the Cargo registry and git database.
`cache-targets` is false. A restored `target/` directory skips rustc, so
sccache would not see those compilations and the R2 bucket would stay
cold. Profile suffixes remain on the rust-cache key so a later change
that caches `target/` again cannot mix an llvm-cov or bencher directory
with a normal build.

The container image compiles inside `Dockerfile` (`docker build` in CI,
`docker/build-push-action` on release). sccache is not installed in that
build. Passing the R2 keys as BuildKit secrets, and keeping the build
working when those secrets are absent, is a second credential path in the
image. Docker layer caching already covers an unchanged source tree. The
runner jobs are where sccache runs.

Test binaries in CI are `cargo nextest` (install-action tool
`cargo-nextest`; `nextest` remains an alias). Doctests stay
`cargo test --doc` ([0021](0021-ci-nextest-mold-judge-matrix.md)).
`.config/nextest.toml` profile `ci` sets `fail-fast = false`,
`retries = 2`, and a 60s slow-timeout that terminates after three
periods. A timeout or a test that fails every attempt fails the job.
Codecov keeps nextest coverage and adds doctests. `cargo llvm-cov --doc`
and `report --doctests` need nightly (`-Z persist-doctests`), so the job
installs nightly and runs nextest, `--doc`, and the report with
`cargo +nightly llvm-cov`. One nightly llvm-profdata merges both.
The Build job still executes doctests with `cargo test --doc`.
