pub(crate) mod agent_session_history_gateway;
pub(crate) mod agent_session_history_query_service;
pub(crate) mod agent_session_query_service;
pub(crate) mod agent_session_repository;
pub(crate) mod provider_agent_launch_gateway;
mod provider_agent_terminal_gateway;
pub(crate) mod provider_availability_gateway;
#[cfg(any(test, feature = "test-support"))]
mod provider_executable_config_repository;
pub(crate) mod session_facts;
pub(crate) use agent_session_history_gateway::LocalAgentSessionHistoryGateway;
pub(crate) use agent_session_history_query_service::LocalAgentSessionHistoryQueryService;
pub(crate) use agent_session_query_service::LocalAgentSessionQueryService;
pub(crate) use agent_session_repository::{agent_session_from_fields, LocalAgentSessionRepository};
pub(crate) use provider_agent_launch_gateway::LocalProviderAgentLaunchGateway;
pub(crate) use provider_availability_gateway::LocalProviderExecutableProbeGateway;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use provider_executable_config_repository::InMemoryProviderExecutableConfigRepository;
pub(crate) use session_facts::{read_session_context, SessionContextReadError, SessionLocation};

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_helpers;
