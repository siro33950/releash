use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::domain::agent_session::{
    provider_history_label, AgentSessionHistoryGateway, AgentSessionHistoryGatewayError,
    AgentSessionHistoryMetadata, AgentSessionOwnershipQuery, ProviderSessionTitleEntry,
};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::usecase::agent_session::{
    AgentSessionHistoryCandidateDto, AgentSessionHistoryPageDto, AgentSessionHistoryQueryError,
    AgentSessionHistoryQueryService, AgentSessionHistoryRequest,
};

const MAX_SCAN_PER_PROVIDER: usize = 201;
const TITLE_BATCH_SIZE: usize = 64;

type HistoryTitles = HashMap<(ProviderKind, String), ProviderSessionTitleEntry>;

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
        let (mut visible, mut titles) = if let Some(search) = &request.search {
            self.search_unowned(&request, &candidates, search).await?
        } else {
            self.list_unowned(&request, candidates).await?
        };
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

impl LocalAgentSessionHistoryQueryService {
    async fn list_unowned(
        &self,
        request: &AgentSessionHistoryRequest,
        candidates: Vec<AgentSessionHistoryMetadata>,
    ) -> Result<(Vec<AgentSessionHistoryMetadata>, HistoryTitles), AgentSessionHistoryQueryError>
    {
        let mut visible = Vec::new();
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
        let titles = self
            .read_titles(
                &request.worktree_path,
                &visible[..visible.len().min(request.visible_count)],
            )
            .await?;
        Ok((visible, titles))
    }

    async fn search_unowned(
        &self,
        request: &AgentSessionHistoryRequest,
        candidates: &[AgentSessionHistoryMetadata],
        search: &crate::domain::search_term::SearchTerm,
    ) -> Result<(Vec<AgentSessionHistoryMetadata>, HistoryTitles), AgentSessionHistoryQueryError>
    {
        let mut visible = Vec::new();
        let mut titles = HistoryTitles::new();
        for batch in candidates.chunks(TITLE_BATCH_SIZE) {
            let mut unowned = Vec::new();
            for candidate in batch {
                if !self
                    .ownership
                    .is_owned(candidate.provider, &candidate.provider_session_id)
                    .await
                    .map_err(map_gateway_error)?
                {
                    unowned.push(candidate.clone());
                }
            }
            let mut batch_titles = self.read_titles(&request.worktree_path, &unowned).await?;
            for candidate in unowned {
                let key = (candidate.provider, candidate.provider_session_id.clone());
                let title = batch_titles.remove(&key).expect("validated title response");
                if !title.matches(search) {
                    continue;
                }
                visible.push(candidate);
                titles.insert(key, title);
                if visible.len() > request.visible_count {
                    return Ok((visible, titles));
                }
            }
        }
        Ok((visible, titles))
    }

    async fn read_titles(
        &self,
        worktree_path: &str,
        candidates: &[AgentSessionHistoryMetadata],
    ) -> Result<HistoryTitles, AgentSessionHistoryQueryError> {
        let mut titles = HistoryTitles::new();
        for provider in ProviderKind::supported() {
            let ids = candidates
                .iter()
                .filter(|entry| entry.provider == *provider)
                .map(|entry| entry.provider_session_id.clone())
                .collect::<Vec<_>>();
            if ids.is_empty() {
                continue;
            }
            let entries = self
                .history
                .list_session_titles(*provider, worktree_path, &ids)
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
                titles.insert((*provider, entry.provider_session_id.clone()), entry);
            }
        }
        Ok(titles)
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
