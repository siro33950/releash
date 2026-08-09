pub(crate) mod backend_registry;
pub(crate) mod context;
pub(crate) mod context_meta;
pub(crate) mod event_log;
pub(crate) mod feedback;
pub(crate) mod notice;
pub(crate) mod notice_query_service;
pub(crate) mod notice_state;
pub(crate) mod operation;
mod provider_agent_session;
mod provider_agent_session_activity;
mod provider_agent_session_exit;
mod provider_agent_session_history;
mod provider_agent_session_initial_instruction;
mod provider_agent_session_launch;
mod provider_agent_session_lifecycle;
mod provider_agent_session_query;
mod provider_agent_session_read;
mod provider_availability;
pub(crate) mod runtime;
pub(crate) mod session;
pub(crate) mod session_feedback_load;
pub(crate) mod status;
pub(crate) mod system_prompt;
pub(crate) mod workspace_session_creation;

pub(crate) use provider_agent_session::{
    ProviderAgentSessionCreateRequest, ProviderAgentSessionUsecase,
    ProviderAgentSessionUsecaseError,
};
pub(crate) use provider_agent_session_activity::{
    ProviderAgentSessionActivityUsecase, ProviderAgentSessionChangeNotifier,
};
#[cfg(test)]
pub(crate) use provider_agent_session_exit::ProviderAgentSessionExitPort;
pub(crate) use provider_agent_session_exit::ProviderAgentSessionExitUsecase;
pub(crate) use provider_agent_session_history::{
    ProviderAgentSessionHistoryCandidateDto, ProviderAgentSessionHistoryPageDto,
    ProviderAgentSessionHistoryQueryError, ProviderAgentSessionHistoryQueryService,
    ProviderAgentSessionHistoryReadUsecase, ProviderAgentSessionHistoryRequest,
};
pub(crate) use provider_agent_session_initial_instruction::ProviderAgentInitialInstructionUsecase;
#[cfg(test)]
pub(crate) use provider_agent_session_launch::ProviderAgentSessionHistoryResumeOutcome;
pub(crate) use provider_agent_session_launch::ProviderAgentSessionLaunchUsecaseError;
pub(crate) use provider_agent_session_launch::{
    ProviderAgentSessionHistoryResumeRequest, ProviderAgentSessionLaunchRequest,
    ProviderAgentSessionLaunchUsecase, ProviderAgentWorkflowSessionLaunchRequest,
};
pub(crate) use provider_agent_session_lifecycle::{
    ProviderAgentSessionGarbageCollectionOutcome, ProviderAgentSessionLifecycleUsecase,
    ProviderAgentSessionLifecycleUsecaseError, ProviderAgentSessionOpenOutcome,
};
pub(crate) use provider_agent_session_query::{
    ProviderAgentSessionActivityDto, ProviderAgentSessionItemDto, ProviderAgentSessionLifecycleDto,
    ProviderAgentSessionListPageDto, ProviderAgentSessionListRequest,
    ProviderAgentSessionOperationsDto, ProviderAgentSessionOriginDto,
    ProviderAgentSessionOriginFilter, ProviderAgentSessionProviderDto,
    ProviderAgentSessionQueryError, ProviderAgentSessionQueryService,
};
#[cfg(test)]
pub(crate) use provider_agent_session_read::ProviderAgentSessionGarbageCollectionPort;
pub(crate) use provider_agent_session_read::{
    ProviderAgentSessionReadUsecase, ProviderAgentSessionReadUsecaseError,
};
pub(crate) use provider_availability::{
    ProviderAvailabilityQueryService, ProviderAvailabilityReadUsecase,
};

#[cfg(test)]
#[path = "provider_agent_session_exit_test.rs"]
mod provider_agent_session_exit_tests;
#[cfg(test)]
#[path = "provider_agent_session_history_test.rs"]
mod provider_agent_session_history_tests;
#[cfg(test)]
#[path = "provider_agent_session_initial_instruction_test.rs"]
mod provider_agent_session_initial_instruction_tests;
#[cfg(test)]
#[path = "provider_agent_session_lifecycle_test.rs"]
mod provider_agent_session_lifecycle_tests;
#[cfg(test)]
#[path = "provider_agent_session_read_test.rs"]
mod provider_agent_session_read_tests;
#[cfg(test)]
#[path = "provider_agent_session_test.rs"]
mod provider_agent_session_tests;
#[cfg(test)]
#[path = "provider_availability_test.rs"]
mod provider_availability_tests;

#[cfg(test)]
mod issue_1499_contract_tests;
