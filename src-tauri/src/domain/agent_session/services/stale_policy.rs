use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::domain::agent_session::entities::MessagePart;
use crate::domain::agent_session::value_objects::TurnPhase;

const DEFAULT_STALE_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_STALE_TIMEOUT: Duration = Duration::from_secs(1_800);
const DEFAULT_PROVIDER_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_PROVIDER_STARTUP_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_PROVIDER_STARTUP_RETRIES: u32 = 10;

pub const MAX_STALL_SIGNALS: u32 = 3;
pub const MAX_STALL_RECOVERY_ATTEMPTS: u32 = 3;

pub fn stale_timeout_from_secs(value: Option<u64>) -> Duration {
    value
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_STALE_TIMEOUT)
        .min(MAX_STALE_TIMEOUT)
}

pub fn provider_startup_timeout(value: Option<Duration>) -> Duration {
    value
        .unwrap_or(DEFAULT_PROVIDER_STARTUP_TIMEOUT)
        .min(MAX_PROVIDER_STARTUP_TIMEOUT)
}

pub fn provider_startup_retries(value: Option<u32>) -> u32 {
    value.unwrap_or(0).min(MAX_PROVIDER_STARTUP_RETRIES)
}

pub fn has_in_flight_tool_use(parts: &[MessagePart]) -> bool {
    let resolved: HashSet<&str> = parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::ToolResult {
                tool_use_id: Some(id),
                ..
            } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    parts.iter().any(
        |part| matches!(part, MessagePart::ToolUse { id, .. } if !resolved.contains(id.as_str())),
    )
}

pub fn effective_stale_timeout(base: Duration, tool_in_flight: bool) -> Duration {
    if tool_in_flight {
        base.max(MAX_STALE_TIMEOUT)
    } else {
        base
    }
}

pub fn turn_is_stale(
    phase: TurnPhase,
    expected_generation: u64,
    actual_generation: u64,
    last_progress_at: Option<Instant>,
    timeout: Duration,
    now: Instant,
) -> bool {
    phase.is_streaming()
        && expected_generation == actual_generation
        && last_progress_at
            .map(|last_progress_at| now.duration_since(last_progress_at) >= timeout)
            .unwrap_or(false)
}

pub fn stale_watchdog_should_continue_waiting(
    phase: TurnPhase,
    expected_generation: u64,
    actual_generation: u64,
) -> bool {
    expected_generation == actual_generation && phase.is_watchdog_live()
}

pub fn remaining_until_stale(
    last_progress_at: Option<Instant>,
    timeout: Duration,
    now: Instant,
) -> Option<Duration> {
    let elapsed = now.duration_since(last_progress_at?);
    Some(timeout.saturating_sub(elapsed))
}

pub fn stall_cap_reached(signal_count: u32) -> bool {
    signal_count >= MAX_STALL_SIGNALS
}

pub fn recovery_cap_reached(recovery_attempts: u32) -> bool {
    recovery_attempts >= MAX_STALL_RECOVERY_ATTEMPTS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_permission_stays_live_without_becoming_stale() {
        let now = Instant::now();
        assert!(stale_watchdog_should_continue_waiting(
            TurnPhase::WaitingPermission,
            3,
            3
        ));
        assert!(!turn_is_stale(
            TurnPhase::WaitingPermission,
            3,
            3,
            Some(now - Duration::from_secs(600)),
            Duration::from_secs(1),
            now,
        ));
    }

    #[test]
    fn configured_timeout_is_bounded() {
        assert_eq!(
            stale_timeout_from_secs(Some(10_000)),
            Duration::from_secs(1_800)
        );
        assert_eq!(stale_timeout_from_secs(None), Duration::from_secs(180));
    }

    #[test]
    fn provider_startup_policy_applies_defaults_and_bounds() {
        assert_eq!(provider_startup_timeout(None), Duration::from_secs(30));
        assert_eq!(
            provider_startup_timeout(Some(Duration::from_secs(999))),
            Duration::from_secs(300)
        );
        assert_eq!(provider_startup_retries(None), 0);
        assert_eq!(provider_startup_retries(Some(99)), 10);
    }
}
