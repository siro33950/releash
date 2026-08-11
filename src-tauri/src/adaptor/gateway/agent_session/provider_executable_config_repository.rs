use std::collections::HashMap;
use std::sync::Mutex;

use crate::domain::agent_session::aggregates::ProviderExecutable;
use crate::domain::agent_session::{
    ProviderExecutableConfigRepository, ProviderExecutableConfigRepositoryError,
};
use crate::domain::provider_lifecycle::ProviderKind;

pub(crate) struct InMemoryProviderExecutableConfigRepository {
    overrides: Mutex<HashMap<ProviderKind, ProviderExecutable>>,
}

impl InMemoryProviderExecutableConfigRepository {
    pub(crate) fn new(
        claude: Option<String>,
        codex: Option<String>,
    ) -> Result<Self, ProviderExecutableConfigRepositoryError> {
        let mut overrides = HashMap::new();
        for (provider, executable) in [(ProviderKind::Claude, claude), (ProviderKind::Codex, codex)]
        {
            if let Some(executable) = executable {
                overrides.insert(
                    provider,
                    ProviderExecutable::new(executable)
                        .map_err(|_| ProviderExecutableConfigRepositoryError::InvalidInput)?,
                );
            }
        }
        Ok(Self {
            overrides: Mutex::new(overrides),
        })
    }
}

impl ProviderExecutableConfigRepository for InMemoryProviderExecutableConfigRepository {
    fn configured_executable(
        &self,
        provider: ProviderKind,
    ) -> Result<Option<ProviderExecutable>, ProviderExecutableConfigRepositoryError> {
        self.overrides
            .lock()
            .map(|overrides| overrides.get(&provider).cloned())
            .map_err(|_| ProviderExecutableConfigRepositoryError::Unavailable)
    }

    fn save_configured_executable(
        &self,
        provider: ProviderKind,
        executable: Option<&ProviderExecutable>,
    ) -> Result<(), ProviderExecutableConfigRepositoryError> {
        let mut overrides = self
            .overrides
            .lock()
            .map_err(|_| ProviderExecutableConfigRepositoryError::Unavailable)?;
        match executable {
            Some(executable) => {
                overrides.insert(provider, executable.clone());
            }
            None => {
                overrides.remove(&provider);
            }
        }
        Ok(())
    }
}
