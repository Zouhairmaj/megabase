// Ported from supabase/auth internal/api/middleware.go and
// internal/api/apilimiter/apilimiter.go (MIT), pin v2.197.0.

//! Header-keyed token-bucket rate limiting, as GoTrue's `Token` limiter.
//!
//! GoTrue builds `tollbooth.NewLimiter(rate, ...).SetBurst(30)` with a one
//! hour entry TTL. It limits only when `GOTRUE_RATE_LIMIT_HEADER` names a
//! header whose first comma-separated value is non-blank. The `Sb-Forwarded-For`
//! address path is not served: the gateway does not set that header.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::http::{HeaderMap, StatusCode};

use crate::error::GoTrueError;

/// Hardcoded burst of the token limiter.
pub(crate) const TOKEN_BURST: f64 = 30.0;
/// `DefaultExpirationTTL` of the token limiter.
const ENTRY_TTL: Duration = Duration::from_secs(60 * 60);
/// Sweep expired entries once the table grows past this size.
const SWEEP_AT: usize = 1024;
/// Sweep at most once per interval, so live keys do not force a scan per request.
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

struct Bucket {
    tokens: f64,
    last: Instant,
}

/// Token buckets per key. Each bucket starts full and refills at `rate` per second.
pub(crate) struct KeyedLimiter {
    rate: f64,
    burst: f64,
    table: Mutex<Table>,
}

#[derive(Default)]
struct Table {
    buckets: HashMap<String, Bucket>,
    last_sweep: Option<Instant>,
}

impl KeyedLimiter {
    /// `rate` is tokens per second.
    pub(crate) fn new(rate: f64, burst: f64) -> Self {
        Self {
            rate: rate.max(0.0),
            burst,
            table: Mutex::new(Table::default()),
        }
    }

    /// Take one token for `key`; `false` when the bucket is empty.
    pub(crate) fn allow(&self, key: &str) -> bool {
        self.allow_at(key, Instant::now())
    }

    fn allow_at(&self, key: &str, now: Instant) -> bool {
        let mut guard = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        let table = &mut *guard;
        let sweep_due = table
            .last_sweep
            .is_none_or(|at| now.saturating_duration_since(at) >= SWEEP_INTERVAL);
        if table.buckets.len() >= SWEEP_AT && sweep_due {
            table
                .buckets
                .retain(|_, bucket| now.saturating_duration_since(bucket.last) < ENTRY_TTL);
            table.last_sweep = Some(now);
        }
        let bucket = table.buckets.entry(key.to_string()).or_insert(Bucket {
            tokens: self.burst,
            last: now,
        });
        let elapsed = now.saturating_duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.rate).min(self.burst);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// `performRateLimitingWithHeader`: `Err` is GoTrue's 429.
///
/// # Errors
/// `over_request_rate_limit` when the request's key has no token left.
pub(crate) fn limit_by_header(
    limiter: &KeyedLimiter,
    header: &str,
    headers: &HeaderMap,
) -> Result<(), GoTrueError> {
    if header.is_empty() {
        return Ok(());
    }
    let Some(value) = headers.get(header) else {
        return Ok(());
    };
    let raw = String::from_utf8_lossy(value.as_bytes());
    let key = raw.split(',').next().unwrap_or_default().trim();
    if key.is_empty() || limiter.allow(key) {
        return Ok(());
    }
    Err(GoTrueError::new(
        StatusCode::TOO_MANY_REQUESTS,
        "over_request_rate_limit",
        "Request rate limit reached",
    ))
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn burst_is_spent_then_refills() {
        let limiter = KeyedLimiter::new(0.5, TOKEN_BURST);
        let start = Instant::now();
        for _ in 0..30 {
            assert!(limiter.allow_at("a", start));
        }
        assert!(!limiter.allow_at("a", start));
        assert!(limiter.allow_at("b", start), "keys are independent");
        assert!(!limiter.allow_at("a", start + Duration::from_secs(1)));
        assert!(limiter.allow_at("a", start + Duration::from_secs(2)));
    }

    #[test]
    fn expired_entries_are_swept() {
        let limiter = KeyedLimiter::new(0.0, 1.0);
        let start = Instant::now();
        for n in 0..SWEEP_AT {
            assert!(limiter.allow_at(&n.to_string(), start));
        }
        let later = start + ENTRY_TTL + Duration::from_secs(1);
        assert!(limiter.allow_at("new", later));
        assert_eq!(table_len(&limiter), 1);
    }

    fn table_len(limiter: &KeyedLimiter) -> usize {
        limiter
            .table
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .buckets
            .len()
    }

    #[test]
    fn sweep_runs_at_most_once_per_interval() {
        let limiter = KeyedLimiter::new(0.0, 1.0);
        let start = Instant::now();
        for n in 0..SWEEP_AT {
            assert!(limiter.allow_at(&n.to_string(), start));
        }
        // First sweep finds nothing expired and records its time.
        let first = start + ENTRY_TTL - Duration::from_secs(30);
        assert!(limiter.allow_at("x", first));
        assert_eq!(table_len(&limiter), SWEEP_AT + 1);
        // Entries are now expired, but the last sweep was 31 s ago: no scan.
        let inside = start + ENTRY_TTL + Duration::from_secs(1);
        assert!(limiter.allow_at("y", inside));
        assert_eq!(table_len(&limiter), SWEEP_AT + 2);
        // Past the interval the sweep runs again.
        let after = first + SWEEP_INTERVAL;
        assert!(limiter.allow_at("z", after));
        assert_eq!(table_len(&limiter), 3);
    }

    #[test]
    fn header_key_is_first_value_and_optional() {
        let limiter = KeyedLimiter::new(0.0, 1.0);
        let mut headers = HeaderMap::new();
        assert!(limit_by_header(&limiter, "x-key", &headers).is_ok());
        assert!(limit_by_header(&limiter, "", &headers).is_ok());

        headers.insert("x-key", HeaderValue::from_static(" k1 , other"));
        assert!(limit_by_header(&limiter, "x-key", &headers).is_ok());
        headers.insert("x-key", HeaderValue::from_static("k1,different"));
        let Err(error) = limit_by_header(&limiter, "x-key", &headers) else {
            panic!("second request on k1 must be limited");
        };
        assert_eq!(error.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(error.error_code, "over_request_rate_limit");
        assert_eq!(error.message, "Request rate limit reached");

        headers.insert("x-key", HeaderValue::from_static(" ,k2"));
        assert!(limit_by_header(&limiter, "x-key", &headers).is_ok());
        assert!(limit_by_header(&limiter, "x-key", &headers).is_ok());
    }
}
