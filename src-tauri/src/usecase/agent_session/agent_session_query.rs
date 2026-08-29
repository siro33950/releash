use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AgentSessionProviderDto {
    Claude,
    Codex,
}

/// AgentSession が属する実行木と NodeExecution の所在。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSessionTreeLocationDto {
    pub tree_id: String,
    pub node_execution_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AgentSessionLifecycleDto {
    Open,
    Paused,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSessionOperationsDto {
    pub can_archive: bool,
    pub can_restore: bool,
    pub can_delete: bool,
    pub can_resume: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSessionItemDto {
    pub id: String,
    pub workspace_identity: String,
    pub worktree_path: String,
    pub provider: AgentSessionProviderDto,
    pub tree_location: AgentSessionTreeLocationDto,
    pub lifecycle: AgentSessionLifecycleDto,
    pub provider_session_id: Option<String>,
    pub transcript_ref: Option<String>,
    pub operations: AgentSessionOperationsDto,
    pub last_exit_abnormal: bool,
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
}
