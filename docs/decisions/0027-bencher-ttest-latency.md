# Student's t-test for gateway latency

- Status: Accepted (agent coordinator)
- Date: 2026-10-10

## Context

[Decision 0026](0026-bencher-static-latency-ceiling.md) set a static upper
boundary of 20,000 ns (20 µs). That boundary is an explicit limit. It does
not move when recent results sit around 4–8 µs, so either smoke route can
slow to just under 20 µs (about 2.5–5×) without an alert. Codex noted this
on pull request #185.

A percentage boundary is a fixed multiple of the historical mean. While the
window still mixes the pre-layer mean (about 1.6 µs) with later results, a
multiple tight enough to catch a 2× jump false-alerts. The same multiple
is too loose once the mean itself has moved up.

The Bencher `latency` measure is nanoseconds. For a t-test, the upper
boundary is a cumulative confidence: `0.5` is the mean and `1.0` is +∞.

## Decision

Latency uses a one-sided Student's t-test (`t_test`) with upper boundary
`0.95`, minimum sample size 8, and maximum sample size 16. The flags are
in `.github/workflows/bencher.yml` and the lockfile Bencher step.
`--thresholds-reset` drops the static model.

`0.95` is narrower than the earlier `0.99` boundary. On a small window,
`0.99` can place the prediction interval past twice the recent mean.
`0.95` still widens when runners disagree, and it fails when a result
jumps clear of that spread.

The Criterion `health` bench collects 50 samples, with a 1 second warm-up
and 3 seconds of measurement. Bencher stores the run's point estimate. The
longer sample keeps one internal outlier from moving that estimate.
Differences between GitHub-hosted runners stay in the historical variance
the t-test uses.

## Consequences

A stable recent mean near 4–8 µs alerts on a multi-fold slowdown instead of
waiting for 20 µs. The test does not run until 8 historical results exist,
and it ignores results older than the most recent 16. Fork pull requests
still skip without the Bencher secret. This replaces the static ceiling in
decision 0026.
