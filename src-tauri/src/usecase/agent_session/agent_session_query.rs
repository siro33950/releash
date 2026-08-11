use serde::Serialize;

use crate::domain::workspace_tree::WorkspaceIdentity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AgentSessionProviderDto {
    Claude,
    Codex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum AgentSessionOriginDto {
    Standalone,
    WorkflowNode {
        workflow_execution_id: String,
        node_execution_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AgentSessionLifecycleDto {
    Open,
    Paused,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AgentSessionActivityDto {
    Running,
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionOriginFilter {
    Standalone,
    WorkflowNode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSessionOperationsDto {
    pub can_archive: bool,
    pub can_restore: bool,
    pub can_delete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSessionItemDto {
    pub id: String,
    pub workspace_identity: String,
    pub worktree_path: String,
    pub provider: AgentSessionProviderDto,
    pub origin: AgentSessionOriginDto,
    pub lifecycle: AgentSessionLifecycleDto,
    pub provider_session_id: Option<String>,
    pub transcript_ref: Option<String>,
    pub operations: AgentSessionOperationsDto,
    pub activity: AgentSessionActivityDto,
    pub last_exit_abnormal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSessionListPageDto {
    pub items: Vec<AgentSessionItemDto>,
    pub next_after_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentSessionListRequest {
    pub workspace: WorkspaceIdentity,
    pub lifecycle: Option<AgentSessionLifecycleDto>,
    pub origin: Option<AgentSessionOriginFilter>,
    pub limit: usize,
    pub after_session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionQueryError {
    InvalidRequest,
    Unavailable,
    Corrupt,
}

#[async_trait::async_trait]
pub(crate) trait AgentSessionQueryService: Send + Sync {
    async fn get(
        &self,
        agent_session_id: &str,
    ) -> Result<Option<AgentSessionItemDto>, AgentSessionQueryError>;

    async fn list(
        &self,
        request: AgentSessionListRequest,
    ) -> Result<AgentSessionListPageDto, AgentSessionQueryError>;
}
