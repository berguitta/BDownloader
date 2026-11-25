//! Global bandwidth limit shared by every download (token bucket).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

pub struct RateLimiter {
    /// Bytes per second, 0 means unlimited.
    limit: AtomicU64,
    bucket: Mutex<Bucket>,
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

impl Default for RateLimiter {
    fn default() -> Self {
        RateLimiter { limit: AtomicU64::new(0), bucket: Mutex::new(Bucket { tokens: 0.0, last: Instant::now() }) }
    }
}

impl RateLimiter {
    pub fn set_limit(&self, bytes_per_sec: u64) {
        self.limit.store(bytes_per_sec, Ordering::Relaxed);
    }

    pub fn limit(&self) -> u64 {
        self.limit.load(Ordering::Relaxed)
    }

    /// Waits until `bytes` may be consumed under the current limit.
    pub async fn acquire(&self, bytes: usize) {
        loop {
            let rate = self.limit() as f64;
            if rate <= 0.0 {
                return;
            }
            let wait = {
                let mut bucket = self.bucket.lock().await;
                let now = Instant::now();
                // At most one second of burst.
                bucket.tokens = (bucket.tokens + now.duration_since(bucket.last).as_secs_f64() * rate).min(rate);
                bucket.last = now;
                let need = bytes as f64;
                if bucket.tokens >= need || need > rate {
                    // Chunks larger than one second of budget go through and
                    // leave a debt that the next calls pay back.
                    bucket.tokens -= need;
                    None
                } else {
                    Some(Duration::from_secs_f64((need - bucket.tokens) / rate))
                }
            };
            match wait {
                None => return,
                Some(wait) => tokio::time::sleep(wait.min(Duration::from_millis(250))).await,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unlimited_never_waits() {
        let limiter = RateLimiter::default();
        let start = Instant::now();
        for _ in 0..1000 {
            limiter.acquire(1 << 20).await;
        }
        assert!(start.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn limits_throughput() {
        let limiter = RateLimiter::default();
        limiter.set_limit(200_000);
        let start = Instant::now();
        // 1 s of burst budget is empty at start, so 100 KB takes ~0.5 s.
        for _ in 0..10 {
            limiter.acquire(10_000).await;
        }
        let elapsed = start.elapsed().as_secs_f64();
        assert!((0.35..1.0).contains(&elapsed), "{elapsed}");
    }
}
