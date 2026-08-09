#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderSessionLaunch {
    New,
    Resume(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderSessionLaunchError {
    ProviderSessionIdMissing,
}

impl ProviderSessionLaunch {
    pub(crate) fn resume(
        provider_session_id: impl Into<String>,
    ) -> Result<Self, ProviderSessionLaunchError> {
        let provider_session_id = provider_session_id.into();
        if provider_session_id.trim().is_empty() {
            return Err(ProviderSessionLaunchError::ProviderSessionIdMissing);
        }
        Ok(Self::Resume(provider_session_id))
    }

    pub(crate) fn provider_session_id(&self) -> Option<&str> {
        match self {
            Self::New => None,
            Self::Resume(provider_session_id) => Some(provider_session_id),
        }
    }
}
