use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct CacheTtl(Duration);

impl CacheTtl {
    pub const fn from_secs(secs: u64) -> Self {
        Self(Duration::from_secs(secs))
    }

    pub fn is_fresh(&self, fetched_at: Instant, now: Instant) -> bool {
        now.duration_since(fetched_at) < self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_fresh_before_ttl() {
        let ttl = CacheTtl::from_secs(30);
        let fetched_at = Instant::now();
        let now = fetched_at.checked_add(Duration::from_secs(29)).unwrap();

        assert!(ttl.is_fresh(fetched_at, now));
    }

    #[test]
    fn is_stale_at_ttl_boundary() {
        let ttl = CacheTtl::from_secs(30);
        let fetched_at = Instant::now();
        let now = fetched_at.checked_add(Duration::from_secs(30)).unwrap();

        assert!(!ttl.is_fresh(fetched_at, now));
    }

    #[test]
    fn is_stale_after_ttl() {
        let ttl = CacheTtl::from_secs(30);
        let fetched_at = Instant::now();
        let now = fetched_at.checked_add(Duration::from_secs(31)).unwrap();

        assert!(!ttl.is_fresh(fetched_at, now));
    }
}
