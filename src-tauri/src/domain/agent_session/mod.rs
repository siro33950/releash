pub mod aggregates;
pub mod entities;
pub mod events;
pub mod gateway;
mod provider_availability_gateway;
mod provider_history_gateway;
mod provider_launch;
mod provider_launch_gateway;
mod provider_terminal_gateway;
pub mod repository;
pub(crate) mod services;
pub(crate) mod storage;
pub(crate) mod value_objects;

pub(crate) use provider_availability_gateway::{
    ProviderAvailabilityReader, ProviderExecutableConfigRepository,
    ProviderExecutableConfigRepositoryError, ProviderExecutableProbeGateway,
    ProviderExecutableProbeGatewayError,
};
pub(crate) use provider_history_gateway::{
    ProviderAgentSessionHistoryGateway, ProviderAgentSessionHistoryGatewayError,
    ProviderAgentSessionHistoryMetadata, ProviderAgentSessionOwnershipQuery,
};
pub(crate) use provider_launch::ProviderSessionLaunch;
#[cfg(test)]
pub(crate) use provider_launch::ProviderSessionLaunchError;
pub(crate) use provider_launch_gateway::{
    PreparedProviderLaunch, ProviderAgentLaunchGateway, ProviderAgentLaunchGatewayError,
};
pub(crate) use provider_terminal_gateway::{
    ProviderAgentTerminalGateway, ProviderAgentTerminalGatewayError,
    ProviderAgentTerminalInputGateway, ProviderAgentTerminalObservationGateway,
};
pub(crate) use services::{
    dedup_instructions, latest_revisions_by_kind, next_epoch_for_identity,
    normalize_path_components, replacement_action, snapshot_is_stale,
};
#[cfg(test)]
pub(crate) use storage::AgentSessionProjectionPreparer;
pub(crate) use storage::{
    AgentSessionProjectedMessage, AgentSessionProjectionCommit, AgentSessionReader,
    AgentSessionStorageTypes,
};
#[cfg(test)]
pub(crate) use storage::{AgentSessionStorage, AgentSessionWriter};
pub(crate) use value_objects::{
    ContextEpoch, ContextEpochId, ContextEpochIdentity, ContextRevision, ContextSnapshot,
    ContextSourceKind, ContextSourceState, InstructionOrigin, InvalidPermissionMode,
    PermissionMode, ReplacementAction, ReplacementTrigger, ResolvedInstruction, SkillEntry,
};
