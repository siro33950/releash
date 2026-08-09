use std::sync::Arc;

use serde::Serialize;

use super::ProviderAgentSessionProviderDto;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderAgentSessionHistoryCandidateDto {
    pub(crate) provider: ProviderAgentSessionProviderDto,
    pub(crate) provider_session_id: String,
    pub(crate) updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderAgentSessionHistoryPageDto {
    pub(crate) items: Vec<ProviderAgentSessionHistoryCandidateDto>,
    pub(crate) next_after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderAgentSessionHistoryRequest {
    pub(crate) worktree_path: String,
    pub(crate) limit: usize,
    pub(crate) after: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderAgentSessionHistoryQueryError {
    InvalidRequest,
    Unavailable,
    Corrupt,
}

#[async_trait::async_trait]
pub(crate) trait ProviderAgentSessionHistoryQueryService: Send + Sync {
    async fn list(
        &self,
        request: ProviderAgentSessionHistoryRequest,
    ) -> Result<ProviderAgentSessionHistoryPageDto, ProviderAgentSessionHistoryQueryError>;
}

pub(crate) struct ProviderAgentSessionHistoryReadUsecase {
    query: Arc<dyn ProviderAgentSessionHistoryQueryService>,
}

impl ProviderAgentSessionHistoryReadUsecase {
    pub(crate) fn new(query: Arc<dyn ProviderAgentSessionHistoryQueryService>) -> Self {
        Self { query }
    }

    pub(crate) async fn list(
        &self,
        request: ProviderAgentSessionHistoryRequest,
    ) -> Result<ProviderAgentSessionHistoryPageDto, ProviderAgentSessionHistoryQueryError> {
        self.query.list(request).await
    }
}
