//! A token bucket per bearer token (and one for registrations), in memory:
//! a burst of a minute's allowance, refilled evenly.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Beyond this many buckets, full ones are dropped (they hold nothing).
const PRUNE_AT: usize = 10_000;

pub struct RateLimiter {
    per_minute: u32,
    buckets: Mutex<HashMap<String, Bucket>>,
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    /// `per_minute` 0 turns limiting off.
    pub fn new(per_minute: u32) -> Self {
        Self { per_minute, buckets: Mutex::new(HashMap::new()) }
    }

    /// Take one request for `key`, or say how long until one is allowed.
    pub fn take(&self, key: &str) -> Result<(), Duration> {
        if self.per_minute == 0 {
            return Ok(());
        }
        let capacity = f64::from(self.per_minute);
        let per_second = capacity / 60.0;
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if buckets.len() > PRUNE_AT {
            buckets.retain(|_, b| b.tokens + now.duration_since(b.last).as_secs_f64() * per_second < capacity);
        }
        let b = buckets.entry(key.to_owned()).or_insert(Bucket { tokens: capacity, last: now });
        b.tokens = (b.tokens + now.duration_since(b.last).as_secs_f64() * per_second).min(capacity);
        b.last = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - b.tokens) / per_second))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_of_a_minutes_allowance_then_waits() {
        let l = RateLimiter::new(3);
        assert!((0..3).all(|_| l.take("a").is_ok()));
        let wait = l.take("a").unwrap_err();
        assert!(wait > Duration::from_secs(15) && wait <= Duration::from_secs(20), "{wait:?}");
        assert!(l.take("b").is_ok(), "each key has its own bucket");
        assert!(RateLimiter::new(0).take("a").is_ok());
    }
}
