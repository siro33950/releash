use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use super::LocalAgentSessionHistoryQueryService;
use crate::domain::agent_session::{
    AgentSessionHistoryGateway, AgentSessionHistoryGatewayError, AgentSessionHistoryMetadata,
    AgentSessionOwnershipQuery, ProviderSessionTitleEntry,
};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::usecase::agent_session::{
    AgentSessionHistoryQueryError, AgentSessionHistoryQueryService, AgentSessionHistoryRequest,
    AgentSessionProviderDto,
};

struct FixedHistoryGateway {
    entries: Vec<AgentSessionHistoryMetadata>,
    titles: HashMap<(ProviderKind, String), Option<String>>,
    prompts: HashMap<(ProviderKind, String), Option<String>>,
    title_requests: Mutex<Vec<(ProviderKind, Vec<String>)>>,
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

    async fn list_session_titles(
        &self,
        provider: ProviderKind,
        _worktree_path: &str,
        provider_session_ids: &[String],
    ) -> Result<Vec<ProviderSessionTitleEntry>, AgentSessionHistoryGatewayError> {
        self.title_requests
            .lock()
            .unwrap()
            .push((provider, provider_session_ids.to_vec()));
        Ok(provider_session_ids
            .iter()
            .map(|provider_session_id| ProviderSessionTitleEntry {
                provider_session_id: provider_session_id.clone(),
                session_title: self
                    .titles
                    .get(&(provider, provider_session_id.clone()))
                    .cloned()
                    .flatten(),
                first_user_prompt: self
                    .prompts
                    .get(&(provider, provider_session_id.clone()))
                    .cloned()
                    .flatten(),
            })
            .collect())
    }
}

struct FixedOwnershipQuery {
    owned: HashSet<(ProviderKind, String)>,
}

struct FailingOwnershipQuery(AgentSessionHistoryGatewayError);

#[async_trait::async_trait]
impl AgentSessionOwnershipQuery for FailingOwnershipQuery {
    async fn is_owned(
        &self,
        _provider: ProviderKind,
        _provider_session_id: &str,
    ) -> Result<bool, AgentSessionHistoryGatewayError> {
        Err(self.0.clone())
    }
}

#[tokio::test]
async fn test_履歴照会_所有照会の業務失敗を保持する() {
    // Given
    for (error, expected) in [
        (
            AgentSessionHistoryGatewayError::Conflict,
            AgentSessionHistoryQueryError::Conflict,
        ),
        (
            AgentSessionHistoryGatewayError::ProviderSessionAlreadyOwned {
                agent_session_id: "owner".into(),
            },
            AgentSessionHistoryQueryError::ProviderSessionAlreadyOwned {
                agent_session_id: "owner".into(),
            },
        ),
    ] {
        let query = LocalAgentSessionHistoryQueryService::new(
            Arc::new(FixedHistoryGateway {
                entries: vec![metadata(ProviderKind::Codex, "session", 10)],
                titles: HashMap::new(),
                prompts: HashMap::new(),
                title_requests: Mutex::new(Vec::new()),
            }),
            Arc::new(FailingOwnershipQuery(error)),
        );

        // When
        let result = query
            .list(AgentSessionHistoryRequest {
                worktree_path: "/repo/worktree".into(),
                visible_count: 1,
            })
            .await;

        // Then
        assert_eq!(result.unwrap_err(), expected);
    }
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
    let history = Arc::new(FixedHistoryGateway {
        entries: vec![
            metadata(ProviderKind::Claude, "claude-old", 10),
            metadata(ProviderKind::Claude, "claude-older", 5),
            metadata(ProviderKind::Claude, "claude-managed", 30),
            metadata(ProviderKind::Codex, "codex-new", 40),
            metadata(ProviderKind::Codex, "codex-old", 20),
        ],
        titles: HashMap::from([
            (
                (ProviderKind::Codex, "codex-new".to_string()),
                Some("  New Codex title  ".to_string()),
            ),
            (
                (ProviderKind::Codex, "codex-old".to_string()),
                Some("  ".to_string()),
            ),
        ]),
        prompts: HashMap::from([
            (
                (ProviderKind::Claude, "claude-old".to_string()),
                Some("  Review\nClaude session  ".to_string()),
            ),
            (
                (ProviderKind::Codex, "codex-old".to_string()),
                Some("  Fix\nprovider history  ".to_string()),
            ),
        ]),
        title_requests: Mutex::new(Vec::new()),
    });
    let query = LocalAgentSessionHistoryQueryService::new(
        history.clone(),
        Arc::new(FixedOwnershipQuery {
            owned: HashSet::from([(ProviderKind::Claude, "claude-managed".to_string())]),
        }),
    );

    let page = query
        .list(AgentSessionHistoryRequest {
            worktree_path: "/repo/worktree".to_string(),
            visible_count: 3,
        })
        .await
        .unwrap();

    assert_eq!(
        page.items
            .iter()
            .map(|candidate| (
                candidate.provider,
                candidate.provider_session_id.as_str(),
                candidate.label.as_str(),
                candidate.updated_at_ms,
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                AgentSessionProviderDto::Codex,
                "codex-new",
                "New Codex title",
                40
            ),
            (
                AgentSessionProviderDto::Codex,
                "codex-old",
                "Fix provider history",
                20
            ),
            (
                AgentSessionProviderDto::Claude,
                "claude-old",
                "Review Claude session",
                10
            ),
        ]
    );
    assert!(page.has_more);
    assert_eq!(
        history.title_requests.lock().unwrap().as_slice(),
        &[
            (ProviderKind::Claude, vec!["claude-old".to_string()]),
            (
                ProviderKind::Codex,
                vec!["codex-new".to_string(), "codex-old".to_string()]
            ),
        ]
    );
}

