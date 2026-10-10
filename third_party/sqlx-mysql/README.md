# sqlx-mysql 0.8.6 (patched)

Unmodified crates.io `sqlx-mysql` 0.8.6, except the `rsa` crate is an
off-by-default feature. The workspace `[patch.crates-io]` entry points here
so `Cargo.lock` does not contain `rsa` (RUSTSEC-2023-0071).

Megabase does not enable the feature. See
[docs/decisions/0033-sqlx-mysql-rsa.md](../../docs/decisions/0033-sqlx-mysql-rsa.md).

License: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).
