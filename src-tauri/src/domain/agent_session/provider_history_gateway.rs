use crate::domain::provider_lifecycle::ProviderKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderAgentSessionHistoryMetadata {
    pub(crate) provider: ProviderKind,
    pub(crate) provider_session_id: String,
    pub(crate) worktree_path: String,
    pub(crate) updated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderAgentSessionHistoryGatewayError {
    InvalidRequest,
    Unavailable,
    Corrupt,
}

#[async_trait::async_trait]
pub(crate) trait ProviderAgentSessionHistoryGateway: Send + Sync {
    async fn list_metadata(
        &self,
        provider: ProviderKind,
        worktree_path: &str,
        limit: usize,
    ) -> Result<Vec<ProviderAgentSessionHistoryMetadata>, ProviderAgentSessionHistoryGatewayError>;
}

#[async_trait::async_trait]
pub(crate) trait ProviderAgentSessionOwnershipQuery: Send + Sync {
    async fn is_owned(
        &self,
        provider: ProviderKind,
        provider_session_id: &str,
    ) -> Result<bool, ProviderAgentSessionHistoryGatewayError>;
}
