use super::super::ProviderLifecycleInputError;
use super::{ProviderKind, ProviderLifecycleScope, ProviderLifecycleUnavailableReason};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderLifecycleEvent {
    BindingArmed {
        slot_id: String,
        binding_id: String,
        provider: ProviderKind,
        scope: ProviderLifecycleScope,
    },
    SessionAssociated {
        binding_id: String,
        provider_session_id: String,
        transcript_ref: Option<String>,
    },
    TranscriptAssociated {
        binding_id: String,
        transcript_ref: String,
    },
    StopObserved {
        binding_id: String,
    },
    StopFailed {
        binding_id: String,
        reason: String,
    },
    LifecycleUnavailable {
        binding_id: String,
        provider: ProviderKind,
        scope: ProviderLifecycleScope,
        reason: ProviderLifecycleUnavailableReason,
    },
    BindingExpired {
        binding_id: String,
    },
}

impl ProviderLifecycleEvent {
    pub(crate) fn binding_armed(
        slot_id: impl Into<String>,
        binding_id: impl Into<String>,
        provider: ProviderKind,
        scope: ProviderLifecycleScope,
    ) -> Result<Self, ProviderLifecycleInputError> {
        Ok(Self::BindingArmed {
            slot_id: non_empty(slot_id.into(), "slot_id")?,
            binding_id: non_empty(binding_id.into(), "binding_id")?,
            provider,
            scope,
        })
    }

    pub(crate) fn session_associated(
        binding_id: impl Into<String>,
        provider_session_id: impl Into<String>,
        transcript_ref: Option<String>,
    ) -> Result<Self, ProviderLifecycleInputError> {
        Ok(Self::SessionAssociated {
            binding_id: non_empty(binding_id.into(), "binding_id")?,
            provider_session_id: non_empty(provider_session_id.into(), "provider_session_id")?,
            transcript_ref: optional_non_empty(transcript_ref, "transcript_ref")?,
        })
    }

    pub(crate) fn transcript_associated(
        binding_id: impl Into<String>,
        transcript_ref: impl Into<String>,
    ) -> Result<Self, ProviderLifecycleInputError> {
        Ok(Self::TranscriptAssociated {
            binding_id: non_empty(binding_id.into(), "binding_id")?,
            transcript_ref: non_empty(transcript_ref.into(), "transcript_ref")?,
        })
    }

    pub(crate) fn stop_observed(
        binding_id: impl Into<String>,
    ) -> Result<Self, ProviderLifecycleInputError> {
        Ok(Self::StopObserved {
            binding_id: non_empty(binding_id.into(), "binding_id")?,
        })
    }

    pub(crate) fn stop_failed(
        binding_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<Self, ProviderLifecycleInputError> {
        Ok(Self::StopFailed {
            binding_id: non_empty(binding_id.into(), "binding_id")?,
            reason: non_empty(reason.into(), "reason")?,
        })
    }

    pub(crate) fn lifecycle_unavailable(
        binding_id: impl Into<String>,
        provider: ProviderKind,
        scope: ProviderLifecycleScope,
        reason: ProviderLifecycleUnavailableReason,
    ) -> Result<Self, ProviderLifecycleInputError> {
        Ok(Self::LifecycleUnavailable {
            binding_id: non_empty(binding_id.into(), "binding_id")?,
            provider,
            scope,
            reason,
        })
    }

    pub(crate) fn binding_expired(
        binding_id: impl Into<String>,
    ) -> Result<Self, ProviderLifecycleInputError> {
        Ok(Self::BindingExpired {
            binding_id: non_empty(binding_id.into(), "binding_id")?,
        })
    }
}

fn non_empty(value: String, field: &'static str) -> Result<String, ProviderLifecycleInputError> {
    if value.trim().is_empty() {
        Err(ProviderLifecycleInputError::Empty(field))
    } else {
        Ok(value)
    }
}

fn optional_non_empty(
    value: Option<String>,
    field: &'static str,
) -> Result<Option<String>, ProviderLifecycleInputError> {
    value.map(|value| non_empty(value, field)).transpose()
}
