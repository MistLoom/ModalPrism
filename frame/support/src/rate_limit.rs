//! 通用限流原语：框架层可复用的令牌桶，供端口 / 网关管道在调用准入时使用。
//!
//! 不做商业 APM、不做分布式配额——只在单进程内提供确定性的准入判定。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// 限流准入判定。`key` 通常是实例 id / 端口名 / 调用方标识。
pub trait RateLimiter: Send + Sync {
    /// 尝试获取一个许可；`true` = 放行，`false` = 已超限。
    fn try_acquire(&self, key: &str) -> bool;
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

/// 令牌桶：`capacity` 容量（允许的突发），`refill_per_sec` 每秒稳定补充速率。
pub struct TokenBucket {
    capacity: f64,
    refill_per_sec: f64,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl TokenBucket {
    pub fn new(capacity: f64, refill_per_sec: f64) -> Self {
        assert!(capacity > 0.0 && refill_per_sec > 0.0, "容量与速率须为正");
        Self {
            capacity,
            refill_per_sec,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// 每个 key 独立桶；key 传空串即为全局单桶。
    fn take(&self, key: &str) -> bool {
        let mut map = self.buckets.lock().unwrap();
        let now = Instant::now();
        let bucket = map.entry(key.to_string()).or_insert(Bucket {
            tokens: self.capacity,
            last: now,
        });
        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

impl RateLimiter for TokenBucket {
    fn try_acquire(&self, key: &str) -> bool {
        self.take(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_then_throttle() {
        let limiter = TokenBucket::new(3.0, 0.0 + f64::EPSILON);
        assert!(limiter.try_acquire("k"));
        assert!(limiter.try_acquire("k"));
        assert!(limiter.try_acquire("k"));
        assert!(!limiter.try_acquire("k"), "超出容量应拒绝");
    }

    #[test]
    fn keys_are_isolated() {
        let limiter = TokenBucket::new(1.0, f64::EPSILON);
        assert!(limiter.try_acquire("a"));
        assert!(!limiter.try_acquire("a"));
        assert!(limiter.try_acquire("b"), "不同 key 独立计桶");
    }
}
