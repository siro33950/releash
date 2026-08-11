use std::sync::Arc;
use std::sync::{Mutex, RwLock};

use crate::domain::agent_session::aggregates::{
    ProviderExecutable, ProviderRegistry, ProviderRegistryEntry, ResolvedProviderExecutable,
};
use crate::domain::agent_session::{
    ProviderAvailabilityReader, ProviderExecutableConfigRepository, ProviderExecutableProbeGateway,
};
use crate::domain::provider_lifecycle::ProviderKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderAvailabilityUsecaseError {
    InvalidInput,
    ConfigUnavailable,
    RefreshUnavailable,
    Corrupt,
}

pub(crate) struct ProviderAvailabilityUsecase {
    config: Arc<dyn ProviderExecutableConfigRepository>,
    probe: Arc<dyn ProviderExecutableProbeGateway>,
    registry: RwLock<ProviderRegistry>,
    operation: Mutex<()>,
}

impl ProviderAvailabilityUsecase {
    pub(crate) fn initialize(
        config: Arc<dyn ProviderExecutableConfigRepository>,
        probe: Arc<dyn ProviderExecutableProbeGateway>,
    ) -> Result<Self, ProviderAvailabilityUsecaseError> {
        let registry = build_registry(config.as_ref(), probe.as_ref())?;
        Ok(Self {
            config,
            probe,
            registry: RwLock::new(registry),
            operation: Mutex::new(()),
        })
    }

    pub(crate) fn snapshot(&self) -> Result<ProviderRegistry, ProviderAvailabilityUsecaseError> {
        self.registry
            .read()
            .map(|registry| registry.clone())
            .map_err(|_| ProviderAvailabilityUsecaseError::Corrupt)
    }

    pub(crate) fn available_providers(
        &self,
    ) -> Result<Vec<ProviderKind>, ProviderAvailabilityUsecaseError> {
        Ok(self
            .snapshot()?
            .entries()
            .iter()
            .filter(|entry| entry.is_available())
            .map(ProviderRegistryEntry::provider)
            .collect())
    }

    pub(crate) fn update_configured_executable(
        &self,
        provider: ProviderKind,
        executable: &str,
    ) -> Result<ProviderRegistry, ProviderAvailabilityUsecaseError> {
        let executable = ProviderExecutable::new(executable)
            .map_err(|_| ProviderAvailabilityUsecaseError::InvalidInput)?;
        self.replace_configured_executable(provider, Some(executable))
    }

    pub(crate) fn reset_configured_executable(
        &self,
        provider: ProviderKind,
    ) -> Result<ProviderRegistry, ProviderAvailabilityUsecaseError> {
        self.replace_configured_executable(provider, None)
    }

    pub(crate) fn refresh(&self) -> Result<ProviderRegistry, ProviderAvailabilityUsecaseError> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| ProviderAvailabilityUsecaseError::Corrupt)?;
        self.probe
            .refresh_search_path()
            .map_err(|_| ProviderAvailabilityUsecaseError::RefreshUnavailable)?;
        self.rebuild_registry()
    }

    fn replace_configured_executable(
        &self,
        provider: ProviderKind,
        executable: Option<ProviderExecutable>,
    ) -> Result<ProviderRegistry, ProviderAvailabilityUsecaseError> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| ProviderAvailabilityUsecaseError::Corrupt)?;
        self.config
            .save_configured_executable(provider, executable.as_ref())
            .map_err(|_| ProviderAvailabilityUsecaseError::ConfigUnavailable)?;
        self.rebuild_registry()
    }

    fn rebuild_registry(&self) -> Result<ProviderRegistry, ProviderAvailabilityUsecaseError> {
        let next = build_registry(self.config.as_ref(), self.probe.as_ref())?;
        *self
            .registry
            .write()
            .map_err(|_| ProviderAvailabilityUsecaseError::Corrupt)? = next.clone();
        Ok(next)
    }
}

impl ProviderAvailabilityReader for ProviderAvailabilityUsecase {
    fn is_available(&self, provider: ProviderKind) -> bool {
        self.registry
            .read()
            .map(|registry| registry.entry(provider).is_available())
            .unwrap_or(false)
    }

    fn resolved_executable(&self, provider: ProviderKind) -> Option<ResolvedProviderExecutable> {
        self.registry
            .read()
            .ok()
            .and_then(|registry| registry.entry(provider).resolved_executable().cloned())
    }
}

fn build_registry(
    config: &dyn ProviderExecutableConfigRepository,
    probe: &dyn ProviderExecutableProbeGateway,
) -> Result<ProviderRegistry, ProviderAvailabilityUsecaseError> {
    let entries = ProviderKind::supported()
        .iter()
        .copied()
        .map(|provider| {
            let configured_executable = config
                .configured_executable(provider)
                .map_err(|_| ProviderAvailabilityUsecaseError::ConfigUnavailable)?;
            Ok(ProviderRegistryEntry::detect(
                provider,
                configured_executable,
                |effective| probe.resolve(effective),
            ))
        })
        .collect::<Result<Vec<_>, ProviderAvailabilityUsecaseError>>()?;
    ProviderRegistry::new(entries).map_err(|_| ProviderAvailabilityUsecaseError::Corrupt)
}
