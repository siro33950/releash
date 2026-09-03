use std::sync::Arc;

use serde::Serialize;

use super::AgentSessionProviderDto;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSessionHistoryCandidateDto {
    pub(crate) provider: AgentSessionProviderDto,
    pub(crate) provider_session_id: String,
    pub(crate) label: String,
    pub(crate) updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSessionHistoryPageDto {
    pub(crate) items: Vec<AgentSessionHistoryCandidateDto>,
    pub(crate) next_after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentSessionHistoryRequest {
    pub(crate) worktree_path: String,
    pub(crate) limit: usize,
    pub(crate) after: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionHistoryQueryError {
    InvalidRequest,
    Unavailable,
    Corrupt,
}

#[async_trait::async_trait]
pub(crate) trait AgentSessionHistoryQueryService: Send + Sync {
    async fn list(
        &self,
        request: AgentSessionHistoryRequest,
    ) -> Result<AgentSessionHistoryPageDto, AgentSessionHistoryQueryError>;
}

pub(crate) struct AgentSessionHistoryReadUsecase {
    query: Arc<dyn AgentSessionHistoryQueryService>,
}

impl AgentSessionHistoryReadUsecase {
    pub(crate) fn new(query: Arc<dyn AgentSessionHistoryQueryService>) -> Self {
        Self { query }
    }

    pub(crate) async fn list(
        &self,
        request: AgentSessionHistoryRequest,
    ) -> Result<AgentSessionHistoryPageDto, AgentSessionHistoryQueryError> {
        self.query.list(request).await
    }
}
