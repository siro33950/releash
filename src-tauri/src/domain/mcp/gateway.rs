use async_trait::async_trait;

use crate::domain::mcp::error::McpError;
use crate::domain::mcp::value_objects::{AgentKind, McpConfigParams, McpConnectionInfo};

#[async_trait]
pub trait McpServerGateway: Send + Sync {
    type Context: Send + Sync;

    async fn start(&self, context: &Self::Context) -> Result<McpConnectionInfo, McpError>;
    async fn stop(&self, context: &Self::Context) -> Result<(), McpError>;
    async fn restart_if_running(
        &self,
        context: &Self::Context,
    ) -> Result<McpConnectionInfo, McpError>;
    fn is_running(&self, context: &Self::Context) -> Result<bool, McpError>;
    fn connection_info(
        &self,
        context: &Self::Context,
    ) -> Result<Option<McpConnectionInfo>, McpError>;
}

pub trait AgentConfigGateway: Send + Sync {
    fn configured_agents(&self) -> Result<Vec<String>, McpError>;
    fn remove(&self, agent: AgentKind) -> Result<bool, McpError>;
    fn generate(
        &self,
        agent: AgentKind,
        params: &McpConfigParams,
    ) -> Result<AgentConfigGenerateResult, McpError>;
    fn preview(&self, agent: AgentKind, params: &McpConfigParams) -> Result<String, McpError>;
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentConfigGenerateResult {
    pub agent: String,
    pub file_path: String,
    pub content: String,
}
