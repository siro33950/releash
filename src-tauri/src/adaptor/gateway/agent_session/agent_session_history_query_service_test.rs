use std::collections::HashSet;
use std::sync::Arc;

use super::LocalAgentSessionHistoryQueryService;
use crate::domain::agent_session::{
    AgentSessionHistoryGateway, AgentSessionHistoryGatewayError, AgentSessionHistoryMetadata,
    AgentSessionOwnershipQuery,
};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::usecase::agent_session::{
    AgentSessionHistoryQueryService, AgentSessionHistoryRequest, AgentSessionProviderDto,
};

struct FixedHistoryGateway {
    entries: Vec<AgentSessionHistoryMetadata>,
}

#[async_trait::async_trait]
impl AgentSessionHistoryGateway for FixedHistoryGateway {
    async fn list_metadata(
        &self,
        provider: ProviderKind,
        worktree_path: &str,
        limit: usize,
    ) -> Result<Vec<AgentSessionHistoryMetadata>, AgentSessionHistoryGatewayError> {
        Ok(self
            .entries
            .iter()
            .filter(|entry| entry.provider == provider && entry.worktree_path == worktree_path)
            .take(limit)
            .cloned()
            .collect())
    }
}

struct FixedOwnershipQuery {
    owned: HashSet<(ProviderKind, String)>,
}

#[async_trait::async_trait]
impl AgentSessionOwnershipQuery for FixedOwnershipQuery {
    async fn is_owned(
        &self,
        provider: ProviderKind,
        provider_session_id: &str,
    ) -> Result<bool, AgentSessionHistoryGatewayError> {
        Ok(self
            .owned
            .contains(&(provider, provider_session_id.to_string())))
    }
}

#[tokio::test]
async fn test_agent_session_history_query_metadataだけを並べ管理中idを除外する() {
    let query = LocalAgentSessionHistoryQueryService::new(
        Arc::new(FixedHistoryGateway {
            entries: vec![
                metadata(ProviderKind::Claude, "claude-old", 10),
                metadata(ProviderKind::Claude, "claude-managed", 30),
                metadata(ProviderKind::Codex, "codex-new", 40),
                metadata(ProviderKind::Codex, "codex-old", 20),
            ],
        }),
        Arc::new(FixedOwnershipQuery {
            owned: HashSet::from([(ProviderKind::Claude, "claude-managed".to_string())]),
        }),
    );

    let page = query
        .list(AgentSessionHistoryRequest {
            worktree_path: "/repo/worktree".to_string(),
            limit: 2,
            after: None,
        })
        .await
        .unwrap();

    assert_eq!(
        page.items
            .iter()
            .map(|candidate| (
                candidate.provider,
                candidate.provider_session_id.as_str(),
                candidate.updated_at_ms,
            ))
            .collect::<Vec<_>>(),
        vec![
            (AgentSessionProviderDto::Codex, "codex-new", 40),
            (AgentSessionProviderDto::Codex, "codex-old", 20),
        ]
    );
    assert!(page.next_after.is_some());
}

#[tokio::test]
async fn test_agent_session_history_query_cursorで次pageをboundedに返す() {
    let query = LocalAgentSessionHistoryQueryService::new(
        Arc::new(FixedHistoryGateway {
            entries: vec![
                metadata(ProviderKind::Claude, "claude-3", 30),
                metadata(ProviderKind::Claude, "claude-1", 10),
                metadata(ProviderKind::Codex, "codex-2", 20),
            ],
        }),
        Arc::new(FixedOwnershipQuery {
            owned: HashSet::new(),
        }),
    );
    let first = query
        .list(AgentSessionHistoryRequest {
            worktree_path: "/repo/worktree".to_string(),
            limit: 2,
            after: None,
        })
        .await
        .unwrap();
    let second = query
        .list(AgentSessionHistoryRequest {
            worktree_path: "/repo/worktree".to_string(),
            limit: 2,
            after: first.next_after,
        })
        .await
        .unwrap();

    assert_eq!(second.items.len(), 1);
    assert_eq!(second.items[0].provider_session_id, "claude-1");
    assert!(second.next_after.is_none());
}

fn metadata(
    provider: ProviderKind,
    provider_session_id: &str,
    updated_at_ms: i64,
) -> AgentSessionHistoryMetadata {
    AgentSessionHistoryMetadata {
        provider,
        provider_session_id: provider_session_id.to_string(),
        worktree_path: "/repo/worktree".to_string(),
        updated_at_ms,
    }
}
