use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct CacheTtl(Duration);

impl CacheTtl {
    pub const EXTERNAL_INFORMATION: Self = Self::from_secs(30);

    pub fn duration(self) -> Duration {
        self.0
    }

    pub const fn from_secs(secs: u64) -> Self {
        Self(Duration::from_secs(secs))
    }

    pub fn is_fresh(&self, fetched_at: Instant, now: Instant) -> bool {
        now.duration_since(fetched_at) < self.0
    }
}

#[cfg(test)]
#[path = "cache_test.rs"]
mod cache_tests;
