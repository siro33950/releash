use serde::Serialize;

use crate::domain::workspace_tree::WorkspaceIdentity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProviderAgentSessionProviderDto {
    Claude,
    Codex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ProviderAgentSessionOriginDto {
    Standalone,
    WorkflowNode {
        workflow_execution_id: String,
        node_execution_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProviderAgentSessionLifecycleDto {
    Open,
    Paused,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProviderAgentSessionActivityDto {
    Running,
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderAgentSessionOriginFilter {
    Standalone,
    WorkflowNode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderAgentSessionOperationsDto {
    pub can_archive: bool,
    pub can_restore: bool,
    pub can_delete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderAgentSessionItemDto {
    pub id: String,
    pub workspace_identity: String,
    pub worktree_path: String,
    pub provider: ProviderAgentSessionProviderDto,
    pub origin: ProviderAgentSessionOriginDto,
    pub lifecycle: ProviderAgentSessionLifecycleDto,
    pub provider_session_id: Option<String>,
    pub transcript_ref: Option<String>,
    pub operations: ProviderAgentSessionOperationsDto,
    pub activity: ProviderAgentSessionActivityDto,
    pub last_exit_abnormal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderAgentSessionListPageDto {
    pub items: Vec<ProviderAgentSessionItemDto>,
    pub next_after_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderAgentSessionListRequest {
    pub workspace: WorkspaceIdentity,
    pub lifecycle: Option<ProviderAgentSessionLifecycleDto>,
    pub origin: Option<ProviderAgentSessionOriginFilter>,
    pub limit: usize,
    pub after_session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderAgentSessionQueryError {
    InvalidRequest,
    Unavailable,
    Corrupt,
}

#[async_trait::async_trait]
pub(crate) trait ProviderAgentSessionQueryService: Send + Sync {
    async fn get(
        &self,
        agent_session_id: &str,
    ) -> Result<Option<ProviderAgentSessionItemDto>, ProviderAgentSessionQueryError>;

    async fn list(
        &self,
        request: ProviderAgentSessionListRequest,
    ) -> Result<ProviderAgentSessionListPageDto, ProviderAgentSessionQueryError>;
}
