use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::domain::agent_session::aggregates::{
    ProviderAvailability, ProviderExecutable, ProviderUnavailableReason, ResolvedProviderExecutable,
};
use crate::domain::agent_session::{
    ProviderExecutableConfigRepository, ProviderExecutableConfigRepositoryError,
    ProviderExecutableProbeGateway, ProviderExecutableProbeGatewayError,
};
use crate::domain::provider_lifecycle::ProviderKind;

#[derive(Default)]
pub struct FakeProviderExecutableConfigRepository {
    overrides: Mutex<HashMap<ProviderKind, ProviderExecutable>>,
    fail_save: AtomicBool,
}

impl FakeProviderExecutableConfigRepository {
    #[cfg(test)]
    pub(crate) fn with_override(provider: ProviderKind, executable: &str) -> Self {
        Self {
            overrides: Mutex::new(HashMap::from([(
                provider,
                ProviderExecutable::new(executable).unwrap(),
            )])),
            fail_save: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_save(&self) {
        self.fail_save.store(true, Ordering::SeqCst);
    }
}

impl ProviderExecutableConfigRepository for FakeProviderExecutableConfigRepository {
    fn configured_executable(
        &self,
        provider: ProviderKind,
    ) -> Result<Option<ProviderExecutable>, ProviderExecutableConfigRepositoryError> {
        Ok(self.overrides.lock().unwrap().get(&provider).cloned())
    }

    fn save_configured_executable(
        &self,
        provider: ProviderKind,
        executable: Option<&ProviderExecutable>,
    ) -> Result<(), ProviderExecutableConfigRepositoryError> {
        if self.fail_save.load(Ordering::SeqCst) {
            return Err(ProviderExecutableConfigRepositoryError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Transient,
                    message: "unavailable".into(),
                },
            ));
        }
        let mut overrides = self.overrides.lock().unwrap();
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

#[derive(Default)]
pub struct FakeProviderExecutableProbeGateway {
    force_missing: AtomicBool,
    pub(crate) refreshes: Mutex<usize>,
}

impl FakeProviderExecutableProbeGateway {
    #[cfg(test)]
    pub(crate) fn set_force_missing(&self, force_missing: bool) {
        self.force_missing.store(force_missing, Ordering::SeqCst);
    }
}

impl ProviderExecutableProbeGateway for FakeProviderExecutableProbeGateway {
    fn resolve(&self, executable: &ProviderExecutable) -> ProviderAvailability {
        if self.force_missing.load(Ordering::SeqCst) || executable.as_str().contains("missing") {
            ProviderAvailability::unavailable(ProviderUnavailableReason::NotFound)
        } else {
            let resolved = if executable.as_str().starts_with('/') {
                executable.as_str().into()
            } else {
                format!("/resolved/{}", executable.as_str()).into()
            };
            ProviderAvailability::available(ResolvedProviderExecutable::new(resolved).unwrap())
        }
    }

    fn refresh_search_path(&self) -> Result<(), ProviderExecutableProbeGatewayError> {
        *self.refreshes.lock().unwrap() += 1;
        Ok(())
    }
}
