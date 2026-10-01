use std::sync::Arc;
use std::sync::{Mutex, RwLock};

use crate::domain::agent_session::aggregates::{
    ProviderExecutable, ProviderRegistry, ProviderRegistryEntry, ProviderUnavailableReason,
    ResolvedProviderExecutable,
};
use crate::domain::agent_session::{
    ProviderAvailabilityReader, ProviderExecutableConfigRepository, ProviderExecutableProbeGateway,
};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::usecase::provider_dto::AgentSessionProviderDto;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderAvailabilitySnapshotDto {
    pub providers: Vec<ProviderAvailabilityItemDto>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderAvailabilityItemDto {
    pub provider: AgentSessionProviderDto,
    pub display_name: String,
    pub default_executable: String,
    pub configured_executable: Option<String>,
    pub effective_executable: String,
    pub available: bool,
    pub resolved_executable: Option<String>,
    pub unavailable_reason: Option<ProviderUnavailableReasonDto>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderUnavailableReasonDto {
    NotFound,
    NotExecutable,
    SearchPathUnavailable,
    ProbeFailed,
}

impl From<ProviderUnavailableReason> for ProviderUnavailableReasonDto {
    fn from(value: ProviderUnavailableReason) -> Self {
        match value {
            ProviderUnavailableReason::NotFound => Self::NotFound,
            ProviderUnavailableReason::NotExecutable => Self::NotExecutable,
            ProviderUnavailableReason::SearchPathUnavailable => Self::SearchPathUnavailable,
            ProviderUnavailableReason::ProbeFailed => Self::ProbeFailed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderAvailabilityUsecaseError {
    InvalidInput,
    Config(crate::domain::agent_session::ProviderExecutableConfigRepositoryError),
    Refresh(crate::domain::agent_session::ProviderExecutableProbeGatewayError),
    Corrupt,
}

pub(crate) struct ProviderAvailabilityUsecase {
    state_publisher: Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
    config: Arc<dyn ProviderExecutableConfigRepository>,
    probe: Arc<dyn ProviderExecutableProbeGateway>,
    registry: RwLock<ProviderRegistry>,
    operation: Mutex<()>,
}

impl ProviderAvailabilityUsecase {
    pub(crate) fn with_state_publisher(
        mut self,
        publisher: crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> Self {
        self.state_publisher = Some(publisher);
        self
    }

    pub(crate) fn initialize(
        config: Arc<dyn ProviderExecutableConfigRepository>,
        probe: Arc<dyn ProviderExecutableProbeGateway>,
    ) -> Result<Self, ProviderAvailabilityUsecaseError> {
        let registry = build_registry(config.as_ref(), probe.as_ref())?;
        Ok(Self {
            state_publisher: None,
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

    pub(crate) fn snapshot_dto(
        &self,
    ) -> Result<ProviderAvailabilitySnapshotDto, ProviderAvailabilityUsecaseError> {
        let registry = self
            .registry
            .read()
            .map_err(|_| ProviderAvailabilityUsecaseError::Corrupt)?;
        Ok(ProviderAvailabilitySnapshotDto {
            providers: registry
                .entries()
                .iter()
                .map(|entry| ProviderAvailabilityItemDto {
                    provider: entry.provider().into(),
                    display_name: entry.display_name().to_string(),
                    default_executable: entry.default_executable().as_str().to_string(),
                    configured_executable: entry
                        .configured_executable()
                        .map(|value| value.as_str().to_string()),
                    effective_executable: entry.effective_executable().as_str().to_string(),
                    available: entry.is_available(),
                    resolved_executable: entry
                        .resolved_executable()
                        .map(|value| value.as_path().to_string_lossy().into_owned()),
                    unavailable_reason: entry.unavailable_reason().map(Into::into),
                })
                .collect(),
        })
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
    ) -> Result<String, ProviderAvailabilityUsecaseError> {
        let executable = ProviderExecutable::new(executable)
            .map_err(|_| ProviderAvailabilityUsecaseError::InvalidInput)?;
        let configured = executable.as_str().to_string();
        self.replace_configured_executable(provider, Some(executable))?;
        Ok(configured)
    }

    pub(crate) fn reset_configured_executable(
        &self,
        provider: ProviderKind,
    ) -> Result<(), ProviderAvailabilityUsecaseError> {
        self.replace_configured_executable(provider, None)
    }

    pub(crate) fn refresh(&self) -> Result<(), ProviderAvailabilityUsecaseError> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| ProviderAvailabilityUsecaseError::Corrupt)?;
        self.probe
            .refresh_search_path()
            .map_err(ProviderAvailabilityUsecaseError::Refresh)?;
        self.rebuild_registry()
    }

    fn replace_configured_executable(
        &self,
        provider: ProviderKind,
        executable: Option<ProviderExecutable>,
    ) -> Result<(), ProviderAvailabilityUsecaseError> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| ProviderAvailabilityUsecaseError::Corrupt)?;
        self.config
            .save_configured_executable(provider, executable.as_ref())
            .map_err(ProviderAvailabilityUsecaseError::Config)?;
        self.rebuild_registry()
    }

    fn rebuild_registry(&self) -> Result<(), ProviderAvailabilityUsecaseError> {
        let next = build_registry(self.config.as_ref(), self.probe.as_ref())?;
        *self
            .registry
            .write()
            .map_err(|_| ProviderAvailabilityUsecaseError::Corrupt)? = next;
        if let Some(publisher) = &self.state_publisher {
            publisher.notify(crate::usecase::state_subscription::StateChangeSource::Providers);
        }
        Ok(())
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
                .map_err(ProviderAvailabilityUsecaseError::Config)?;
            Ok(ProviderRegistryEntry::detect(
                provider,
                configured_executable,
                |effective| probe.resolve(effective),
            ))
        })
        .collect::<Result<Vec<_>, ProviderAvailabilityUsecaseError>>()?;
    ProviderRegistry::new(entries).map_err(|_| ProviderAvailabilityUsecaseError::Corrupt)
}
