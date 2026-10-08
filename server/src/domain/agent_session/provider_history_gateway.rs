use crate::domain::provider_lifecycle::ProviderKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionHistoryMetadata {
    pub provider: ProviderKind,
    pub provider_session_id: String,
    pub worktree_path: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSessionTitleEntry {
    pub provider_session_id: String,
    pub session_title: Option<String>,
    pub first_user_prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSessionHistoryGatewayError {
    Conflict,
    ProviderSessionAlreadyOwned { agent_session_id: String },
    Store(crate::domain::failure::StorageFailure),
    InvalidRequest,
    Technical(crate::domain::failure::TechnicalFailure),
    Corrupt,
}

#[async_trait::async_trait]
pub trait AgentSessionHistoryGateway: Send + Sync {
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
pub trait AgentSessionOwnershipQuery: Send + Sync {
    async fn is_owned(
        &self,
        provider: ProviderKind,
        provider_session_id: &str,
    ) -> Result<bool, AgentSessionHistoryGatewayError>;
}
