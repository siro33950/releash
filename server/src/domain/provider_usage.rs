pub(crate) fn valid_window(used_percent: f64, window_minutes: u64, resets_at: u64) -> bool {
    used_percent.is_finite()
        && (0.0..=100.0).contains(&used_percent)
        && window_minutes > 0
        && resets_at > 0
        && resets_at <= i64::MAX as u64
}

pub(crate) fn warns(used_percent: f64) -> bool {
    used_percent > 80.0
}

#[cfg(test)]
#[path = "provider_usage_test.rs"]
mod provider_usage_tests;
