mod agent_session_change_notifier;
mod agent_session_exit;
mod agent_session_history;
mod agent_session_initial_instruction;
mod agent_session_launch;
mod agent_session_lifecycle;
mod agent_session_query;
mod agent_session_read;
mod agent_session_rename;
mod provider_availability;
mod provider_session_title_ingestion;
mod usecase;

pub(crate) use agent_session_change_notifier::AgentSessionChangeNotifier;
#[cfg(test)]
pub(crate) use agent_session_exit::AgentSessionExitPort;
pub(crate) use agent_session_exit::AgentSessionExitUsecase;
pub(crate) use agent_session_history::{
    AgentSessionHistoryCandidateDto, AgentSessionHistoryPageDto, AgentSessionHistoryQueryError,
    AgentSessionHistoryQueryService, AgentSessionHistoryReadUsecase, AgentSessionHistoryRequest,
};
pub(crate) use agent_session_initial_instruction::AgentSessionInitialInstructionUsecase;
#[cfg(test)]
pub(crate) use agent_session_launch::AgentSessionLaunchExecutionTrees;
pub(crate) use agent_session_launch::AgentSessionLaunchUsecaseError;
pub(crate) use agent_session_launch::{
    AgentSessionExecutionTreeLifecycle, AgentSessionHistoryResumeOutcome,
    AgentSessionHistoryResumeRequest, AgentSessionLaunchRequest, AgentSessionLaunchUsecase,
    ExecutionTreeCache, ExecutionTreeCacheReleaseError, ProviderAgentRuntime,
    StartedExecutionTreeRegistrar, StartedExecutionTreeRegistrationError,
    WorkflowAgentSessionLaunchRequest, WorktreeMutationAdmission,
};
pub(crate) use agent_session_lifecycle::{
    AgentSessionGarbageCollectionOutcome, AgentSessionLifecycleUsecase,
    AgentSessionLifecycleUsecaseError, AgentSessionOpenOutcome,
};
pub(crate) use agent_session_query::{
    AgentSessionItemDto, AgentSessionLifecycleDto, AgentSessionOperationsDto,
    AgentSessionProviderDto, AgentSessionQueryError, AgentSessionQueryService,
    AgentSessionTreeLocationDto,
};
#[cfg(test)]
pub(crate) use agent_session_read::AgentSessionGarbageCollectionPort;
pub(crate) use agent_session_read::{AgentSessionReadUsecase, AgentSessionReadUsecaseError};
pub(crate) use agent_session_rename::{
    AgentSessionRenameError, AgentSessionRenameExecutor, AgentSessionRenameUsecase,
};
pub(crate) use provider_availability::{
    ProviderAvailabilityUsecase, ProviderAvailabilityUsecaseError,
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
#[path = "agent_session_initial_instruction_test.rs"]
mod agent_session_initial_instruction_tests;
#[cfg(test)]
#[path = "agent_session_lifecycle_test.rs"]
mod agent_session_lifecycle_tests;
#[cfg(test)]
#[path = "agent_session_read_test.rs"]
mod agent_session_read_tests;
#[cfg(test)]
#[path = "agent_session_rename_test.rs"]
mod agent_session_rename_tests;
#[cfg(test)]
#[path = "agent_session_test.rs"]
mod agent_session_tests;
#[cfg(test)]
#[path = "provider_availability_test.rs"]
pub(crate) mod provider_availability_tests;
#[cfg(test)]
#[path = "provider_session_title_ingestion_test.rs"]
mod provider_session_title_ingestion_tests;
