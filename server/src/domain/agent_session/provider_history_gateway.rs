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

impl ProviderSessionTitleEntry {
    /// この履歴が検索語に当たるかを決める。
    /// 対象は Session の題と最初の指示であり、どちらかが検索語を含めば当たる。
    pub(crate) fn matches(&self, term: &crate::domain::search_term::SearchTerm) -> bool {
        self.session_title
            .as_deref()
            .is_some_and(|title| term.matches(title))
            || self
                .first_user_prompt
                .as_deref()
                .is_some_and(|prompt| term.matches(prompt))
    }
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

#[cfg(test)]
#[path = "provider_history_gateway_test.rs"]
mod provider_history_gateway_tests;
