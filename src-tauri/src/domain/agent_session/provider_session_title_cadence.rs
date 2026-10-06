use std::time::Duration;

pub(crate) const PROVIDER_SESSION_TITLE_TICK_INTERVAL: Duration = Duration::from_secs(20);
pub(crate) const PROVIDER_SESSION_TITLE_REFRESH_TICKS: u64 = 15;

pub(crate) fn should_read_provider_session_title(tick: u64, has_title: bool) -> bool {
    !has_title || tick.is_multiple_of(PROVIDER_SESSION_TITLE_REFRESH_TICKS)
}

#[cfg(test)]
#[path = "provider_session_title_cadence_test.rs"]
mod provider_session_title_cadence_tests;
