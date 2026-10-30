//! Byte-rate limiting for transfers.
//!
//! A token bucket, not GCRA: `governor`-style algorithms rate-limit requests,
//! while downloads need a steady byte budget shared by several workers.

use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

/// Shared byte-rate limiter. `None` means unlimited.
pub struct RateLimiter {
    rate_bps: f64,
    state: Mutex<Bucket>,
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

impl Bucket {
    fn new(now: Instant) -> Self {
        Self {
            tokens: 0.0,
            last: now,
        }
    }

    /// Refill for the elapsed time, capped at `burst` (or the request size).
    fn refill(&mut self, now: Instant, rate_bps: f64, cap: f64) {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * rate_bps).min(cap);
    }

    /// Take `bytes` if available, otherwise return how long to wait.
    fn try_take(&mut self, bytes: f64, rate_bps: f64) -> Option<Duration> {
        if self.tokens >= bytes {
            self.tokens -= bytes;
            return None;
        }
        let missing = bytes - self.tokens;
        Some(Duration::from_secs_f64((missing / rate_bps).min(1.0)))
    }
}

impl RateLimiter {
    /// `rate_bps == 0` means unlimited and returns `None`.
    #[must_use]
    pub fn new(rate_bps: u64) -> Option<Self> {
        (rate_bps > 0).then(|| Self {
            rate_bps: rate_bps as f64,
            state: Mutex::new(Bucket::new(Instant::now())),
        })
    }

    #[must_use]
    pub fn rate_bps(&self) -> u64 {
        self.rate_bps as u64
    }

    /// Wait until `bytes` may be transferred.
    pub async fn acquire(&self, bytes: u64) {
        let bytes = bytes as f64;
        if bytes <= 0.0 {
            return;
        }
        // A quarter second of traffic, but never less than one request's
        // worth: otherwise a single large chunk could starve forever.
        let burst = (self.rate_bps * 0.25).max(1.0);

        loop {
            let wait = {
                let mut bucket = self.state.lock().await;
                let cap = burst.max(bytes);
                bucket.refill(Instant::now(), self.rate_bps, cap);
                match bucket.try_take(bytes, self.rate_bps) {
                    None => return,
                    Some(wait) => wait,
                }
            };
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refills_at_the_configured_rate() {
        let now = Instant::now();
        let mut bucket = Bucket::new(now);
        bucket.refill(now + Duration::from_secs(1), 1000.0, 250.0);
        assert_eq!(bucket.tokens, 250.0);
    }

    #[test]
    fn spending_reduces_tokens() {
        let now = Instant::now();
        let mut bucket = Bucket::new(now);
        bucket.tokens = 100.0;
        assert!(bucket.try_take(40.0, 1000.0).is_none());
        assert_eq!(bucket.tokens, 60.0);
    }

    #[test]
    fn insufficient_tokens_wait_proportionally() {
        let now = Instant::now();
        let mut bucket = Bucket::new(now);
        bucket.tokens = 0.0;
        let wait = bucket.try_take(500.0, 1000.0).expect("must wait");
        assert!((wait.as_secs_f64() - 0.5).abs() < 1e-6);
    }

    #[tokio::test]
    async fn zero_rate_is_unlimited() {
        assert!(RateLimiter::new(0).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn acquire_waits_for_the_budget() {
        let limiter = RateLimiter::new(10_000).expect("limited");
        let start = Instant::now();
        limiter.acquire(5_000).await;
        limiter.acquire(5_000).await;
        let elapsed = start.elapsed();
        // The first chunk may borrow the initial burst; the second must wait.
        assert!(elapsed >= Duration::from_millis(200), "elapsed {elapsed:?}");
    }
}
