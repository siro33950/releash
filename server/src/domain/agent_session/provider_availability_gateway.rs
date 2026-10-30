use crate::domain::agent_session::aggregates::{
    ProviderAvailability, ProviderExecutable, ResolvedProviderExecutable,
};
use crate::domain::provider_lifecycle::ProviderKind;

pub trait ProviderAvailabilityReader: Send + Sync {
    fn is_available(&self, provider: ProviderKind) -> bool;

    fn resolved_executable(&self, provider: ProviderKind) -> Option<ResolvedProviderExecutable>;
}

pub trait ProviderExecutableProbeGateway: Send + Sync {
    fn resolve(&self, executable: &ProviderExecutable) -> ProviderAvailability;
    fn refresh_search_path(&self) -> Result<(), ProviderExecutableProbeGatewayError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderExecutableProbeGatewayError {
    Technical(crate::domain::failure::TechnicalFailure),
}
