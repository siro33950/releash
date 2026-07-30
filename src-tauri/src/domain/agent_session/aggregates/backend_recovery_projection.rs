use crate::domain::agent_session::value_objects::{ContextCarryState, SessionState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendRecoveryProjection {
    pub session_state: SessionState,
    pub error_reason: Option<String>,
    pub queue_paused: bool,
    pub provider_session_id: Option<String>,
    pub provider_session_generation: u64,
    pub provider_session_observation_id: Option<String>,
    pub context_reinjection_generation: Option<u64>,
    pub context_carry: Option<ContextCarryState>,
    pub has_recovery_publication_snapshot: bool,
    pub has_pending_recovery_message: bool,
    pub pending_recovery_failure: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendRecoveryProjectionRejection {
    InvalidObservationIdentity,
    ConflictingProviderIdentity,
    QueuePaused,
    StaleProviderGeneration,
    ProviderGenerationExhausted,
    DurableEvidenceMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderSessionEstablishment {
    Applied,
    AlreadyApplied,
    Fenced,
}

impl BackendRecoveryProjection {
    pub fn owns_provider_establishment(
        &self,
        observation_id: &str,
        backend_session_id: &str,
    ) -> bool {
        self.provider_session_observation_id.as_deref() == Some(observation_id)
            && self.provider_session_id.as_deref() == Some(backend_session_id)
    }

    pub fn observe_provider_session_established(
        &mut self,
        observation_id: &str,
        backend_session_id: &str,
        expected_provider_session_generation: u64,
        context_carry: Option<ContextCarryState>,
    ) -> Result<ProviderSessionEstablishment, BackendRecoveryProjectionRejection> {
        if observation_id.is_empty() {
            return Err(BackendRecoveryProjectionRejection::InvalidObservationIdentity);
        }
        if self.provider_session_observation_id.as_deref() == Some(observation_id) {
            return if self.provider_session_id.as_deref() == Some(backend_session_id) {
                Ok(ProviderSessionEstablishment::AlreadyApplied)
            } else {
                Err(BackendRecoveryProjectionRejection::ConflictingProviderIdentity)
            };
        }
        if self.has_recovery_publication_snapshot
            || self.pending_recovery_failure
            || self.session_state.fences_context_restore()
        {
            return Ok(ProviderSessionEstablishment::Fenced);
        }
        if self.provider_session_generation != expected_provider_session_generation {
            return Err(BackendRecoveryProjectionRejection::StaleProviderGeneration);
        }
        let provider_session_generation = expected_provider_session_generation
            .checked_add(1)
            .ok_or(BackendRecoveryProjectionRejection::ProviderGenerationExhausted)?;
        let reinjection_pending =
            self.context_reinjection_generation == Some(self.provider_session_generation);
        self.provider_session_id = Some(backend_session_id.to_string());
        self.provider_session_generation = provider_session_generation;
        self.provider_session_observation_id = Some(observation_id.to_string());
        if let Some(context_carry) = context_carry {
            self.context_carry = Some(context_carry);
            self.context_reinjection_generation = None;
        } else if reinjection_pending {
            self.context_reinjection_generation = Some(provider_session_generation);
        }
        Ok(ProviderSessionEstablishment::Applied)
    }

    pub fn start(
        &mut self,
        expected_generation: u64,
        publication_state: SessionState,
        publication_error_reason: Option<String>,
    ) -> Result<(), BackendRecoveryProjectionRejection> {
        if self.queue_paused {
            return Err(BackendRecoveryProjectionRejection::QueuePaused);
        }
        if self.provider_session_generation != expected_generation {
            return Err(BackendRecoveryProjectionRejection::StaleProviderGeneration);
        }
        self.provider_session_id = None;
        self.provider_session_observation_id = None;
        self.context_reinjection_generation = None;
        self.context_carry = Some(ContextCarryState::Failed);
        if publication_state.is_closed() {
            self.session_state = publication_state;
            self.error_reason = publication_error_reason;
        }
        self.has_recovery_publication_snapshot = true;
        Ok(())
    }

    pub fn complete(
        &mut self,
        expected_generation: u64,
        provider_session_generation: u64,
        backend_session_id: String,
        observation_id: String,
    ) -> Result<(), BackendRecoveryProjectionRejection> {
        if self.provider_session_generation != expected_generation {
            return Err(BackendRecoveryProjectionRejection::StaleProviderGeneration);
        }
        self.provider_session_id = Some(backend_session_id);
        self.provider_session_generation = provider_session_generation;
        self.provider_session_observation_id = Some(observation_id);
        self.context_reinjection_generation = Some(provider_session_generation);
        self.has_pending_recovery_message = true;
        self.has_recovery_publication_snapshot = false;
        Ok(())
    }

    pub fn complete_from_readback(
        &mut self,
        old_provider_session_generation: u64,
        provider_session_generation: u64,
        backend_session_id: &str,
        observation_id: String,
    ) -> Result<(), BackendRecoveryProjectionRejection> {
        let expected = old_provider_session_generation
            .checked_add(1)
            .ok_or(BackendRecoveryProjectionRejection::ProviderGenerationExhausted)?;
        if provider_session_generation != expected
            || self.provider_session_generation != provider_session_generation
            || self.provider_session_id.as_deref() != Some(backend_session_id)
            || self.context_reinjection_generation != Some(provider_session_generation)
        {
            return Err(BackendRecoveryProjectionRejection::DurableEvidenceMismatch);
        }
        self.provider_session_observation_id = Some(observation_id);
        self.has_pending_recovery_message = true;
        self.has_recovery_publication_snapshot = false;
        Ok(())
    }

    pub fn fail(&mut self, error: Option<String>) {
        self.session_state = SessionState::Error;
        self.provider_session_observation_id = None;
        if error.is_some() {
            self.error_reason = error;
        }
        self.has_pending_recovery_message = true;
        self.has_recovery_publication_snapshot = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projection() -> BackendRecoveryProjection {
        BackendRecoveryProjection {
            session_state: SessionState::Done,
            error_reason: None,
            queue_paused: false,
            provider_session_id: Some("provider-1".into()),
            provider_session_generation: 4,
            provider_session_observation_id: Some("old".into()),
            context_reinjection_generation: None,
            context_carry: Some(ContextCarryState::Resumed),
            has_recovery_publication_snapshot: false,
            has_pending_recovery_message: false,
            pending_recovery_failure: false,
        }
    }

    #[test]
    fn recovery_start_is_fenced_by_queue_and_generation() {
        let mut paused = projection();
        paused.queue_paused = true;
        assert_eq!(
            paused.start(4, SessionState::Done, None),
            Err(BackendRecoveryProjectionRejection::QueuePaused)
        );
        let mut stale = projection();
        assert_eq!(
            stale.start(3, SessionState::Done, None),
            Err(BackendRecoveryProjectionRejection::StaleProviderGeneration)
        );
    }

    #[test]
    fn recovery_start_preserves_closed_public_lifecycle() {
        let mut state = projection();
        state
            .start(4, SessionState::Closed, Some("closed".into()))
            .unwrap();
        assert_eq!(state.session_state, SessionState::Closed);
        assert_eq!(state.error_reason.as_deref(), Some("closed"));
        assert!(state.provider_session_id.is_none());
        assert_eq!(state.context_carry, Some(ContextCarryState::Failed));
    }

    #[test]
    fn readback_requires_the_exact_durable_provider_generation() {
        let mut state = projection();
        state.provider_session_id = Some("provider-2".into());
        state.provider_session_generation = 5;
        state.context_reinjection_generation = Some(5);
        state
            .complete_from_readback(4, 5, "provider-2", "observation".into())
            .unwrap();
        assert!(state.has_pending_recovery_message);
        assert!(!state.has_recovery_publication_snapshot);
    }

    #[test]
    fn provider_establishment_is_idempotent_and_fenced_by_recovery() {
        let mut state = projection();
        assert_eq!(
            state
                .observe_provider_session_established("observation", "provider-2", 4, None)
                .unwrap(),
            ProviderSessionEstablishment::Applied
        );
        assert_eq!(
            state
                .observe_provider_session_established("observation", "provider-2", 4, None)
                .unwrap(),
            ProviderSessionEstablishment::AlreadyApplied
        );
        assert!(state.owns_provider_establishment("observation", "provider-2"));
        assert!(!state.owns_provider_establishment("other", "provider-2"));

        let mut fenced = projection();
        fenced.pending_recovery_failure = true;
        assert_eq!(
            fenced
                .observe_provider_session_established("observation", "provider-2", 4, None)
                .unwrap(),
            ProviderSessionEstablishment::Fenced
        );
    }
}
