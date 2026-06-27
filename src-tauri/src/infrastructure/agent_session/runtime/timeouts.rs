use std::time::Duration;

pub(crate) const DEFAULT_STARTUP_TIMEOUT_SECS: u64 = 30;
pub(crate) const DEFAULT_STALE_TIMEOUT_SECS: u64 = 180;
/// Defensive ceiling for corrupted local session meta. Policy-generated values
/// are expected to be well below this limit.
pub(crate) const MAX_STARTUP_TIMEOUT_SECS: u64 = 300;
/// Defensive ceiling for corrupted local session meta. Policy-generated values
/// are expected to be well below this limit.
pub(crate) const MAX_STALE_TIMEOUT_SECS: u64 = 1800;
/// Defensive ceiling for corrupted local session meta. Policy-generated values
/// are expected to be well below this limit.
pub(crate) const MAX_STARTUP_RETRIES: u32 = 10;

pub(crate) fn default_startup_timeout() -> Duration {
    Duration::from_secs(DEFAULT_STARTUP_TIMEOUT_SECS)
}

pub(crate) fn default_stale_timeout() -> Duration {
    Duration::from_secs(DEFAULT_STALE_TIMEOUT_SECS)
}
