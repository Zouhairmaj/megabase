# Continuous benchmarking

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

Criterion benches start with a trivial gateway health/route measurement.
Bencher project `megabase` (created on the fly if missing) tracks `main` and
PRs (`rust_criterion`, t-test upper boundary 0.99, `--error-on-alert`). Fork
PRs skip without the secret.
