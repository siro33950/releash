use crate::domain::agent_session::aggregates::{
    ProviderAvailability, ProviderExecutable, ResolvedProviderExecutable,
};
use crate::domain::provider_lifecycle::ProviderKind;

pub(crate) trait ProviderAvailabilityReader: Send + Sync {
    fn is_available(&self, provider: ProviderKind) -> bool;

    fn resolved_executable(&self, provider: ProviderKind) -> Option<ResolvedProviderExecutable>;
}

pub(crate) trait ProviderExecutableProbeGateway: Send + Sync {
    fn resolve(&self, executable: &ProviderExecutable) -> ProviderAvailability;
    fn refresh_search_path(&self) -> Result<(), ProviderExecutableProbeGatewayError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderExecutableProbeGatewayError {
    Technical(crate::domain::failure::TechnicalFailure),
}

pub(crate) trait ProviderExecutableConfigRepository: Send + Sync {
    fn configured_executable(
        &self,
        provider: ProviderKind,
    ) -> Result<Option<ProviderExecutable>, ProviderExecutableConfigRepositoryError>;

    fn save_configured_executable(
        &self,
        provider: ProviderKind,
        executable: Option<&ProviderExecutable>,
    ) -> Result<(), ProviderExecutableConfigRepositoryError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderExecutableConfigRepositoryError {
    InvalidInput,
    Technical(crate::domain::failure::TechnicalFailure),
}
