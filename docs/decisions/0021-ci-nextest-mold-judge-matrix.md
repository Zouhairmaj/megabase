# CI tests, mold, and a per-service judge

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

GitHub Actions test steps run `cargo nextest run --workspace --locked`.
Doctests stay on `cargo test --doc --workspace --locked` because nextest
does not run them. `nextest` is installed with
`taiki-e/install-action` pinned by commit SHA. Local `just test` remains
`cargo test`.

Linux CI and the Cursor Cloud image link with mold when the `mold` binary
is installed (`.github/actions/linux-mold`, `CARGO_TARGET_*_RUSTFLAGS`, and
`$CARGO_HOME/config.toml` in the Cloud image). A machine without mold keeps
the default linker. Release musl cross builds do not set those flags.

The Judge workflow runs one job per `judge/cases/*.toml` file
(`Judge (auth)`, `Judge (rest)`, …) so a failure names the service file.
A final job named `Judge` merges the results. That name is the required
status check.
