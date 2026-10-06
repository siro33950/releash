mod agent_session_exit;
pub(crate) mod agent_session_history;
pub(crate) mod agent_session_initial_instruction;
pub(crate) mod agent_session_launch;
pub(crate) mod agent_session_lifecycle;
pub(crate) mod agent_session_query;
pub(crate) mod agent_session_read;
mod agent_session_rename;
pub(crate) mod provider_availability;
mod provider_session_title_ingestion;
pub(crate) mod usecase;

#[cfg(test)]
pub(crate) use agent_session_exit::AgentSessionExitPort;
pub(crate) use agent_session_exit::AgentSessionExitUsecase;
pub(crate) use agent_session_history::{
    AgentSessionHistoryCandidateDto, AgentSessionHistoryPageDto, AgentSessionHistoryQueryError,
    AgentSessionHistoryQueryService, AgentSessionHistoryReadUsecase, AgentSessionHistoryRequest,
};
pub(crate) use agent_session_initial_instruction::AgentSessionInitialInstructionUsecase;
pub(crate) use agent_session_launch::{
    AgentSessionExecutionTreeLifecycle, AgentSessionHistoryResumeRequest,
    AgentSessionLaunchRequest, AgentSessionLaunchUsecase, ExecutionTreeCache,
    ExecutionTreeCacheReleaseError, ProviderAgentRuntime, StartedExecutionTreeRegistrar,
    StartedExecutionTreeRegistrationError, WorkflowAgentSessionLaunchRequest,
    WorktreeMutationAdmission,
};
pub(crate) use agent_session_launch::{AgentSessionLaunchUsecaseError, LaunchRetention};
pub(crate) use agent_session_lifecycle::{
    AgentSessionGarbageCollectionOutcome, AgentSessionLifecycleUsecase,
    AgentSessionLifecycleUsecaseError, AgentSessionOpenOutcome,
};
pub(crate) use agent_session_query::{
    AgentSessionItemDto, AgentSessionLifecycleDto, AgentSessionOperationsDto,
    AgentSessionQueryError, AgentSessionQueryService, AgentSessionTreeLocationDto,
};
pub(crate) use agent_session_read::AgentSessionReadUsecase;
pub(crate) use agent_session_rename::{
    AgentSessionRenameError, AgentSessionRenameExecutor, AgentSessionRenameUsecase,
};
#[cfg(test)]
pub(crate) use provider_availability::ProviderAvailabilityItemDto;
pub(crate) use provider_availability::{
    ProviderAvailabilitySnapshotDto, ProviderAvailabilityUsecase, ProviderAvailabilityUsecaseError,
    ProviderUnavailableReasonDto,
};
pub(crate) use provider_session_title_ingestion::ProviderSessionTitleIngestionUsecase;
pub(crate) use usecase::{
    AgentSessionCreateRequest, AgentSessionUsecase, AgentSessionUsecaseError,
};

#[cfg(test)]
#[path = "agent_session_exit_test.rs"]
mod agent_session_exit_tests;
#[cfg(test)]
#[path = "agent_session_history_test.rs"]
mod agent_session_history_tests;

#[cfg(test)]
#[path = "agent_session_read_test.rs"]
mod agent_session_read_tests;
#[cfg(test)]
#[path = "agent_session_rename_test.rs"]
mod agent_session_rename_tests;

#[cfg(test)]
#[path = "provider_availability_test.rs"]
pub(crate) mod provider_availability_tests;
#[cfg(test)]
#[path = "provider_session_title_ingestion_test.rs"]
pub(crate) mod provider_session_title_ingestion_tests;

pub(crate) use agent_session_read::AgentSessionReadUsecaseError;

pub(crate) use agent_session_initial_instruction::AgentSessionInitialInstructionError;

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_helpers_provider_availability;
