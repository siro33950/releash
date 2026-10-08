pub(crate) mod tests {
    use super::super::*;

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
