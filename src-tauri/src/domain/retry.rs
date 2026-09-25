use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct RetryBackoff {
    initial: Duration,
    multiplier: f64,
    maximum: Duration,
}

impl RetryBackoff {
    pub const ITEM: Self = Self::new(Duration::from_millis(5), 2.0, Duration::from_secs(1000));
    pub const RECOVERY: Self = Self::new(Duration::from_millis(800), 2.0, Duration::from_secs(30));
    pub const CONFLICT: Self = Self::new(Duration::from_millis(10), 5.0, Duration::from_secs(1));
    pub const SERVICE: Self = Self::new(Duration::from_secs(1), 1.6, Duration::from_secs(120));

    pub const fn new(initial: Duration, multiplier: f64, maximum: Duration) -> Self {
        assert!(!initial.is_zero());
        assert!(multiplier >= 1.0 && multiplier < f64::INFINITY);
        assert!(!maximum.is_zero());
        Self {
            initial,
            multiplier,
            maximum,
        }
    }

    pub fn delay(self, failure_count: u64, jitter: f64) -> Duration {
        assert!((0.8..=1.2).contains(&jitter));
        let exponent = failure_count.saturating_sub(1).min(i32::MAX as u64) as i32;
        Duration::from_secs_f64(
            (self.initial.as_secs_f64() * self.multiplier.powi(exponent))
                .min(self.maximum.as_secs_f64())
                * jitter,
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RetryBucket {
    tokens: f64,
    updated_at: Duration,
}

impl RetryBucket {
    pub fn new(now: Duration) -> Self {
        Self {
            tokens: 100.0,
            updated_at: now,
        }
    }

    pub fn acquire(&mut self, now: Duration) -> Duration {
        self.tokens =
            (self.tokens + now.saturating_sub(self.updated_at).as_secs_f64() * 10.0).min(100.0);
        self.updated_at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Duration::ZERO
        } else {
            Duration::from_secs_f64((1.0 - self.tokens) / 10.0)
        }
    }
}

#[cfg(test)]
#[path = "retry_test.rs"]
mod retry_tests;
