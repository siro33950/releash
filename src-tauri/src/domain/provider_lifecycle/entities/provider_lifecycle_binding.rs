use super::super::value_objects::{
    ProviderKind, ProviderLifecycleEvent, ProviderLifecycleOutcome, ProviderLifecycleRejection,
    ProviderLifecycleScope, ProviderLifecycleSignal, ProviderLifecycleSignalKind,
    ProviderLifecycleSlotId, ProviderLifecycleUnavailableObservation,
    ProviderLifecycleUnavailableReason,
};
use super::super::ProviderLifecycleInputError;
#[cfg(test)]
use super::super::ProviderLifecycleReplayError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderLifecycleBinding {
    binding_id: String,
    provider: ProviderKind,
    scope: ProviderLifecycleScope,
    provider_session_id: Option<String>,
    transcript_ref: Option<String>,
    unavailable: Option<ProviderLifecycleUnavailableReason>,
    expired: bool,
}

impl ProviderLifecycleBinding {
    #[cfg(test)]
    pub(crate) fn rehydrate(
        events: impl IntoIterator<Item = ProviderLifecycleEvent>,
    ) -> Result<Self, ProviderLifecycleReplayError> {
        let mut events = events.into_iter();
        let first = events
            .next()
            .ok_or(ProviderLifecycleReplayError::EmptyHistory)?;
        let ProviderLifecycleEvent::BindingArmed {
            slot_id: _,
            binding_id,
            provider,
            scope,
        } = first
        else {
            return Err(ProviderLifecycleReplayError::FirstEventNotBindingArmed);
        };
        let mut binding = Self::arm(binding_id, provider, scope)
            .map_err(|_| ProviderLifecycleReplayError::InvalidTransition)?;
        for event in events {
            binding.apply_replayed_event(event)?;
        }
        Ok(binding)
    }

    pub(crate) fn arm(
        binding_id: impl Into<String>,
        provider: ProviderKind,
        scope: ProviderLifecycleScope,
    ) -> Result<Self, ProviderLifecycleInputError> {
        let binding_id = binding_id.into();
        if binding_id.trim().is_empty() {
            return Err(ProviderLifecycleInputError::Empty("binding_id"));
        }
        Ok(Self {
            binding_id,
            provider,
            scope,
            provider_session_id: None,
            transcript_ref: None,
            unavailable: None,
            expired: false,
        })
    }

    #[cfg(test)]
    pub(crate) fn provider_session_id(&self) -> Option<&str> {
        self.provider_session_id.as_deref()
    }

    pub(crate) fn binding_id(&self) -> &str {
        &self.binding_id
    }

    pub(crate) fn provider(&self) -> ProviderKind {
        self.provider
    }

    pub(crate) fn scope(&self) -> &ProviderLifecycleScope {
        &self.scope
    }

    pub(crate) fn armed_event(&self, slot_id: &ProviderLifecycleSlotId) -> ProviderLifecycleEvent {
        ProviderLifecycleEvent::BindingArmed {
            slot_id: slot_id.as_str().to_string(),
            binding_id: self.binding_id.clone(),
            provider: self.provider,
            scope: self.scope.clone(),
        }
    }

    #[cfg(test)]
    pub(crate) fn transcript_ref(&self) -> Option<&str> {
        self.transcript_ref.as_deref()
    }

    pub(crate) fn expire(&mut self) -> ProviderLifecycleOutcome {
        if self.expired {
            return ProviderLifecycleOutcome::Duplicate;
        }
        self.expired = true;
        ProviderLifecycleOutcome::Applied(vec![ProviderLifecycleEvent::BindingExpired {
            binding_id: self.binding_id.clone(),
        }])
    }

