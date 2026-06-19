use crate::domain::mcp::gateway::AgentConfigGenerateResult;
use crate::domain::mcp::value_objects::{McpConnectionInfo, McpServerStatus};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct GenerateResult {
    pub agent: String,
    pub file_path: String,
    pub content: String,
}

impl From<AgentConfigGenerateResult> for GenerateResult {
    fn from(result: AgentConfigGenerateResult) -> Self {
        Self {
            agent: result.agent,
            file_path: result.file_path,
            content: result.content,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct McpConnectionInfoDto {
    pub url: String,
    pub token: String,
}

impl From<McpConnectionInfo> for McpConnectionInfoDto {
    fn from(info: McpConnectionInfo) -> Self {
        Self {
            url: info.url,
            token: info.token,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct McpServerStatusDto {
    pub running: bool,
    pub port: Option<u16>,
}

impl From<McpServerStatus> for McpServerStatusDto {
    fn from(status: McpServerStatus) -> Self {
        Self {
            running: status.running,
            port: status.port,
        }
    }
}
