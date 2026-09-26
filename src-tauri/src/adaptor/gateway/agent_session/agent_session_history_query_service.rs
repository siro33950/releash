use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::agent_session::{
    provider_history_label, AgentSessionHistoryGateway, AgentSessionHistoryGatewayError,
    AgentSessionHistoryMetadata, AgentSessionOwnershipQuery,
};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::usecase::agent_session::{
    AgentSessionHistoryCandidateDto, AgentSessionHistoryPageDto, AgentSessionHistoryQueryError,
    AgentSessionHistoryQueryService, AgentSessionHistoryRequest, AgentSessionProviderDto,
};

const MAX_SCAN_PER_PROVIDER: usize = 201;

pub(crate) struct LocalAgentSessionHistoryQueryService {
    history: Arc<dyn AgentSessionHistoryGateway>,
    ownership: Arc<dyn AgentSessionOwnershipQuery>,
}

impl LocalAgentSessionHistoryQueryService {
    pub(crate) fn new(
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
        let mut unique = HashMap::new();
        for provider in ProviderKind::supported() {
            let entries = self
                .history
                .list_metadata(*provider, &request.worktree_path, MAX_SCAN_PER_PROVIDER)
                .await
                .map_err(map_gateway_error)?;
            if entries.len() > MAX_SCAN_PER_PROVIDER {
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
            if visible.len() > request.visible_count {
                break;
            }
        }
        let has_more = visible.len() > request.visible_count;
        visible.truncate(request.visible_count);
        let mut titles = HashMap::new();
        for provider in ProviderKind::supported() {
            let provider_session_ids = visible
                .iter()
                .filter(|entry| entry.provider == *provider)
                .map(|entry| entry.provider_session_id.clone())
                .collect::<Vec<_>>();
            if provider_session_ids.is_empty() {
                continue;
            }
            let entries = self
                .history
                .list_session_titles(*provider, &request.worktree_path, &provider_session_ids)
                .await
                .map_err(map_gateway_error)?;
            if entries.len() != provider_session_ids.len() {
                return Err(AgentSessionHistoryQueryError::Corrupt);
            }
            for entry in entries {
                if !provider_session_ids.contains(&entry.provider_session_id)
                    || titles
                        .insert(
                            (*provider, entry.provider_session_id),
                            (entry.session_title, entry.first_user_prompt),
                        )
                        .is_some()
                {
                    return Err(AgentSessionHistoryQueryError::Corrupt);
                }
            }
        }
        Ok(AgentSessionHistoryPageDto {
            items: visible
                .into_iter()
                .map(|entry| {
                    let (title, first_user_prompt) = titles
                        .remove(&(entry.provider, entry.provider_session_id.clone()))
                        .unwrap_or((None, None));
                    let label = provider_history_label(
                        entry.provider,
                        &entry.provider_session_id,
                        title.as_deref(),
                        first_user_prompt.as_deref(),
                    );
                    AgentSessionHistoryCandidateDto {
                        provider: match entry.provider {
                            ProviderKind::Claude => AgentSessionProviderDto::Claude,
                            ProviderKind::Codex => AgentSessionProviderDto::Codex,
                        },
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
        AgentSessionHistoryGatewayError::Unavailable => AgentSessionHistoryQueryError::Unavailable,
        AgentSessionHistoryGatewayError::Store(kind) => AgentSessionHistoryQueryError::Store(kind),
        AgentSessionHistoryGatewayError::Corrupt => AgentSessionHistoryQueryError::Corrupt,
    }
}
