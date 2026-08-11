pub(crate) mod claude;
pub(crate) mod codex;
#[cfg(test)]
pub(crate) mod fixtures;
pub(crate) mod instruction_source;
pub(crate) mod lifecycle_repository;
pub(crate) mod operation;
pub(crate) mod prompt_suggestion;
mod provider_agent_launch_gateway;
mod provider_agent_session_history_gateway;
mod provider_agent_session_history_query_service;
mod provider_agent_session_query_service;
mod provider_agent_session_repository;
mod provider_agent_terminal_gateway;
mod provider_availability_gateway;
mod provider_executable_config_repository;
pub(crate) mod runtime_driver;
pub(crate) mod runtime_projection;
pub(crate) mod session_storage;
mod state_serde;

pub(crate) use instruction_source::FileSystemInstructionSourceGateway;
pub(crate) use lifecycle_repository::LocalAgentSessionLifecycleRepository;
pub(crate) use prompt_suggestion::GitAgentPromptSuggestionGateway;
pub(crate) use provider_agent_launch_gateway::LocalProviderAgentLaunchGateway;
pub(crate) use provider_agent_session_history_gateway::LocalProviderAgentSessionHistoryGateway;
pub(crate) use provider_agent_session_history_query_service::LocalProviderAgentSessionHistoryQueryService;
pub(crate) use provider_agent_session_query_service::LocalProviderAgentSessionQueryService;
pub(crate) use provider_agent_session_repository::LocalProviderAgentSessionRepository;
pub(crate) use provider_availability_gateway::LocalProviderExecutableProbeGateway;
pub(crate) use provider_executable_config_repository::InMemoryProviderExecutableConfigRepository;
pub(crate) use runtime_driver::{TokioAgentTaskSpawner, WorkflowRuntimeAgentSessionNotifier};

#[cfg(test)]
#[path = "provider_agent_launch_gateway_test.rs"]
mod provider_agent_launch_gateway_tests;
#[cfg(test)]
#[path = "provider_agent_session_history_gateway_test.rs"]
mod provider_agent_session_history_gateway_tests;
#[cfg(test)]
#[path = "provider_agent_session_history_query_service_test.rs"]
mod provider_agent_session_history_query_service_tests;
#[cfg(test)]
#[path = "provider_agent_session_repository_test.rs"]
mod provider_agent_session_repository_tests;
#[cfg(test)]
#[path = "provider_availability_gateway_test.rs"]
mod provider_availability_gateway_tests;
#[cfg(test)]
pub(crate) use session_storage::FileSessionStorage;
