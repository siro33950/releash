use std::sync::Arc;
use std::sync::{Mutex, RwLock};

use crate::domain::agent_session::aggregates::{
    ProviderExecutable, ProviderRegistry, ProviderRegistryEntry, ProviderUnavailableReason,
    ResolvedProviderExecutable,
};
use crate::domain::agent_session::{ProviderAvailabilityReader, ProviderExecutableProbeGateway};
use crate::domain::app_config::{AppConfigError, AppConfigRepository};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::usecase::provider_dto::AgentSessionProviderDto;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAvailabilitySnapshotDto {
    pub providers: Vec<ProviderAvailabilityItemDto>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAvailabilityItemDto {
    pub provider: AgentSessionProviderDto,
    pub display_name: String,
    pub default_executable: String,
    pub configured_executable: Option<String>,
    pub configuration_revision: u32,
    pub effective_executable: String,
    pub available: bool,
    pub resolved_executable: Option<String>,
    pub unavailable_reason: Option<ProviderUnavailableReasonDto>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderUnavailableReasonDto {
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
pub enum ProviderAvailabilityUsecaseError {
    InvalidInput,
    Config(AppConfigError),
    Refresh(crate::domain::agent_session::ProviderExecutableProbeGatewayError),
    Corrupt,
}

impl ProviderAvailabilityUsecaseError {
    pub(crate) fn technical_failure(&self) -> Option<&crate::domain::failure::TechnicalFailure> {
        match self {
            Self::Config(AppConfigError::Technical(failure))
            | Self::Refresh(
                crate::domain::agent_session::ProviderExecutableProbeGatewayError::Technical(
                    failure,
                ),
            ) => Some(failure),
            Self::InvalidInput | Self::Corrupt | Self::Config(AppConfigError::InvalidInput(_)) => {
                None
            }
        }
    }
}

pub struct ProviderAvailabilityUsecase {
    config_mutation: Arc<parking_lot::Mutex<()>>,
    state_publisher: Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
    config: Arc<dyn AppConfigRepository>,
    probe: Arc<dyn ProviderExecutableProbeGateway>,
    registry: RwLock<ProviderRegistry>,
    operation: Mutex<()>,
}

impl ProviderAvailabilityUsecase {
    pub fn with_state_publisher(
        mut self,
        publisher: crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> Self {
        self.state_publisher = Some(publisher);
        self
    }

    pub fn initialize(
        config: Arc<dyn AppConfigRepository>,
        probe: Arc<dyn ProviderExecutableProbeGateway>,
        config_mutation: Arc<parking_lot::Mutex<()>>,
    ) -> Result<Self, ProviderAvailabilityUsecaseError> {
        let registry = build_registry(config.as_ref(), probe.as_ref())?;
        Ok(Self {
            config_mutation,
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
        let _operation = self
            .operation
            .lock()
            .map_err(|_| ProviderAvailabilityUsecaseError::Corrupt)?;
        let config = self
            .config
            .load()
            .map_err(ProviderAvailabilityUsecaseError::Config)?;
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
                    configuration_revision: config
                        .provider_configuration_revision(entry.provider()),
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

    /// provider の実行ファイル指定を保存し、利用可能状態を更新する。
    pub(crate) fn update_configured_executable(
        &self,
        provider: ProviderKind,
        executable: &str,
    ) -> Result<(), ProviderAvailabilityUsecaseError> {
        let executable = ProviderExecutable::new(executable)
            .map_err(|_| ProviderAvailabilityUsecaseError::InvalidInput)?;
        let _operation = self
            .operation
            .lock()
            .map_err(|_| ProviderAvailabilityUsecaseError::Corrupt)?;
        let guard = self.config_mutation.lock();
        let mut config = self
            .config
            .load()
            .map_err(ProviderAvailabilityUsecaseError::Config)?;
        config
            .update_provider_executable(provider, Some(executable))
            .map_err(|_| ProviderAvailabilityUsecaseError::Corrupt)?;
        self.config
            .save(config)
            .map_err(ProviderAvailabilityUsecaseError::Config)?;
        drop(guard);
        self.rebuild_registry()
    }

    /// provider の実行ファイルの上書きを解除し、利用可能状態を更新する。
    pub(crate) fn reset_configured_executable(
        &self,
        provider: ProviderKind,
    ) -> Result<(), ProviderAvailabilityUsecaseError> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| ProviderAvailabilityUsecaseError::Corrupt)?;
        let guard = self.config_mutation.lock();
        let mut config = self
            .config
            .load()
            .map_err(ProviderAvailabilityUsecaseError::Config)?;
        config
            .update_provider_executable(provider, None)
            .map_err(|_| ProviderAvailabilityUsecaseError::Corrupt)?;
        self.config
            .save(config)
            .map_err(ProviderAvailabilityUsecaseError::Config)?;
        drop(guard);
        self.rebuild_registry()
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
    config: &dyn AppConfigRepository,
    probe: &dyn ProviderExecutableProbeGateway,
) -> Result<ProviderRegistry, ProviderAvailabilityUsecaseError> {
    let config = config
        .load()
        .map_err(ProviderAvailabilityUsecaseError::Config)?;
    let entries = ProviderKind::supported()
        .iter()
        .copied()
        .map(|provider| {
            ProviderRegistryEntry::detect(
                provider,
                config.provider_executable(provider).cloned(),
                |effective| probe.resolve(effective),
            )
        })
        .collect();
    ProviderRegistry::new(entries).map_err(|_| ProviderAvailabilityUsecaseError::Corrupt)
}

#[cfg(test)]
#[path = "provider_availability_test.rs"]
mod provider_availability_tests;
