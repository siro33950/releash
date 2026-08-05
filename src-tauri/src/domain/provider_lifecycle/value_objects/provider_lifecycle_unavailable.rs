use super::super::ProviderLifecycleInputError;
use super::{ProviderKind, ProviderLifecycleScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderLifecycleUnavailableReason {
    SessionStartDeadlineExceeded,
    CodexHookDeliveryUnconfirmed,
    ProviderHookConfigurationRejected,
    LocalApiUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderLifecycleUnavailableObservation {
    binding_id: String,
    provider: ProviderKind,
    scope: ProviderLifecycleScope,
    reason: ProviderLifecycleUnavailableReason,
}

impl ProviderLifecycleUnavailableObservation {
    pub(crate) fn new(
        binding_id: impl Into<String>,
        provider: ProviderKind,
        scope: ProviderLifecycleScope,
        reason: ProviderLifecycleUnavailableReason,
    ) -> Result<Self, ProviderLifecycleInputError> {
        let binding_id = binding_id.into();
        if binding_id.trim().is_empty() {
            return Err(ProviderLifecycleInputError::Empty("binding_id"));
        }
        Ok(Self {
            binding_id,
            provider,
            scope,
            reason,
        })
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

    pub(crate) fn reason(&self) -> ProviderLifecycleUnavailableReason {
        self.reason
    }
}
