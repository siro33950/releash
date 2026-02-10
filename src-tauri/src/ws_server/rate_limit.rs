use std::collections::HashMap;
use std::time::{Duration, Instant};

pub(super) const RATE_LIMIT_MAX_FAILURES: u32 = 3;
pub(super) const RATE_LIMIT_BLOCK_SECS: u64 = 30;

pub(super) struct RateLimitEntry {
    pub(super) failures: u32,
    pub(super) blocked_until: Option<Instant>,
}

pub(super) fn is_ip_blocked(
    rate_limits: &HashMap<std::net::IpAddr, RateLimitEntry>,
    ip: &std::net::IpAddr,
) -> bool {
    if let Some(entry) = rate_limits.get(ip) {
        if let Some(blocked_until) = entry.blocked_until {
            if Instant::now() < blocked_until {
                return true;
            }
        }
    }
    false
}

pub(super) fn record_auth_failure(
    rate_limits: &mut HashMap<std::net::IpAddr, RateLimitEntry>,
    ip: std::net::IpAddr,
) {
    let entry = rate_limits.entry(ip).or_insert(RateLimitEntry {
        failures: 0,
        blocked_until: None,
    });
    if let Some(blocked_until) = entry.blocked_until {
        if Instant::now() >= blocked_until {
            entry.failures = 0;
            entry.blocked_until = None;
        }
    }
    entry.failures += 1;
    if entry.failures >= RATE_LIMIT_MAX_FAILURES {
        entry.blocked_until = Some(Instant::now() + Duration::from_secs(RATE_LIMIT_BLOCK_SECS));
    }
}

pub(super) fn clear_auth_failures(
    rate_limits: &mut HashMap<std::net::IpAddr, RateLimitEntry>,
    ip: &std::net::IpAddr,
) {
    rate_limits.remove(ip);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limit_not_blocked_initially() {
        let limits = HashMap::new();
        let ip: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        assert!(!is_ip_blocked(&limits, &ip));
    }

    #[test]
    fn test_rate_limit_blocked_after_max_failures() {
        let mut limits = HashMap::new();
        let ip: std::net::IpAddr = "192.168.1.1".parse().unwrap();

        for _ in 0..RATE_LIMIT_MAX_FAILURES {
            record_auth_failure(&mut limits, ip);
        }

        assert!(is_ip_blocked(&limits, &ip));
    }

    #[test]
    fn test_rate_limit_not_blocked_before_max() {
        let mut limits = HashMap::new();
        let ip: std::net::IpAddr = "192.168.1.1".parse().unwrap();

        for _ in 0..(RATE_LIMIT_MAX_FAILURES - 1) {
            record_auth_failure(&mut limits, ip);
        }

        assert!(!is_ip_blocked(&limits, &ip));
    }

    #[test]
    fn test_clear_auth_failures() {
        let mut limits = HashMap::new();
        let ip: std::net::IpAddr = "10.0.0.1".parse().unwrap();

        for _ in 0..RATE_LIMIT_MAX_FAILURES {
            record_auth_failure(&mut limits, ip);
        }
        assert!(is_ip_blocked(&limits, &ip));

        clear_auth_failures(&mut limits, &ip);
        assert!(!is_ip_blocked(&limits, &ip));
    }

    #[test]
    fn test_rate_limit_block_recovery_after_timeout() {
        let mut limits = HashMap::new();
        let ip: std::net::IpAddr = "10.0.0.2".parse().unwrap();

        for _ in 0..RATE_LIMIT_MAX_FAILURES {
            record_auth_failure(&mut limits, ip);
        }
        assert!(is_ip_blocked(&limits, &ip));

        if let Some(entry) = limits.get_mut(&ip) {
            entry.blocked_until = Some(Instant::now() - Duration::from_secs(1));
        }
        assert!(!is_ip_blocked(&limits, &ip));
    }

    #[test]
    fn test_rate_limit_independent_ips() {
        let mut limits = HashMap::new();
        let ip1: std::net::IpAddr = "192.168.1.10".parse().unwrap();
        let ip2: std::net::IpAddr = "192.168.1.20".parse().unwrap();

        for _ in 0..RATE_LIMIT_MAX_FAILURES {
            record_auth_failure(&mut limits, ip1);
        }

        assert!(is_ip_blocked(&limits, &ip1));
        assert!(!is_ip_blocked(&limits, &ip2));
    }

    #[test]
    fn test_rate_limit_reset_counter_after_block_expires() {
        let mut limits = HashMap::new();
        let ip: std::net::IpAddr = "10.0.0.3".parse().unwrap();

        for _ in 0..RATE_LIMIT_MAX_FAILURES {
            record_auth_failure(&mut limits, ip);
        }
        assert!(is_ip_blocked(&limits, &ip));

        if let Some(entry) = limits.get_mut(&ip) {
            entry.blocked_until = Some(Instant::now() - Duration::from_secs(1));
        }

        record_auth_failure(&mut limits, ip);
        let entry = limits.get(&ip).unwrap();
        assert_eq!(entry.failures, 1);
        assert!(!is_ip_blocked(&limits, &ip));
    }
}
