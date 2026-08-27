mod agent_session;
#[cfg(test)]
#[path = "agent_session_test.rs"]
mod agent_session_tests;
mod provider_registry;
#[cfg(test)]
#[path = "provider_registry_test.rs"]
mod provider_registry_tests;

pub(crate) use agent_session::{
    derive_agent_session_operations, AgentSession, AgentSessionArchiveOutcome,
    AgentSessionInitialInstructionOutcome, AgentSessionLifecycle, AgentSessionLifecycleEvent,
    AgentSessionMutationOutcome, AgentSessionOpenAction, AgentSessionOperations,
    AgentSessionProcessExitOutcome, AgentSessionRecoveryResult, AgentSessionRemovalAuthorization,
    AgentSessionTreeLocation, AgentSessionTreeLocationError, ManagedPtyPresence,
};
#[cfg(test)]
pub(crate) use provider_registry::ProviderRegistryError;
pub(crate) use provider_registry::{
    ProviderAvailability, ProviderExecutable, ProviderRegistry, ProviderRegistryEntry,
    ProviderUnavailableReason, ResolvedProviderExecutable,
};
