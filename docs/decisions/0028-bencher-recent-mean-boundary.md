# Latency alert at 1.75× the recent mean

- Status: Accepted (agent coordinator)
- Date: 2026-10-10

## Context

[Decision 0027](0027-bencher-ttest-latency.md) replaced the 20 µs static
ceiling with a one-sided t-test (upper boundary `0.95`, at most the latest
16 results). Pull request #187 changes judge cases only. Its Bencher run
still alerted:

| Route | Result | Baseline | 0.95 limit |
|---|---|---|---|
| `GET /_megabase/health` | 5.31 µs | 2.27 µs | 4.79 µs |
| `GET /rest/v1/todos` | 6.61 µs | 3.29 µs | 6.00 µs |

The baseline is the mean of the 16 results ending at the pull request's
start point (`d7d4c54`). Twelve of those are the pre-layer results around
1.1–1.8 µs. Four are the post-layer results around 3.5–5.4 µs. The 0.95
prediction interval lands below results `main` had already recorded
(health 5.35 µs, REST 6.71 µs). `--thresholds-reset` replaced the model.
It did not drop history, and the sample-size minimum of 8 was met. The
boundary was too tight for the mixed window, not short of samples.

A t-test at `0.99` on that same window only moves the health limit to
about 6.0 µs. That passes 5.31 µs and still sits under a slower runner.
Once the window is a tight cluster, a t-test narrows further and a
different GitHub-hosted runner alerts again. With 50 Criterion samples
the within-run interval is a few tens of nanoseconds. The spread that
matters is between runs: post-layer health on `main` is 3.53–5.37 µs.

## Decision

Latency uses a percentage test with upper boundary `0.75`, minimum sample
size 6, and maximum sample size 6. The limit is 1.75× the mean of the six
most recent results. The flags are in `.github/workflows/bencher.yml` and
the lockfile Bencher step. `--thresholds-reset` drops the t-test model.

On current `main` those six health results average 4.76 µs, so the limit
is 8.34 µs. The six REST results average 5.96 µs, so the limit is 10.42
µs. Twice either mean is above its limit. The #187 results, including
against the older start point, are below 1.75× that window's mean.

The Criterion sample (50 samples, 1 second warm-up, 3 seconds of
measurement) is unchanged.

## Consequences

A no-op pull request can move with the runner and stay under the limit.
A result at twice the recent mean alerts. Results older than the latest
six do not set the mean, so the pre-layer samples age out. The test does
not run until six historical results exist. Fork pull requests still skip
without the Bencher secret. This replaces the 0.95 t-test in decision
0027.
