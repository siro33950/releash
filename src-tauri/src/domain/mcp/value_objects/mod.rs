mod agent_kind;

pub use agent_kind::AgentKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpConfigParams {
    pub port: u16,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpConnectionInfo {
    pub url: String,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerStatus {
    pub running: bool,
    pub port: Option<u16>,
}
