use std::sync::Arc;

use crate::domain::mcp::gateway::AgentConfigGateway;
use crate::domain::mcp::value_objects::McpConfigParams;
use crate::usecase::mcp::agent_config_usecase::McpAgentConfigUsecase;

pub struct McpQueryService<G: AgentConfigGateway> {
    agent_config: McpAgentConfigUsecase<G>,
}

impl<G: AgentConfigGateway> McpQueryService<G> {
    pub fn new(gateway: Arc<G>) -> Self {
        Self {
            agent_config: McpAgentConfigUsecase::new(gateway),
        }
    }

    pub fn configured_agents(&self) -> Result<Vec<String>, String> {
        self.agent_config.configured_agents()
    }

    pub fn preview(&self, agent_type: String, params: &McpConfigParams) -> Result<String, String> {
        self.agent_config.preview(agent_type, params)
    }
}
