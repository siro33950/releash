#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderSessionLaunch {
    New,
    NewWithInitialInstruction(String),
    Resume(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderSessionLaunchError {
    InitialInstructionMissing,
    ProviderSessionIdMissing,
}

impl ProviderSessionLaunch {
    pub(crate) fn new_with_initial_instruction(
        initial_instruction: impl Into<String>,
    ) -> Result<Self, ProviderSessionLaunchError> {
        let initial_instruction = initial_instruction.into();
        if initial_instruction.trim().is_empty() {
            return Err(ProviderSessionLaunchError::InitialInstructionMissing);
        }
        Ok(Self::NewWithInitialInstruction(initial_instruction))
    }

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
            Self::New | Self::NewWithInitialInstruction(_) => None,
            Self::Resume(provider_session_id) => Some(provider_session_id),
        }
    }

    pub(crate) fn initial_instruction(&self) -> Option<&str> {
        match self {
            Self::NewWithInitialInstruction(initial_instruction) => Some(initial_instruction),
            Self::New | Self::Resume(_) => None,
        }
    }
}
