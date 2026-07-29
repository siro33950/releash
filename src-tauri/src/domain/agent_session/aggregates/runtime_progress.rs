use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistenceRetryObservation {
    First { attempt: u64 },
    Repeated { attempt: u64 },
}

#[derive(Debug, Default)]
pub struct PersistenceRetry {
    failed_attempts: u64,
}

impl PersistenceRetry {
    pub fn observe_failure(&mut self) -> PersistenceRetryObservation {
        self.failed_attempts = self.failed_attempts.saturating_add(1);
        if self.failed_attempts == 1 {
            PersistenceRetryObservation::First { attempt: 1 }
        } else {
            PersistenceRetryObservation::Repeated {
                attempt: self.failed_attempts,
            }
        }
    }
}

use crate::domain::agent_session::services::{recovery_cap_reached, stall_cap_reached};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeStallSignal {
    pub idle_secs: u64,
    pub signal_count: u32,
    pub cap_reached: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStallDecision {
    Exhausted,
    Observe {
        signal: Option<RuntimeStallSignal>,
        request_recovery: bool,
        should_rearm: bool,
    },
}

/// Process-local progress and stall-observation aggregate for the current
/// runtime turn.
#[derive(Debug, Default)]
pub struct RuntimeProgress {
    last_progress_at: Option<Instant>,
    turn_started_at: Option<Instant>,
    first_backend_event_recorded: bool,
    stall_signal_count: u32,
    stall_recovery_attempts: u32,
    stall_observation_active: bool,
}

impl RuntimeProgress {
    pub fn start_turn(&mut self, at: Instant) {
        self.last_progress_at = Some(at);
        self.turn_started_at = Some(at);
        self.first_backend_event_recorded = false;
        self.stall_signal_count = 0;
        self.stall_recovery_attempts = 0;
        self.stall_observation_active = false;
    }

    pub fn clear_turn(&mut self) {
        self.last_progress_at = None;
        self.turn_started_at = None;
        self.first_backend_event_recorded = false;
        self.stall_signal_count = 0;
        self.stall_recovery_attempts = 0;
        self.stall_observation_active = false;
    }

    pub fn finish_turn(&mut self) {
        self.turn_started_at = None;
        self.stall_observation_active = false;
    }

    pub fn last_progress_at(&self) -> Option<Instant> {
        self.last_progress_at
    }

    pub fn turn_started_at(&self) -> Option<Instant> {
        self.turn_started_at
    }

    pub fn mark_progress(&mut self, at: Instant) -> bool {
        self.last_progress_at = Some(at);
        let had_active_observation = self.stall_observation_active;
        self.stall_observation_active = false;
        had_active_observation
    }

    pub fn record_progress(&mut self, at: Instant) -> bool {
        self.last_progress_at = Some(at);
        self.stall_observation_active
    }

    pub fn clear_stall_observation(&mut self) {
        self.stall_observation_active = false;
    }

    #[cfg(test)]
    pub fn stall_observation_is_active(&self) -> bool {
        self.stall_observation_active
    }

    pub fn record_first_backend_event(
        &mut self,
        has_active_turn: bool,
        now: Instant,
    ) -> Option<Duration> {
        if !has_active_turn || self.first_backend_event_recorded {
            return None;
        }
        let started_at = self.turn_started_at?;
        self.first_backend_event_recorded = true;
        Some(now.duration_since(started_at))
    }

    pub fn observe_stall(&mut self, has_runtime: bool, now: Instant) -> RuntimeStallDecision {
        self.stall_observation_active = true;
        if stall_cap_reached(self.stall_signal_count)
            && (recovery_cap_reached(self.stall_recovery_attempts) || !has_runtime)
        {
            return RuntimeStallDecision::Exhausted;
        }

        let signal = if stall_cap_reached(self.stall_signal_count) {
            None
        } else {
            self.stall_signal_count = self.stall_signal_count.saturating_add(1);
            Some(RuntimeStallSignal {
                idle_secs: self
                    .last_progress_at
                    .map(|last_progress_at| now.duration_since(last_progress_at).as_secs())
                    .unwrap_or(0),
                signal_count: self.stall_signal_count,
                cap_reached: stall_cap_reached(self.stall_signal_count),
            })
        };
        let request_recovery = has_runtime && !recovery_cap_reached(self.stall_recovery_attempts);
        if request_recovery {
            self.stall_recovery_attempts = self.stall_recovery_attempts.saturating_add(1);
        }
        let should_rearm = !stall_cap_reached(self.stall_signal_count)
            || (has_runtime && !recovery_cap_reached(self.stall_recovery_attempts));
        RuntimeStallDecision::Observe {
            signal,
            request_recovery,
            should_rearm,
        }
    }

    #[cfg(test)]
    pub fn restore_for_test(
        &mut self,
        last_progress_at: Option<Instant>,
        stall_signal_count: u32,
        stall_recovery_attempts: u32,
        stall_observation_active: bool,
    ) {
        self.last_progress_at = last_progress_at;
        self.stall_signal_count = stall_signal_count;
        self.stall_recovery_attempts = stall_recovery_attempts;
        self.stall_observation_active = stall_observation_active;
    }

    #[cfg(test)]
    pub fn stall_signal_count(&self) -> u32 {
        self.stall_signal_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistence_retry_classifies_first_and_repeated_failures() {
        let mut retry = PersistenceRetry::default();
        assert_eq!(
            retry.observe_failure(),
            PersistenceRetryObservation::First { attempt: 1 }
        );
        assert_eq!(
            retry.observe_failure(),
            PersistenceRetryObservation::Repeated { attempt: 2 }
        );
    }

    #[test]
    fn progress_clears_one_active_stall_observation() {
        let mut progress = RuntimeProgress::default();
        let now = Instant::now();
        progress.start_turn(now);
        assert!(matches!(
            progress.observe_stall(true, now),
            RuntimeStallDecision::Observe { .. }
        ));
        assert!(progress.mark_progress(now));
        assert!(!progress.stall_observation_is_active());
        assert!(!progress.mark_progress(now));
    }

    #[test]
    fn stall_caps_close_after_the_existing_signal_and_recovery_budget() {
        let mut progress = RuntimeProgress::default();
        let now = Instant::now();
        progress.start_turn(now);
        for _ in 0..3 {
            assert!(matches!(
                progress.observe_stall(true, now),
                RuntimeStallDecision::Observe {
                    request_recovery: true,
                    ..
                }
            ));
        }
        assert_eq!(
            progress.observe_stall(true, now),
            RuntimeStallDecision::Exhausted
        );
    }
}
