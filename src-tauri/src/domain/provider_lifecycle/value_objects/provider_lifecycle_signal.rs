use super::super::ProviderLifecycleInputError;
use super::{ProviderKind, ProviderLifecycleScope};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderLifecycleSignal {
    binding_id: String,
    provider: ProviderKind,
    scope: ProviderLifecycleScope,
    kind: ProviderLifecycleSignalKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderLifecycleSignalKind {
    SessionStarted {
        provider_session_id: String,
        transcript_ref: Option<String>,
    },
    StopObserved {
        provider_session_id: String,
        transcript_ref: Option<String>,
    },
    StopFailed {
        provider_session_id: String,
        transcript_ref: Option<String>,
        reason: String,
    },
}

impl ProviderLifecycleSignal {
    pub(crate) fn session_started(
        binding_id: impl Into<String>,
        provider: ProviderKind,
        scope: ProviderLifecycleScope,
        provider_session_id: impl Into<String>,
        transcript_ref: Option<&str>,
    ) -> Result<Self, ProviderLifecycleInputError> {
        Self::new(
            binding_id,
            provider,
            scope,
            ProviderLifecycleSignalKind::SessionStarted {
                provider_session_id: non_empty(provider_session_id.into(), "provider_session_id")?,
                transcript_ref: optional_non_empty(transcript_ref, "transcript_ref")?,
            },
        )
    }

    pub(crate) fn stop_observed(
        binding_id: impl Into<String>,
        provider: ProviderKind,
        scope: ProviderLifecycleScope,
        provider_session_id: impl Into<String>,
        transcript_ref: Option<&str>,
    ) -> Result<Self, ProviderLifecycleInputError> {
        Self::new(
            binding_id,
            provider,
            scope,
            ProviderLifecycleSignalKind::StopObserved {
                provider_session_id: non_empty(provider_session_id.into(), "provider_session_id")?,
                transcript_ref: optional_non_empty(transcript_ref, "transcript_ref")?,
            },
        )
    }

    pub(crate) fn stop_failed(
        binding_id: impl Into<String>,
        provider: ProviderKind,
        scope: ProviderLifecycleScope,
        provider_session_id: impl Into<String>,
        transcript_ref: Option<&str>,
        reason: impl Into<String>,
    ) -> Result<Self, ProviderLifecycleInputError> {
        Self::new(
            binding_id,
            provider,
            scope,
            ProviderLifecycleSignalKind::StopFailed {
                provider_session_id: non_empty(provider_session_id.into(), "provider_session_id")?,
                transcript_ref: optional_non_empty(transcript_ref, "transcript_ref")?,
                reason: non_empty(reason.into(), "reason")?,
            },
        )
    }

    fn new(
        binding_id: impl Into<String>,
        provider: ProviderKind,
        scope: ProviderLifecycleScope,
        kind: ProviderLifecycleSignalKind,
    ) -> Result<Self, ProviderLifecycleInputError> {
        Ok(Self {
            binding_id: non_empty(binding_id.into(), "binding_id")?,
            provider,
            scope,
            kind,
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

    pub(crate) fn into_kind(self) -> ProviderLifecycleSignalKind {
        self.kind
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
    value: Option<&str>,
    field: &'static str,
) -> Result<Option<String>, ProviderLifecycleInputError> {
    value
        .map(|value| non_empty(value.to_string(), field))
        .transpose()
}
