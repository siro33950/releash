use crate::domain::provider_lifecycle::ProviderKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentSessionHistoryMetadata {
    pub(crate) provider: ProviderKind,
    pub(crate) provider_session_id: String,
    pub(crate) worktree_path: String,
    pub(crate) updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderSessionTitleEntry {
    pub(crate) provider_session_id: String,
    pub(crate) session_title: Option<String>,
    pub(crate) first_user_prompt: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionHistoryGatewayError {
    InvalidRequest,
    Unavailable,
    Corrupt,
}

#[async_trait::async_trait]
pub(crate) trait AgentSessionHistoryGateway: Send + Sync {
    async fn list_metadata(
        &self,
        provider: ProviderKind,
        worktree_path: &str,
        limit: usize,
    ) -> Result<Vec<AgentSessionHistoryMetadata>, AgentSessionHistoryGatewayError>;

    async fn list_session_titles(
        &self,
        provider: ProviderKind,
        worktree_path: &str,
        provider_session_ids: &[String],
    ) -> Result<Vec<ProviderSessionTitleEntry>, AgentSessionHistoryGatewayError>;
}

#[async_trait::async_trait]
pub(crate) trait AgentSessionOwnershipQuery: Send + Sync {
    async fn is_owned(
        &self,
        provider: ProviderKind,
        provider_session_id: &str,
    ) -> Result<bool, AgentSessionHistoryGatewayError>;
}
