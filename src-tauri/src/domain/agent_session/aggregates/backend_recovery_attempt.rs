use crate::domain::agent_session::events::BackendSessionRecoveryReason;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendRecoveryTurnResume {
    NoStartedTurn,
    AwaitingAcceptedTurnStart,
    AcceptedTurnStarted { context_was_reinjected: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendRecoveryCompletion {
    pub recovery_id: String,
    pub old_provider_session_generation: u64,
    pub reason: BackendSessionRecoveryReason,
    pub backend_session_id: String,
    pub context_was_reinjected: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderIdentityObservation {
    Recorded,
    AlreadyObserved,
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendRecoveryFailureClaim {
    Claimed,
    Joined,
    Rejected,
}

#[derive(Debug)]
pub struct BackendRecoveryAttempt {
    recovery_id: String,
    old_provider_session_generation: u64,
    reason: BackendSessionRecoveryReason,
    pending_failure: Option<String>,
    turn_resume: BackendRecoveryTurnResume,
    observed_backend_session_id: Option<String>,
    completion_in_flight: bool,
    failure_in_flight: bool,
}

impl BackendRecoveryAttempt {
    pub fn start(
        recovery_id: String,
        old_provider_session_generation: u64,
        reason: BackendSessionRecoveryReason,
        has_accepted_turn: bool,
    ) -> Self {
        Self {
            recovery_id,
            old_provider_session_generation,
            reason,
            pending_failure: None,
            turn_resume: if has_accepted_turn {
                BackendRecoveryTurnResume::AwaitingAcceptedTurnStart
            } else {
                BackendRecoveryTurnResume::NoStartedTurn
            },
            observed_backend_session_id: None,
            completion_in_flight: false,
            failure_in_flight: false,
        }
    }

    pub fn recovery_id(&self) -> &str {
        &self.recovery_id
    }

    pub fn matches_recovery_id(&self, recovery_id: &str) -> bool {
        self.recovery_id == recovery_id
    }

    pub fn pending_failure(&self) -> Option<&str> {
        self.pending_failure.as_deref()
    }

    pub fn completion_in_flight(&self) -> bool {
        self.completion_in_flight
    }

    pub fn failure_in_flight(&self) -> bool {
        self.failure_in_flight
    }

    pub fn observe_provider_identity(
        &mut self,
        backend_session_id: &str,
    ) -> ProviderIdentityObservation {
        match self.observed_backend_session_id.as_deref() {
            Some(observed) if observed == backend_session_id => {
                ProviderIdentityObservation::AlreadyObserved
            }
            Some(_) => ProviderIdentityObservation::Conflict,
            None => {
                self.observed_backend_session_id = Some(backend_session_id.to_string());
                ProviderIdentityObservation::Recorded
            }
        }
    }

    pub fn accepted_turn_started(&mut self, context_was_reinjected: bool) -> bool {
        match self.turn_resume {
            BackendRecoveryTurnResume::AwaitingAcceptedTurnStart => {
                self.turn_resume = BackendRecoveryTurnResume::AcceptedTurnStarted {
                    context_was_reinjected,
                };
                true
            }
            BackendRecoveryTurnResume::AcceptedTurnStarted {
                context_was_reinjected: current,
            } => current == context_was_reinjected,
            BackendRecoveryTurnResume::NoStartedTurn => false,
        }
    }

    pub fn claim_completion(&mut self) -> Option<BackendRecoveryCompletion> {
        if self.pending_failure.is_some() || self.completion_in_flight {
            return None;
        }
        let backend_session_id = self.observed_backend_session_id.clone()?;
        let context_was_reinjected = match self.turn_resume {
            BackendRecoveryTurnResume::NoStartedTurn => None,
            BackendRecoveryTurnResume::AwaitingAcceptedTurnStart => return None,
            BackendRecoveryTurnResume::AcceptedTurnStarted {
                context_was_reinjected,
            } => Some(context_was_reinjected),
        };
        self.completion_in_flight = true;
        Some(BackendRecoveryCompletion {
            recovery_id: self.recovery_id.clone(),
            old_provider_session_generation: self.old_provider_session_generation,
            reason: self.reason,
            backend_session_id,
            context_was_reinjected,
        })
    }

    pub fn owns_completion(&self, recovery_id: &str) -> bool {
        self.recovery_id == recovery_id
            && self.completion_in_flight
            && self.pending_failure.is_none()
    }

    pub fn claim_failure(&mut self, error: &str) -> BackendRecoveryFailureClaim {
        if self.completion_in_flight {
            return BackendRecoveryFailureClaim::Rejected;
        }
        if self.failure_in_flight {
            return if self.pending_failure.as_deref() == Some(error) {
                BackendRecoveryFailureClaim::Joined
            } else {
                BackendRecoveryFailureClaim::Rejected
            };
        }
        self.pending_failure = Some(error.to_string());
        self.failure_in_flight = true;
        BackendRecoveryFailureClaim::Claimed
    }

    pub fn owns_failure(&self, recovery_id: &str, error: &str) -> bool {
        self.recovery_id == recovery_id
            && self.failure_in_flight
            && !self.completion_in_flight
            && self.pending_failure.as_deref() == Some(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(has_accepted_turn: bool) -> BackendRecoveryAttempt {
        BackendRecoveryAttempt::start(
            "recovery-1".to_string(),
            7,
            BackendSessionRecoveryReason::BackendSessionLost,
            has_accepted_turn,
        )
    }

    #[test]
    fn completion_waits_for_provider_identity_and_accepted_turn_start() {
        let mut recovery = attempt(true);
        assert!(recovery.claim_completion().is_none());
        assert_eq!(
            recovery.observe_provider_identity("provider-2"),
            ProviderIdentityObservation::Recorded
        );
        assert!(recovery.claim_completion().is_none());
        assert!(recovery.accepted_turn_started(true));

        let completion = recovery.claim_completion().expect("completion");

        assert_eq!(completion.recovery_id, "recovery-1");
        assert_eq!(completion.backend_session_id, "provider-2");
        assert_eq!(completion.context_was_reinjected, Some(true));
        assert!(recovery.owns_completion("recovery-1"));
    }

    #[test]
    fn provider_identity_conflict_does_not_replace_the_first_observation() {
        let mut recovery = attempt(false);
        assert_eq!(
            recovery.observe_provider_identity("provider-2"),
            ProviderIdentityObservation::Recorded
        );
        assert_eq!(
            recovery.observe_provider_identity("provider-3"),
            ProviderIdentityObservation::Conflict
        );
        assert_eq!(
            recovery.claim_completion().unwrap().backend_session_id,
            "provider-2"
        );
    }

    #[test]
    fn failure_and_completion_claims_are_mutually_exclusive_and_idempotent() {
        let mut failure = attempt(false);
        assert_eq!(
            failure.claim_failure("broken"),
            BackendRecoveryFailureClaim::Claimed
        );
        assert_eq!(
            failure.claim_failure("broken"),
            BackendRecoveryFailureClaim::Joined
        );
        assert_eq!(
            failure.claim_failure("different"),
            BackendRecoveryFailureClaim::Rejected
        );
        assert!(failure.owns_failure("recovery-1", "broken"));
        assert!(failure.claim_completion().is_none());

        let mut completion = attempt(false);
        completion.observe_provider_identity("provider-2");
        assert!(completion.claim_completion().is_some());
        assert_eq!(
            completion.claim_failure("broken"),
            BackendRecoveryFailureClaim::Rejected
        );
    }
}
