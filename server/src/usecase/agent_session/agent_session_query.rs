use serde::Serialize;

use crate::usecase::provider_dto::AgentSessionProviderDto;

/// AgentSession が属する実行木と NodeExecution の所在。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionTreeLocationDto {
    pub tree_id: String,
    pub node_execution_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentSessionLifecycleDto {
    Open,
    Paused,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionOperationsDto {
    pub can_archive: bool,
    pub can_restore: bool,
    pub can_delete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionItemDto {
    pub id: String,
    pub workspace_identity: String,
    pub worktree_path: String,
    pub workspace_worktree_path: String,
    pub provider: AgentSessionProviderDto,
    pub tree_location: AgentSessionTreeLocationDto,
    pub lifecycle: AgentSessionLifecycleDto,
    pub provider_session_id: Option<String>,
    pub transcript_ref: Option<String>,
    pub operations: AgentSessionOperationsDto,
    pub last_exit_abnormal: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_presence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSessionQueryError {
    Store(crate::domain::failure::StorageFailure),
    InvalidRequest,
    Unavailable,
    Corrupt,
}

#[async_trait::async_trait]
pub trait AgentSessionQueryService: Send + Sync {
    async fn get(
        &self,
        agent_session_id: &str,
    ) -> Result<Option<AgentSessionItemDto>, AgentSessionQueryError>;
}
