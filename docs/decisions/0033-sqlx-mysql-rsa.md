# sqlx-mysql patch drops RUSTSEC-2023-0071

- Status: Accepted (agent coordinator)
- Date: 2026-10-10

OpenSSF Scorecard's vulnerability check reads every tracked `Cargo.lock`.
`rsa` 0.9.10 (RUSTSEC-2023-0071, Marvin timing side channel, no patched
release) was in the workspace lockfile because `sqlx` 0.8.6 records optional
`sqlx-mysql`, and that driver depended on `rsa` unconditionally. Megabase
enables only the postgres driver, so the binary never linked `rsa`.
`cargo audit` ignored the advisory in `.cargo/audit.toml`. Scorecard does
not read that ignore list.

`sqlx` 0.9 makes RSA auth opt-in (`mysql-rsa`) and a postgres-only lockfile
then omits `rsa`. That release sets `rust-version` to 1.94. This workspace
MSRV is 1.89, and the MSRV job lives under `.github/` (a protected path).
The 0.8.6 driver is patched in `third_party/sqlx-mysql` instead: `rsa` is an
off-by-default feature, `[patch.crates-io]` replaces the crates.io crate, and
the audit ignore is gone. Non-TLS MySQL `caching_sha2_password` /
`sha256_password` returns a configuration error unless that feature is
enabled. TLS MySQL auth and every postgres path are unchanged. Megabase does
not enable MySQL.
