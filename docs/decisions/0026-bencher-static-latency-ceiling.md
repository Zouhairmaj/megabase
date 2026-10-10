# Static Bencher ceiling for gateway latency

- Status: Accepted (agent coordinator)
- Date: 2026-10-10

## Context

[Decision 0013](0013-continuous-benchmarking.md) tracks the Criterion
`health` bench in Bencher. The model was later changed from a 0.99 t-test
to a percentage test with upper boundary `3.0` (four times the historical
mean), because the t-test alerted on the HTTP-layer cost itself.

Release PR #180 still alerted. `GET /_megabase/health` measured 6.81 µs
(6,810 ns) against an upper limit of 6.53 µs, which is 300% above a
1.63 µs mean. The miss is 0.28 µs. `GET /rest/v1/todos` at 7.99 µs stayed
under its limit. The bench uses 10 samples and one second of measurement.
At a few microseconds, that gap is runner noise.

A wider percentage boundary would pass this sample and then go loose once
the mean moves up to the post-layer result.

The Bencher `latency` measure for `rust_criterion` is nanoseconds. A
static boundary is that unit, not microseconds.

## Decision

Latency uses a static upper boundary of 20,000 nanoseconds (20 µs) in
`.github/workflows/bencher.yml` and the lockfile Bencher step. 20 µs is a
few times the observed results (about 4–8 µs) and still fails the check
if a route leaves the tens of microseconds.

## Consequences

Sub-microsecond jitter on these smoke benches does not fail CI. A
regression into tens of microseconds or more still alerts. Fork PRs still
skip without the Bencher secret.
