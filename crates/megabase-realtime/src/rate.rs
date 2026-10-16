//! The tenant's joins-per-second `RateCounter`, which rejects joins (and
//! socket connects) once the average join rate reaches the limit.

// Ported from supabase/realtime lib/realtime/rate_counter/rate_counter.ex,
// lib/realtime/tenants.ex (`joins_per_second_rate/2`),
// lib/realtime_web/channels/tenant_rate_limiters.ex and the `limit_joins/2` step of
// lib/realtime_web/channels/realtime_channel.ex (Apache-2.0), pin v2.143.3.

use std::collections::VecDeque;

/// `TENANT_MAX_JOINS_PER_SECOND` default, the self-hosted tenant's
/// `max_joins_per_second`.
const MAX_JOINS_PER_SECOND: u64 = 100;
/// `joins_per_second_rate/2` `tick`: 5 s.
const TICK_MS: i64 = 5_000;
/// `joins_per_second_rate/2` `max_bucket_len`.
const MAX_BUCKET_LEN: usize = 12;
/// `@idle_shutdown`: 5 min.
const IDLE_SHUTDOWN_MS: i64 = 300_000;

/// A running `RateCounter` process (its cached state and `GenCounter`).
#[derive(Debug)]
struct Counter {
    /// When the next `:tick` fires (Unix ms).
    next_tick: i64,
    /// When the next `:idle_shutdown` check fires (Unix ms).
    next_idle: i64,
    /// `GenCounter` joins since the last tick.
    count: u64,
    /// Per-tick counts, newest first.
    bucket: VecDeque<u64>,
    /// `limit.triggered` as of the last tick.
    triggered: bool,
}

impl Counter {
    /// `init/1`: the first `:tick` fires at once.
    fn new(now_ms: i64) -> Self {
        Self {
            next_tick: now_ms,
            next_idle: now_ms.saturating_add(IDLE_SHUTDOWN_MS),
            count: 0,
            bucket: VecDeque::with_capacity(MAX_BUCKET_LEN + 1),
            triggered: false,
        }
    }

    /// `handle_info(:tick, …)` with `maybe_trigger_limit/1`: the counter's
    /// `avg` (`sum / bucket_len / 5`) at or above the limit triggers it, and
    /// below it releases it.
    fn tick(&mut self) {
        self.bucket.push_front(std::mem::take(&mut self.count));
        self.bucket.truncate(MAX_BUCKET_LEN);
        let sum: u64 = self.bucket.iter().sum();
        let len = u64::try_from(self.bucket.len()).unwrap_or(u64::MAX);
        let tick_s = u64::try_from(TICK_MS / 1000).unwrap_or(1);
        self.triggered = sum
            >= MAX_JOINS_PER_SECOND
                .saturating_mul(len)
                .saturating_mul(tick_s);
        self.next_tick = self.next_tick.saturating_add(TICK_MS);
    }
}

/// `{:channel, :joins, tenant}`: absent until the first connect or join, and
/// again after an idle shutdown.
#[derive(Debug, Default)]
pub(crate) struct JoinRate {
    counter: Option<Counter>,
}

impl JoinRate {
    /// `RateCounter.new/1` plus `RateCounter.get/1` at Unix time `now_ms`:
    /// runs the timers due by then, starting a counter if none runs.
    fn get(&mut self, now_ms: i64) -> &mut Counter {
        while let Some(counter) = &mut self.counter {
            if counter.next_tick.min(counter.next_idle) > now_ms {
                break;
            }
            if counter.next_tick <= counter.next_idle {
                counter.tick();
            } else if counter.bucket.iter().all(|count| *count == 0) {
                // `:idle_shutdown` with an all-zero bucket stops the process.
                self.counter = None;
            } else {
                counter.next_idle = counter.next_idle.saturating_add(IDLE_SHUTDOWN_MS);
            }
        }
        self.counter.get_or_insert_with(|| Counter::new(now_ms))
    }

    /// `TenantRateLimiters.check_tenant/1` on connect: whether the limit is
    /// triggered. A connect is not counted.
    pub(crate) fn triggered(&mut self, now_ms: i64) -> bool {
        let counter = self.get(now_ms);
        // A fresh counter's `:tick` runs before anything reads it.
        if counter.next_tick <= now_ms {
            counter.tick();
        }
        counter.triggered
    }

    /// `limit_joins/2`: `false` while the limit is triggered, else the join
    /// is counted.
    pub(crate) fn join(&mut self, now_ms: i64) -> bool {
        if self.triggered(now_ms) {
            return false;
        }
        if let Some(counter) = &mut self.counter {
            counter.count += 1;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_700_000_000_000;

    #[test]
    fn triggers_at_an_average_of_100_joins_per_second() {
        let mut rate = JoinRate::default();
        // The first tick ran at T0 with no joins: the bucket is `[0]`.
        for _ in 0..1000 {
            assert!(rate.join(T0));
        }
        // `[1000, 0]`: 1000 / 2 / 5 = 100.
        assert!(rate.triggered(T0 + TICK_MS));
        // The state holds until the next tick.
        assert!(!rate.join(T0 + 2 * TICK_MS - 1));
        // Rejected joins are not counted: `[0, 1000, 0]` is below the limit.
        assert!(rate.join(T0 + 2 * TICK_MS));
    }

    #[test]
    fn stays_below_the_limit() {
        let mut rate = JoinRate::default();
        for _ in 0..999 {
            assert!(rate.join(T0));
        }
        assert!(!rate.triggered(T0 + TICK_MS));
    }

    #[test]
    fn the_limit_holds_until_the_average_drops() {
        let mut rate = JoinRate::default();
        for _ in 0..5000 {
            rate.join(T0);
        }
        // `[5000, 0]` = 500/s, then `[0, 5000, 0]` = 333/s ... the average
        // drops below 100 at `[0 × 9, 5000, 0]`: 5000 / 11 / 5 = 90.9.
        for k in 1..=9 {
            assert!(rate.triggered(T0 + k * TICK_MS), "tick {k}");
        }
        assert!(!rate.triggered(T0 + 10 * TICK_MS));
    }

    #[test]
    fn idle_shutdown_restarts_the_counter() {
        let mut rate = JoinRate::default();
        rate.join(T0);
        // The last 12 ticks before the first idle check are all zero: the
        // counter stops. A new one has the bucket `[0]`, so 1000 joins
        // trigger it, where `[1000, 0 × 11]` would not.
        let later = T0 + IDLE_SHUTDOWN_MS + 1;
        for _ in 0..1000 {
            assert!(rate.join(later));
        }
        assert!(rate.triggered(later + TICK_MS));
    }

    #[test]
    fn a_recent_join_keeps_the_counter_running() {
        let mut rate = JoinRate::default();
        rate.join(T0);
        // A join in the last minute before the idle check keeps the bucket.
        rate.join(T0 + IDLE_SHUTDOWN_MS - 30_000);
        let later = T0 + IDLE_SHUTDOWN_MS + 1;
        for _ in 0..1000 {
            assert!(rate.join(later));
        }
        // `[1000, 0, …, 1, …]` over 12 ticks: 1001 / 12 / 5 < 100.
        assert!(!rate.triggered(T0 + IDLE_SHUTDOWN_MS + TICK_MS));
    }
}
