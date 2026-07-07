//! Stale detection and stall signaling are related but distinct concerns.
//!
//! A turn is stale when the watchdog observes that the runtime has produced no
//! relevant progress past the configured threshold. A stall is the non-terminal
//! workflow/agent signal emitted when that stale threshold is reached. In other
//! words, stale detection causes stall observation, but stale is the timeout
//! condition/watchdog boundary while stall is the active intervention signal.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use super::session_state::RuntimeSessionPhase;
use crate::domain::agent_session::entities::MessagePart;
use crate::usecase::agent_session::session::ChatSession;

const DEFAULT_STALE_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_STALE_TIMEOUT: Duration = Duration::from_secs(1_800);

pub(crate) const MAX_STALL_SIGNALS: u32 = 3;
pub(crate) const MAX_STALL_RECOVERY_ATTEMPTS: u32 = 3;

pub(crate) fn stale_timeout_for_session(session: &ChatSession) -> Duration {
    timeout_from_secs(
        session
            .workflow_step_context
            .as_ref()
            .and_then(|context| context.stale_timeout_secs),
    )
}

pub(crate) fn timeout_from_secs(value: Option<u64>) -> Duration {
    value
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_STALE_TIMEOUT)
        .min(MAX_STALE_TIMEOUT)
}

/// ToolResult が未到着の ToolUse が残っている（= backend 側でツール実行中）か。
pub(crate) fn has_in_flight_tool_use(parts: &[MessagePart]) -> bool {
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

/// ツール実行中は backend が無出力でも正常（cargo test 等の長時間コマンド）のため、
/// stale timeout を上限値まで延長する。
pub(crate) fn effective_stale_timeout(base: Duration, tool_in_flight: bool) -> Duration {
    if tool_in_flight {
        base.max(MAX_STALE_TIMEOUT)
    } else {
        base
    }
}

pub(crate) fn turn_is_stale(
    phase: RuntimeSessionPhase,
    expected_generation: u64,
    actual_generation: u64,
    last_progress_at: Option<Instant>,
    timeout: Duration,
    now: Instant,
) -> bool {
    phase == RuntimeSessionPhase::Streaming
        && expected_generation == actual_generation
        && last_progress_at
            .map(|last_progress_at| now.duration_since(last_progress_at) >= timeout)
            .unwrap_or(false)
}

pub(crate) fn stale_watchdog_should_continue_waiting(
    phase: RuntimeSessionPhase,
    expected_generation: u64,
    actual_generation: u64,
) -> bool {
    expected_generation == actual_generation
        && matches!(
            phase,
            RuntimeSessionPhase::Streaming | RuntimeSessionPhase::WaitingPermission
        )
}

pub(crate) fn remaining_until_stale(
    last_progress_at: Option<Instant>,
    timeout: Duration,
    now: Instant,
) -> Option<Duration> {
    let elapsed = now.duration_since(last_progress_at?);
    Some(timeout.saturating_sub(elapsed))
}

pub(crate) fn stall_cap_reached(signal_count: u32) -> bool {
    signal_count >= MAX_STALL_SIGNALS
}

pub(crate) fn recovery_cap_reached(recovery_attempts: u32) -> bool {
    recovery_attempts >= MAX_STALL_RECOVERY_ATTEMPTS
}

pub(crate) fn startup_timeout_for_session(session: &ChatSession) -> Option<Duration> {
    session
        .workflow_step_context
        .as_ref()
        .and_then(|context| context.startup_timeout_secs)
        .map(Duration::from_secs)
}

pub(crate) fn startup_max_retries_for_session(session: &ChatSession) -> Option<u32> {
    session
        .workflow_step_context
        .as_ref()
        .and_then(|context| context.startup_max_retries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stale_timeout_未指定は既定値を返す() {
        // Given: workflow context does not define a stale timeout.
        // When: resolving the runtime stale timeout.
        let timeout = timeout_from_secs(None);

        // Then: the design default is used.
        assert_eq!(timeout, Duration::from_secs(180));
    }

    #[test]
    fn test_stale_timeout_上限を超える値は一八〇〇秒へ丸める() {
        // Given: a workflow context requests a timeout above the cap.
        // When: resolving the runtime stale timeout.
        let timeout = timeout_from_secs(Some(9_999));

        // Then: the design cap is applied.
        assert_eq!(timeout, Duration::from_secs(1_800));
    }

    #[test]
    fn test_turn_is_stale_streamingかつ同一世代で超過した場合のみtrue() {
        // Given: a streaming turn whose last progress is older than the timeout.
        let last_progress_at = Instant::now() - Duration::from_secs(10);

        // When / Then: only the matching generation streaming turn is stale.
        assert!(turn_is_stale(
            RuntimeSessionPhase::Streaming,
            7,
            7,
            Some(last_progress_at),
            Duration::from_secs(5),
            Instant::now(),
        ));
        assert!(!turn_is_stale(
            RuntimeSessionPhase::Idle,
            7,
            7,
            Some(last_progress_at),
            Duration::from_secs(5),
            Instant::now(),
        ));
        assert!(!turn_is_stale(
            RuntimeSessionPhase::Streaming,
            7,
            8,
            Some(last_progress_at),
            Duration::from_secs(5),
            Instant::now(),
        ));
    }

    #[test]
    fn test_has_in_flight_tool_use_toolresult未到着のtooluseがあればtrue() {
        // Given: a tool use whose result has not arrived yet.
        let running = vec![MessagePart::ToolUse {
            id: "tool-1".to_string(),
            tool: "Bash".to_string(),
            input: crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                "{}".to_string(),
            ),
            parent_tool_use_id: None,
        }];

        // Then: the turn counts as tool-in-flight.
        assert!(has_in_flight_tool_use(&running));

        // Given: the matching tool result has arrived.
        let mut finished = running.clone();
        finished.push(MessagePart::ToolResult {
            content: "ok".to_string(),
            is_error: false,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
            content_ref: None,
            summary: None,
        });

        // Then: the turn is no longer tool-in-flight.
        assert!(!has_in_flight_tool_use(&finished));

        // Given: text-only parts.
        let text_only = vec![MessagePart::Text {
            content: "hello".to_string(),
            parent_tool_use_id: None,
        }];

        // Then: no tool is in flight.
        assert!(!has_in_flight_tool_use(&text_only));
    }

    #[test]
    fn test_effective_stale_timeout_ツール実行中は上限まで延長する() {
        // Given: the default timeout with a tool in flight.
        // Then: the timeout extends to the cap.
        assert_eq!(
            effective_stale_timeout(Duration::from_secs(180), true),
            Duration::from_secs(1_800)
        );

        // Given: no tool in flight.
        // Then: the base timeout is kept.
        assert_eq!(
            effective_stale_timeout(Duration::from_secs(180), false),
            Duration::from_secs(180)
        );

        // Given: a configured timeout above the cap with a tool in flight.
        // Then: the larger value wins (no shrink).
        assert_eq!(
            effective_stale_timeout(Duration::from_secs(2_000), true),
            Duration::from_secs(2_000)
        );
    }

    #[test]
    fn test_stale_watchdog_should_continue_waiting_permission() {
        assert!(stale_watchdog_should_continue_waiting(
            RuntimeSessionPhase::WaitingPermission,
            1,
            1,
        ));
        assert!(!stale_watchdog_should_continue_waiting(
            RuntimeSessionPhase::Idle,
            1,
            1,
        ));
        assert!(!stale_watchdog_should_continue_waiting(
            RuntimeSessionPhase::WaitingPermission,
            1,
            2,
        ));
    }

    #[test]
    fn test_stall_cap_reached_上限到達以上でtrue() {
        assert!(!stall_cap_reached(MAX_STALL_SIGNALS - 1));
        assert!(stall_cap_reached(MAX_STALL_SIGNALS));
        assert!(stall_cap_reached(MAX_STALL_SIGNALS + 1));
    }

    #[test]
    fn test_recovery_cap_reached_上限到達以上でtrue() {
        assert!(!recovery_cap_reached(MAX_STALL_RECOVERY_ATTEMPTS - 1));
        assert!(recovery_cap_reached(MAX_STALL_RECOVERY_ATTEMPTS));
        assert!(recovery_cap_reached(MAX_STALL_RECOVERY_ATTEMPTS + 1));
    }
}
