mod agent_session_activity;
mod agent_session_exit;
mod agent_session_history;
mod agent_session_initial_instruction;
mod agent_session_interrupt;
mod agent_session_launch;
mod agent_session_lifecycle;
mod agent_session_query;
mod agent_session_read;
mod provider_availability;
mod usecase;

pub(crate) use agent_session_activity::{AgentSessionActivityUsecase, AgentSessionChangeNotifier};
#[cfg(test)]
pub(crate) use agent_session_exit::AgentSessionExitPort;
pub(crate) use agent_session_exit::AgentSessionExitUsecase;
pub(crate) use agent_session_history::{
    AgentSessionHistoryCandidateDto, AgentSessionHistoryPageDto, AgentSessionHistoryQueryError,
    AgentSessionHistoryQueryService, AgentSessionHistoryReadUsecase, AgentSessionHistoryRequest,
};
pub(crate) use agent_session_initial_instruction::AgentSessionInitialInstructionUsecase;
pub(crate) use agent_session_interrupt::AgentSessionInterruptUsecase;
pub(crate) use agent_session_launch::AgentSessionLaunchUsecaseError;
pub(crate) use agent_session_launch::{
    AgentSessionHistoryResumeOutcome, AgentSessionHistoryResumeRequest, AgentSessionLaunchRequest,
    AgentSessionLaunchUsecase, ExecutionTreeCacheReleaseError, ProviderAgentRuntime,
    StartedExecutionTreeRegistrar, StartedExecutionTreeRegistrationError,
    WorkflowAgentSessionLaunchRequest,
};
pub(crate) use agent_session_lifecycle::{
    AgentSessionGarbageCollectionOutcome, AgentSessionLifecycleUsecase,
    AgentSessionLifecycleUsecaseError, AgentSessionOpenOutcome,
};
pub(crate) use agent_session_query::{
    AgentSessionActivityDto, AgentSessionItemDto, AgentSessionLifecycleDto,
    AgentSessionOperationsDto, AgentSessionProviderDto, AgentSessionQueryError,
    AgentSessionQueryService, AgentSessionTreeLocationDto,
};
#[cfg(test)]
pub(crate) use agent_session_read::AgentSessionGarbageCollectionPort;
pub(crate) use agent_session_read::{AgentSessionReadUsecase, AgentSessionReadUsecaseError};
pub(crate) use provider_availability::{
    ProviderAvailabilityUsecase, ProviderAvailabilityUsecaseError,
};
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
#[path = "agent_session_test.rs"]
mod agent_session_tests;
#[cfg(test)]
#[path = "provider_availability_test.rs"]
mod provider_availability_tests;
