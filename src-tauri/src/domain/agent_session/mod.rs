pub mod aggregates;
mod provider_availability_gateway;
mod provider_history_gateway;
mod provider_launch;
mod provider_launch_gateway;
mod provider_session_ownership;
mod provider_terminal_gateway;
pub mod repository;

pub(crate) use provider_availability_gateway::{
    ProviderAvailabilityReader, ProviderExecutableConfigRepository,
    ProviderExecutableConfigRepositoryError, ProviderExecutableProbeGateway,
    ProviderExecutableProbeGatewayError,
};
pub(crate) use provider_history_gateway::{
    AgentSessionHistoryGateway, AgentSessionHistoryGatewayError, AgentSessionHistoryMetadata,
    AgentSessionOwnershipQuery,
};
pub(crate) use provider_launch::ProviderSessionLaunch;
#[cfg(test)]
pub(crate) use provider_launch::ProviderSessionLaunchError;
pub(crate) use provider_launch_gateway::{
    PreparedProviderLaunch, ProviderAgentLaunchGateway, ProviderAgentLaunchGatewayError,
};
pub(crate) use provider_session_ownership::{
    ProviderSessionOwnership, ProviderSessionOwnershipEvent,
};
pub(crate) use provider_terminal_gateway::{
    ProviderAgentTerminalGateway, ProviderAgentTerminalGatewayError,
    ProviderAgentTerminalInputGateway, ProviderAgentTerminalObservationGateway,
};
