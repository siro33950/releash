use crate::domain::agent_session::events::ProviderSessionOwnershipEvent;
use crate::domain::provider_lifecycle::ProviderKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderSessionOwnershipClaimOutcome {
    Claimed,
    AlreadyClaimed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderSessionOwnershipReleaseOutcome {
    Released,
    AlreadyReleased,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderSessionOwnershipReleaseError {
    pub(crate) agent_session_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderSessionOwnershipError {
    EmptyProviderSessionId,
    InvalidEventSequence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderSessionOwnership {
    provider: ProviderKind,
    provider_session_id: String,
    agent_session_id: Option<String>,
    uncommitted_events: Vec<ProviderSessionOwnershipEvent>,
}

impl ProviderSessionOwnership {
    pub(crate) fn new(
        provider: ProviderKind,
        provider_session_id: impl Into<String>,
    ) -> Result<Self, ProviderSessionOwnershipError> {
        let provider_session_id = provider_session_id.into();
        if provider_session_id.trim().is_empty() {
            return Err(ProviderSessionOwnershipError::EmptyProviderSessionId);
        }
        Ok(Self {
            provider,
            provider_session_id,
            agent_session_id: None,
            uncommitted_events: Vec::new(),
        })
    }

    pub(crate) fn restore(
        provider: ProviderKind,
        provider_session_id: impl Into<String>,
        agent_session_id: Option<&str>,
    ) -> Result<Self, ProviderSessionOwnershipError> {
        if agent_session_id.is_some_and(|id| id.trim().is_empty()) {
            return Err(ProviderSessionOwnershipError::InvalidEventSequence);
        }
        let mut ownership = Self::new(provider, provider_session_id)?;
        ownership.agent_session_id = agent_session_id.map(str::to_string);
        Ok(ownership)
    }

    pub(crate) fn claim(
        &mut self,
        agent_session_id: impl Into<String>,
    ) -> Result<ProviderSessionOwnershipClaimOutcome, ProviderSessionAlreadyOwned> {
        let agent_session_id = agent_session_id.into();
        if self.agent_session_id.as_deref() == Some(agent_session_id.as_str()) {
            return Ok(ProviderSessionOwnershipClaimOutcome::AlreadyClaimed);
        }
        if let Some(owner) = &self.agent_session_id {
            return Err(ProviderSessionAlreadyOwned {
                agent_session_id: owner.clone(),
            });
        }
        self.agent_session_id = Some(agent_session_id.clone());
        self.uncommitted_events
            .push(ProviderSessionOwnershipEvent::Claimed {
                provider: self.provider,
                provider_session_id: self.provider_session_id.clone(),
                agent_session_id,
            });
        Ok(ProviderSessionOwnershipClaimOutcome::Claimed)
    }

    pub(crate) fn take_uncommitted_events(&mut self) -> Vec<ProviderSessionOwnershipEvent> {
        std::mem::take(&mut self.uncommitted_events)
    }

    pub(crate) fn agent_session_id(&self) -> Option<&str> {
        self.agent_session_id.as_deref()
    }

    pub(crate) fn release(
        &mut self,
        agent_session_id: &str,
    ) -> Result<ProviderSessionOwnershipReleaseOutcome, ProviderSessionOwnershipReleaseError> {
        let Some(owner) = &self.agent_session_id else {
            return Ok(ProviderSessionOwnershipReleaseOutcome::AlreadyReleased);
        };
        if owner != agent_session_id {
            return Err(ProviderSessionOwnershipReleaseError {
                agent_session_id: owner.clone(),
            });
        }
        let agent_session_id = owner.clone();
        self.agent_session_id = None;
        self.uncommitted_events
            .push(ProviderSessionOwnershipEvent::Released {
                provider: self.provider,
                provider_session_id: self.provider_session_id.clone(),
                agent_session_id,
            });
        Ok(ProviderSessionOwnershipReleaseOutcome::Released)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderSessionAlreadyOwned {
    pub(crate) agent_session_id: String,
}
