mod agent_session_history_gateway;
mod agent_session_history_query_service;
mod agent_session_query_service;
mod agent_session_repository;
mod provider_agent_launch_gateway;
mod provider_agent_terminal_gateway;
mod provider_availability_gateway;
mod provider_executable_config_repository;
mod session_facts;
pub(crate) use agent_session_history_gateway::LocalAgentSessionHistoryGateway;
pub(crate) use agent_session_history_query_service::LocalAgentSessionHistoryQueryService;
pub(crate) use agent_session_query_service::LocalAgentSessionQueryService;
pub(crate) use agent_session_repository::{agent_session_from_fields, LocalAgentSessionRepository};
pub(crate) use provider_agent_launch_gateway::LocalProviderAgentLaunchGateway;
pub(crate) use provider_availability_gateway::LocalProviderExecutableProbeGateway;
pub(crate) use provider_executable_config_repository::InMemoryProviderExecutableConfigRepository;
pub(crate) use session_facts::{read_session_context, SessionContextReadError, SessionLocation};

#[cfg(test)]
#[path = "agent_session_history_gateway_test.rs"]
mod agent_session_history_gateway_tests;
#[cfg(test)]
#[path = "agent_session_history_query_service_test.rs"]
mod agent_session_history_query_service_tests;
#[cfg(test)]
#[path = "agent_session_repository_test.rs"]
mod agent_session_repository_tests;

#[cfg(test)]
#[path = "provider_availability_gateway_test.rs"]
mod provider_availability_gateway_tests;