#[tokio::test]
async fn test_agent_session_history_query_表示件数を増やすと先頭から指定件数を返す() {
    let query = LocalAgentSessionHistoryQueryService::new(
        Arc::new(FixedHistoryGateway {
            entries: vec![
                metadata(ProviderKind::Claude, "claude-3", 30),
                metadata(ProviderKind::Claude, "claude-1", 10),
                metadata(ProviderKind::Codex, "codex-2", 20),
            ],
            titles: HashMap::new(),
            prompts: HashMap::new(),
            title_requests: Mutex::new(Vec::new()),
        }),
        Arc::new(FixedOwnershipQuery {
            owned: HashSet::new(),
        }),
    );
    let first = query
        .list(AgentSessionHistoryRequest {
            worktree_path: "/repo/worktree".to_string(),
            visible_count: 2,
        })
        .await
        .unwrap();
    let second = query
        .list(AgentSessionHistoryRequest {
            worktree_path: "/repo/worktree".to_string(),
            visible_count: 3,
        })
        .await
        .unwrap();

    assert_eq!(first.items.len(), 2);
    assert!(first.has_more);
    assert_eq!(second.items.len(), 3);
    assert_eq!(second.items[2].provider_session_id, "claude-1");
    assert!(!second.has_more);
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

#[tokio::test]
async fn test_履歴表示件数_百件を超えて全providerの既存走査範囲へ到達できる() {
    let entries = ProviderKind::supported()
        .iter()
        .flat_map(|provider| {
            (0..201).map(move |index| metadata(*provider, &format!("session-{index}"), index))
        })
        .collect();
    let history = Arc::new(FixedHistoryGateway {
        entries,
        titles: HashMap::new(),
        prompts: HashMap::new(),
        title_requests: Mutex::new(vec![]),
    });
    let query = LocalAgentSessionHistoryQueryService::new(
        history.clone(),
        Arc::new(FixedOwnershipQuery {
            owned: HashSet::new(),
        }),
    );
    for count in [100, 120, 402, 420, usize::MAX] {
        let page = query
            .list(AgentSessionHistoryRequest {
                worktree_path: "/repo/worktree".into(),
                visible_count: count,
            })
            .await
            .unwrap();
        assert_eq!(page.items.len(), count.min(402));
        assert_eq!(page.has_more, count < 402);
    }
}
