/// provider CLI へそのまま渡す起動設定。値域は provider CLI が定め、
/// Releash は写像・検証を行わない。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ProviderLaunchOptions {
    pub(crate) model: Option<String>,
    pub(crate) permission: Option<String>,
}

impl ProviderLaunchOptions {
    pub(crate) fn new(model: Option<String>, permission: Option<String>) -> Self {
        Self { model, permission }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProviderSessionLaunchMode {
    New,
    NewWithInitialInstruction(String),
    Resume(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderSessionLaunch {
    mode: ProviderSessionLaunchMode,
    options: ProviderLaunchOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderSessionLaunchError {
    InitialInstructionMissing,
    ProviderSessionIdMissing,
}

impl ProviderSessionLaunch {
    #[allow(non_upper_case_globals)]
    pub(crate) const New: Self = Self {
        mode: ProviderSessionLaunchMode::New,
        options: ProviderLaunchOptions {
            model: None,
            permission: None,
        },
    };

    pub(crate) fn new_with_initial_instruction(
        initial_instruction: impl Into<String>,
    ) -> Result<Self, ProviderSessionLaunchError> {
        let initial_instruction = initial_instruction.into();
        if initial_instruction.trim().is_empty() {
            return Err(ProviderSessionLaunchError::InitialInstructionMissing);
        }
        Ok(Self {
            mode: ProviderSessionLaunchMode::NewWithInitialInstruction(initial_instruction),
            options: ProviderLaunchOptions::default(),
        })
    }

    pub(crate) fn resume(
        provider_session_id: impl Into<String>,
    ) -> Result<Self, ProviderSessionLaunchError> {
        let provider_session_id = provider_session_id.into();
        if provider_session_id.trim().is_empty() {
            return Err(ProviderSessionLaunchError::ProviderSessionIdMissing);
        }
        Ok(Self {
            mode: ProviderSessionLaunchMode::Resume(provider_session_id),
            options: ProviderLaunchOptions::default(),
        })
    }

    pub(crate) fn with_options(mut self, options: ProviderLaunchOptions) -> Self {
        self.options = options;
        self
    }

    pub(crate) fn options(&self) -> &ProviderLaunchOptions {
        &self.options
    }

    pub(crate) fn provider_session_id(&self) -> Option<&str> {
        match &self.mode {
            ProviderSessionLaunchMode::New
            | ProviderSessionLaunchMode::NewWithInitialInstruction(_) => None,
            ProviderSessionLaunchMode::Resume(provider_session_id) => Some(provider_session_id),
        }
    }

    pub(crate) fn initial_instruction(&self) -> Option<&str> {
        match &self.mode {
            ProviderSessionLaunchMode::NewWithInitialInstruction(initial_instruction) => {
                Some(initial_instruction)
            }
            ProviderSessionLaunchMode::New | ProviderSessionLaunchMode::Resume(_) => None,
        }
    }
}
