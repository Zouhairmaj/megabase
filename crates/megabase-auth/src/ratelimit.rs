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

struct Bucket {
    tokens: f64,
    last: Instant,
}

/// Token buckets per key. Each bucket starts full and refills at `rate` per second.
pub(crate) struct KeyedLimiter {
    rate: f64,
    burst: f64,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl KeyedLimiter {
    /// `rate` is tokens per second.
    pub(crate) fn new(rate: f64, burst: f64) -> Self {
        Self {
            rate: rate.max(0.0),
            burst,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Take one token for `key`; `false` when the bucket is empty.
    pub(crate) fn allow(&self, key: &str) -> bool {
        self.allow_at(key, Instant::now())
    }

    fn allow_at(&self, key: &str, now: Instant) -> bool {
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        if buckets.len() >= SWEEP_AT {
            buckets.retain(|_, bucket| now.saturating_duration_since(bucket.last) < ENTRY_TTL);
        }
        let bucket = buckets.entry(key.to_string()).or_insert(Bucket {
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
        let size = limiter
            .buckets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        assert_eq!(size, 1);
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