    pub(crate) fn observe(&mut self, signal: ProviderLifecycleSignal) -> ProviderLifecycleOutcome {
        if self.expired {
            return ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::BindingExpired);
        }
        if signal.binding_id() != self.binding_id {
            return ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::BindingMismatch);
        }
        if signal.provider() != self.provider {
            return ProviderLifecycleOutcome::Rejected(
                ProviderLifecycleRejection::ProviderMismatch,
            );
        }
        if signal.scope() != &self.scope {
            return ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::ScopeMismatch);
        }
        match signal.into_kind() {
            ProviderLifecycleSignalKind::SessionStarted {
                provider_session_id,
                transcript_ref,
            } => self.associate_session(provider_session_id, transcript_ref),
            ProviderLifecycleSignalKind::StopObserved {
                provider_session_id,
                transcript_ref,
            } => self.observe_stop(provider_session_id, transcript_ref),
            ProviderLifecycleSignalKind::StopFailed {
                provider_session_id,
                transcript_ref,
                reason,
            } => self.observe_stop_failure(provider_session_id, transcript_ref, reason),
            ProviderLifecycleSignalKind::ActivityObserved {
                provider_session_id,
                transcript_ref,
                activity: _,
            } => self.observe_activity(provider_session_id, transcript_ref),
        }
    }

    pub(crate) fn mark_unavailable(
        &mut self,
        observation: ProviderLifecycleUnavailableObservation,
    ) -> ProviderLifecycleOutcome {
        if self.expired {
            return ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::BindingExpired);
        }
        if observation.binding_id() != self.binding_id {
            return ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::BindingMismatch);
        }
        if observation.provider() != self.provider {
            return ProviderLifecycleOutcome::Rejected(
                ProviderLifecycleRejection::ProviderMismatch,
            );
        }
        if observation.scope() != &self.scope {
            return ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::ScopeMismatch);
        }
        if self.provider_session_id.is_some() {
            return ProviderLifecycleOutcome::Rejected(
                ProviderLifecycleRejection::SessionAlreadyAssociated,
            );
        }
        if self.unavailable == Some(observation.reason()) {
            return ProviderLifecycleOutcome::Duplicate;
        }
        self.unavailable = Some(observation.reason());
        ProviderLifecycleOutcome::Applied(vec![ProviderLifecycleEvent::LifecycleUnavailable {
            binding_id: self.binding_id.clone(),
            provider: self.provider,
            scope: self.scope.clone(),
            reason: observation.reason(),
        }])
    }

    fn associate_session(
        &mut self,
        provider_session_id: String,
        transcript_ref: Option<String>,
    ) -> ProviderLifecycleOutcome {
        match self.provider_session_id.as_deref() {
            Some(existing) if existing != provider_session_id => {
                return ProviderLifecycleOutcome::Rejected(
                    ProviderLifecycleRejection::ProviderSessionMismatch,
                );
            }
            Some(_) => {}
            None => {
                self.unavailable = None;
                self.provider_session_id = Some(provider_session_id.clone());
                self.transcript_ref = transcript_ref.clone();
                return ProviderLifecycleOutcome::Applied(vec![
                    ProviderLifecycleEvent::SessionAssociated {
                        binding_id: self.binding_id.clone(),
                        provider_session_id,
                        transcript_ref,
                    },
                ]);
            }
        }

        match self.transcript_events(transcript_ref) {
            Ok(events) if events.is_empty() => ProviderLifecycleOutcome::Duplicate,
            Ok(events) => ProviderLifecycleOutcome::Applied(events),
            Err(rejection) => ProviderLifecycleOutcome::Rejected(rejection),
        }
    }

    fn observe_stop(
        &mut self,
        provider_session_id: String,
        transcript_ref: Option<String>,
    ) -> ProviderLifecycleOutcome {
        if let Err(rejection) = self.ensure_session(&provider_session_id) {
            return ProviderLifecycleOutcome::Rejected(rejection);
        }
        let mut events = match self.transcript_events(transcript_ref) {
            Ok(events) => events,
            Err(rejection) => return ProviderLifecycleOutcome::Rejected(rejection),
        };
        events.push(ProviderLifecycleEvent::StopObserved {
            binding_id: self.binding_id.clone(),
        });
        ProviderLifecycleOutcome::Applied(events)
    }

    fn observe_stop_failure(
        &mut self,
        provider_session_id: String,
        transcript_ref: Option<String>,
        reason: String,
    ) -> ProviderLifecycleOutcome {
        if let Err(rejection) = self.ensure_session(&provider_session_id) {
            return ProviderLifecycleOutcome::Rejected(rejection);
        }
        let mut events = match self.transcript_events(transcript_ref) {
            Ok(events) => events,
            Err(rejection) => return ProviderLifecycleOutcome::Rejected(rejection),
        };
        events.push(ProviderLifecycleEvent::StopFailed {
            binding_id: self.binding_id.clone(),
            reason,
        });
        ProviderLifecycleOutcome::Applied(events)
    }

    fn observe_activity(
        &mut self,
        provider_session_id: String,
        transcript_ref: Option<String>,
    ) -> ProviderLifecycleOutcome {
        if let Err(rejection) = self.ensure_session(&provider_session_id) {
            return ProviderLifecycleOutcome::Rejected(rejection);
        }
        match self.transcript_events(transcript_ref) {
            Ok(events) if events.is_empty() => ProviderLifecycleOutcome::Duplicate,
            Ok(events) => ProviderLifecycleOutcome::Applied(events),
            Err(rejection) => ProviderLifecycleOutcome::Rejected(rejection),
        }
    }

    fn ensure_session(&self, provider_session_id: &str) -> Result<(), ProviderLifecycleRejection> {
        match self.provider_session_id.as_deref() {
            None => Err(ProviderLifecycleRejection::SessionNotAssociated),
            Some(existing) if existing != provider_session_id => {
                Err(ProviderLifecycleRejection::ProviderSessionMismatch)
            }
            Some(_) => Ok(()),
        }
    }

    fn transcript_events(
        &mut self,
        transcript_ref: Option<String>,
    ) -> Result<Vec<ProviderLifecycleEvent>, ProviderLifecycleRejection> {
        match (self.transcript_ref.as_deref(), transcript_ref) {
            (_, None) => Ok(Vec::new()),
            (Some(existing), Some(candidate)) if existing != candidate => {
                Err(ProviderLifecycleRejection::TranscriptMismatch)
            }
            (Some(_), Some(_)) => Ok(Vec::new()),
            (None, Some(transcript_ref)) => {
                self.transcript_ref = Some(transcript_ref.clone());
                Ok(vec![ProviderLifecycleEvent::TranscriptAssociated {
                    binding_id: self.binding_id.clone(),
                    transcript_ref,
                }])
            }
        }
    }

    #[cfg(test)]
    fn apply_replayed_event(
        &mut self,
        event: ProviderLifecycleEvent,
    ) -> Result<(), ProviderLifecycleReplayError> {
        let expected = event.clone();
        let outcome = match event {
            ProviderLifecycleEvent::BindingArmed { .. } => {
                return Err(ProviderLifecycleReplayError::DuplicateBindingArmed);
            }
            ProviderLifecycleEvent::SessionAssociated {
                binding_id,
                provider_session_id,
                transcript_ref,
            } => {
                let signal = ProviderLifecycleSignal::session_started(
                    binding_id,
                    self.provider,
                    self.scope.clone(),
                    provider_session_id,
                    transcript_ref.as_deref(),
                )
                .map_err(|_| ProviderLifecycleReplayError::InvalidTransition)?;
                self.observe(signal)
            }
            ProviderLifecycleEvent::TranscriptAssociated {
                binding_id,
                transcript_ref,
            } => {
                let provider_session_id = self
                    .provider_session_id
                    .clone()
                    .ok_or(ProviderLifecycleReplayError::InvalidTransition)?;
                let signal = ProviderLifecycleSignal::session_started(
                    binding_id,
                    self.provider,
                    self.scope.clone(),
                    provider_session_id,
                    Some(&transcript_ref),
                )
                .map_err(|_| ProviderLifecycleReplayError::InvalidTransition)?;
                self.observe(signal)
            }
            ProviderLifecycleEvent::StopObserved { binding_id } => {
                let provider_session_id = self
                    .provider_session_id
                    .clone()
                    .ok_or(ProviderLifecycleReplayError::InvalidTransition)?;
                let signal = ProviderLifecycleSignal::stop_observed(
                    binding_id,
                    self.provider,
                    self.scope.clone(),
                    provider_session_id,
                    None,
                )
                .map_err(|_| ProviderLifecycleReplayError::InvalidTransition)?;
                self.observe(signal)
            }
            ProviderLifecycleEvent::StopFailed { binding_id, reason } => {
                let provider_session_id = self
                    .provider_session_id
                    .clone()
                    .ok_or(ProviderLifecycleReplayError::InvalidTransition)?;
                let signal = ProviderLifecycleSignal::stop_failed(
                    binding_id,
                    self.provider,
                    self.scope.clone(),
                    provider_session_id,
                    None,
                    reason,
                )
                .map_err(|_| ProviderLifecycleReplayError::InvalidTransition)?;
                self.observe(signal)
            }
            ProviderLifecycleEvent::LifecycleUnavailable {
                binding_id,
                provider,
                scope,
                reason,
            } => {
                let observation = ProviderLifecycleUnavailableObservation::new(
                    binding_id, provider, scope, reason,
                )
                .map_err(|_| ProviderLifecycleReplayError::InvalidTransition)?;
                self.mark_unavailable(observation)
            }
            ProviderLifecycleEvent::BindingExpired { binding_id } => {
                if binding_id != self.binding_id {
                    return Err(ProviderLifecycleReplayError::BindingMismatch);
                }
                self.expire()
            }
        };
        match outcome {
            ProviderLifecycleOutcome::Applied(events) if events == vec![expected] => Ok(()),
            ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::BindingMismatch) => {
                Err(ProviderLifecycleReplayError::BindingMismatch)
            }
            ProviderLifecycleOutcome::Applied(_)
            | ProviderLifecycleOutcome::Duplicate
            | ProviderLifecycleOutcome::Rejected(_) => {
                Err(ProviderLifecycleReplayError::InvalidTransition)
            }
        }
    }
}
