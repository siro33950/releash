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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSessionItemDto {
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionQueryError {
    Store(crate::domain::failure::FailureKind),
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

impl From<crate::domain::local_event::LocalEventQueryError> for AgentSessionQueryError {
    fn from(error: crate::domain::local_event::LocalEventQueryError) -> Self {
        use crate::domain::failure::ClassifiedFailure;
        match error.failure_kind() {
            crate::domain::failure::FailureKind::Temporary => Self::Unavailable,
            crate::domain::failure::FailureKind::Corrupt => Self::Corrupt,
            crate::domain::failure::FailureKind::InvalidInput => Self::InvalidRequest,
            kind => Self::Store(kind),
        }
    }
}

impl crate::domain::failure::ClassifiedFailure for AgentSessionQueryError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        use crate::domain::failure::FailureKind;
        match self {
            Self::Store(kind) => *kind,
            Self::InvalidRequest => FailureKind::InvalidInput,
            Self::Unavailable => FailureKind::Temporary,
            Self::Corrupt => FailureKind::Corrupt,
        }
    }
}

#[cfg(test)]
#[path = "agent_session_query_test.rs"]
mod agent_session_query_tests;
