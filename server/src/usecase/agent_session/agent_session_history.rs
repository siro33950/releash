use std::sync::Arc;

use serde::Serialize;

use crate::usecase::provider_dto::AgentSessionProviderDto;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionHistoryCandidateDto {
    pub provider: AgentSessionProviderDto,
    pub provider_session_id: String,
    pub label: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionHistoryPageDto {
    pub items: Vec<AgentSessionHistoryCandidateDto>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionHistoryRequest {
    pub worktree_path: String,
    pub visible_count: usize,
    /// 検索語。あれば、題か最初の指示が検索語を含む履歴に絞る。
    pub search: Option<crate::domain::search_term::SearchTerm>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSessionHistoryQueryError {
    Technical(crate::domain::failure::TechnicalFailure),
    Conflict,
    ProviderSessionAlreadyOwned { agent_session_id: String },
    Store(crate::domain::failure::StorageFailure),
    InvalidRequest,
    Corrupt,
}

#[async_trait::async_trait]
pub trait AgentSessionHistoryQueryService: Send + Sync {
    /// Provider の履歴を、最終更新の新しい順に `visible_count` 件まで返す。
    /// 検索語があれば、題か最初の指示が検索語を含む履歴に絞ってから数える。
    /// `has_more` は、絞った後の履歴に続きがあるかである。
    async fn list(
        &self,
        request: AgentSessionHistoryRequest,
    ) -> Result<AgentSessionHistoryPageDto, AgentSessionHistoryQueryError>;
}

pub struct AgentSessionHistoryReadUsecase {
    query: Arc<dyn AgentSessionHistoryQueryService>,
}

impl AgentSessionHistoryReadUsecase {
    pub fn new(query: Arc<dyn AgentSessionHistoryQueryService>) -> Self {
        Self { query }
    }

    pub(crate) async fn list(
        &self,
        request: AgentSessionHistoryRequest,
    ) -> Result<AgentSessionHistoryPageDto, AgentSessionHistoryQueryError> {
        self.query.list(request).await
    }
}

#[cfg(test)]
#[path = "agent_session_history_test.rs"]
mod agent_session_history_tests;
