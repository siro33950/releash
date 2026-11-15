use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::domain::agent_session::{
    provider_history_label, AgentSessionHistoryGateway, AgentSessionHistoryGatewayError,
    AgentSessionHistoryMetadata, AgentSessionOwnershipQuery,
};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::usecase::agent_session::{
    AgentSessionHistoryCandidateDto, AgentSessionHistoryPageDto, AgentSessionHistoryQueryError,
    AgentSessionHistoryQueryService, AgentSessionHistoryRequest,
};

const MAX_SCAN_PER_PROVIDER: usize = 201;

pub struct LocalAgentSessionHistoryQueryService {
    history: Arc<dyn AgentSessionHistoryGateway>,
    ownership: Arc<dyn AgentSessionOwnershipQuery>,
}

impl LocalAgentSessionHistoryQueryService {
    pub fn new(
        history: Arc<dyn AgentSessionHistoryGateway>,
        ownership: Arc<dyn AgentSessionOwnershipQuery>,
    ) -> Self {
        Self { history, ownership }
    }
}

#[async_trait::async_trait]
impl AgentSessionHistoryQueryService for LocalAgentSessionHistoryQueryService {
    async fn list(
        &self,
        request: AgentSessionHistoryRequest,
    ) -> Result<AgentSessionHistoryPageDto, AgentSessionHistoryQueryError> {
        if request.worktree_path.trim().is_empty() || request.visible_count == 0 {
            return Err(AgentSessionHistoryQueryError::InvalidRequest);
        }
        let scan_limit = if request.search.is_some() {
            usize::MAX
        } else {
            MAX_SCAN_PER_PROVIDER
        };
        let mut unique = HashMap::new();
        for provider in ProviderKind::supported() {
            let entries = self
                .history
                .list_metadata(*provider, &request.worktree_path, scan_limit)
                .await
                .map_err(map_gateway_error)?;
            if entries.len() > scan_limit {
                return Err(AgentSessionHistoryQueryError::Corrupt);
            }
            for entry in entries {
                if entry.provider != *provider
                    || entry.worktree_path != request.worktree_path
                    || entry.provider_session_id.trim().is_empty()
                {
                    return Err(AgentSessionHistoryQueryError::Corrupt);
                }
                let key = (entry.provider, entry.provider_session_id.clone());
                unique
                    .entry(key)
                    .and_modify(|current: &mut AgentSessionHistoryMetadata| {
                        if entry.updated_at_ms > current.updated_at_ms {
                            *current = entry.clone();
                        }
                    })
                    .or_insert(entry);
            }
        }
        let mut candidates = unique.into_values().collect::<Vec<_>>();
        candidates.sort_by(compare_metadata);
        let mut visible = Vec::with_capacity(
            candidates
                .len()
                .min(request.visible_count.saturating_add(1)),
        );
        for candidate in candidates {
            if self
                .ownership
                .is_owned(candidate.provider, &candidate.provider_session_id)
                .await
                .map_err(map_gateway_error)?
            {
                continue;
            }
            visible.push(candidate);
            if request.search.is_none() && visible.len() > request.visible_count {
                break;
            }
        }
        let mut titles = HashMap::new();
        for provider in ProviderKind::supported() {
            let provider_session_ids = visible
                .iter()
                .take(if request.search.is_some() {
                    visible.len()
                } else {
                    request.visible_count
                })
                .filter(|entry| entry.provider == *provider)
                .map(|entry| entry.provider_session_id.clone())
                .collect::<Vec<_>>();
            if provider_session_ids.is_empty() {
                continue;
            }
            let batch_size = if request.search.is_some() {
                64
            } else {
                provider_session_ids.len()
            };
            let previous_count = titles.len();
            for ids in provider_session_ids.chunks(batch_size) {
                let entries = self
                    .history
                    .list_session_titles(*provider, &request.worktree_path, ids)
                    .await
                    .map_err(map_gateway_error)?;
                if entries.len() != ids.len() {
                    return Err(AgentSessionHistoryQueryError::Corrupt);
                }
                let mut expected = ids.iter().map(String::as_str).collect::<HashSet<_>>();
                for entry in entries {
                    if !expected.remove(entry.provider_session_id.as_str()) {
                        return Err(AgentSessionHistoryQueryError::Corrupt);
                    }
                    if request
                        .search
                        .as_ref()
                        .is_none_or(|term| entry.matches(term))
                    {
                        titles.insert((*provider, entry.provider_session_id.clone()), entry);
                    }
                }
                if request.search.is_some() && titles.len() - previous_count > request.visible_count
                {
                    break;
                }
            }
        }
        if request.search.is_some() {
            visible.retain(|entry| {
                titles.contains_key(&(entry.provider, entry.provider_session_id.clone()))
            });
        }
        let has_more = visible.len() > request.visible_count;
        visible.truncate(request.visible_count);
        Ok(AgentSessionHistoryPageDto {
            items: visible
                .into_iter()
                .map(|entry| {
                    let title = titles.remove(&(entry.provider, entry.provider_session_id.clone()));
                    let label = provider_history_label(
                        entry.provider,
                        &entry.provider_session_id,
                        title
                            .as_ref()
                            .and_then(|title| title.session_title.as_deref()),
                        title
                            .as_ref()
                            .and_then(|title| title.first_user_prompt.as_deref()),
                    );
                    AgentSessionHistoryCandidateDto {
                        provider: entry.provider.into(),
                        provider_session_id: entry.provider_session_id,
                        label,
                        updated_at_ms: entry.updated_at_ms,
                    }
                })
                .collect(),
            has_more,
        })
    }
}

fn compare_metadata(
    left: &AgentSessionHistoryMetadata,
    right: &AgentSessionHistoryMetadata,
) -> Ordering {
    right
        .updated_at_ms
        .cmp(&left.updated_at_ms)
        .then_with(|| provider_rank(left.provider).cmp(&provider_rank(right.provider)))
        .then_with(|| left.provider_session_id.cmp(&right.provider_session_id))
}

fn provider_rank(provider: ProviderKind) -> u8 {
    match provider {
        ProviderKind::Claude => 0,
        ProviderKind::Codex => 1,
    }
}

fn map_gateway_error(error: AgentSessionHistoryGatewayError) -> AgentSessionHistoryQueryError {
    match error {
        AgentSessionHistoryGatewayError::Conflict => AgentSessionHistoryQueryError::Conflict,
        AgentSessionHistoryGatewayError::ProviderSessionAlreadyOwned { agent_session_id } => {
            AgentSessionHistoryQueryError::ProviderSessionAlreadyOwned { agent_session_id }
        }
        AgentSessionHistoryGatewayError::InvalidRequest => {
            AgentSessionHistoryQueryError::InvalidRequest
        }
        AgentSessionHistoryGatewayError::Technical(error) => {
            AgentSessionHistoryQueryError::Technical(error)
        }
        AgentSessionHistoryGatewayError::Store(kind) => AgentSessionHistoryQueryError::Store(kind),
        AgentSessionHistoryGatewayError::Corrupt => AgentSessionHistoryQueryError::Corrupt,
    }
}

#[cfg(test)]
#[path = "agent_session_history_query_service_test.rs"]
mod agent_session_history_query_service_tests;
