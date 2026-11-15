use crate::adaptor::gateway::agent_session::test_helpers::*;
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
};
use crate::usecase::provider_dto::AgentSessionProviderDto;

struct FixedHistoryGateway {
    entries: Vec<AgentSessionHistoryMetadata>,
    titles: HashMap<(ProviderKind, String), Option<String>>,
    prompts: HashMap<(ProviderKind, String), Option<String>>,
    title_requests: Mutex<Vec<(ProviderKind, Vec<String>)>>,
    title_override: Option<Vec<ProviderSessionTitleEntry>>,
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
        if let Some(entries) = &self.title_override {
            return Ok(entries.clone());
        }
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
    requests: Mutex<Vec<(ProviderKind, String)>>,
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
                title_override: None,
                title_requests: Mutex::new(Vec::new()),
            }),
            Arc::new(FailingOwnershipQuery(error)),
        );

        // When
        let result = query
            .list(AgentSessionHistoryRequest {
                search: None,
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
        self.requests
            .lock()
            .unwrap()
            .push((provider, provider_session_id.into()));
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
        title_override: None,
        title_requests: Mutex::new(Vec::new()),
    });
    let query = LocalAgentSessionHistoryQueryService::new(
        history.clone(),
        Arc::new(FixedOwnershipQuery {
            requests: Mutex::new(vec![]),
            owned: HashSet::from([(ProviderKind::Claude, "claude-managed".to_string())]),
        }),
    );

    let page = query
        .list(AgentSessionHistoryRequest {
            search: None,
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
            title_override: None,
            title_requests: Mutex::new(Vec::new()),
        }),
        Arc::new(FixedOwnershipQuery {
            requests: Mutex::new(vec![]),
            owned: HashSet::new(),
        }),
    );
    let first = query
        .list(AgentSessionHistoryRequest {
            search: None,
            worktree_path: "/repo/worktree".to_string(),
            visible_count: 2,
        })
        .await
        .unwrap();
    let second = query
        .list(AgentSessionHistoryRequest {
            search: None,
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
        title_override: None,
        title_requests: Mutex::new(vec![]),
    });
    let query = LocalAgentSessionHistoryQueryService::new(
        history.clone(),
        Arc::new(FixedOwnershipQuery {
            requests: Mutex::new(vec![]),
            owned: HashSet::new(),
        }),
    );
    for count in [100, 120, 402, 420, usize::MAX] {
        let page = query
            .list(AgentSessionHistoryRequest {
                search: None,
                worktree_path: "/repo/worktree".into(),
                visible_count: count,
            })
            .await
            .unwrap();
        assert_eq!(page.items.len(), count.min(402));
        assert_eq!(page.has_more, count < 402);
    }
}

#[tokio::test]
async fn test_履歴検索_未表示の題と指示を絞ってからページ化する() {
    // Given
    let gateway = FixedHistoryGateway {
        entries: vec![
            metadata(ProviderKind::Codex, "new", 30),
            metadata(ProviderKind::Codex, "title", 20),
            metadata(ProviderKind::Codex, "prompt", 10),
        ],
        titles: HashMap::from([((ProviderKind::Codex, "title".into()), Some("Thread".into()))]),
        prompts: HashMap::from([(
            (ProviderKind::Codex, "prompt".into()),
            Some("thread in prompt".into()),
        )]),
        title_override: None,
        title_requests: Mutex::new(Vec::new()),
    };
    let query = LocalAgentSessionHistoryQueryService::new(
        Arc::new(gateway),
        Arc::new(FixedOwnershipQuery {
            requests: Mutex::new(vec![]),
            owned: HashSet::new(),
        }),
    );
    // When
    let page = query
        .list(AgentSessionHistoryRequest {
            worktree_path: "/repo/worktree".into(),
            visible_count: 1,
            search: crate::domain::search_term::SearchTerm::new("THREAD"),
        })
        .await
        .unwrap();
    // Then
    assert_eq!(page.items[0].provider_session_id, "title");
    assert!(page.has_more);
}

#[tokio::test]
async fn test_履歴検索_各providerの202件目以降は検索結果に含めない() {
    // Given
    let entries = ProviderKind::supported()
        .iter()
        .flat_map(|provider| {
            (0..203).map(move |index| metadata(*provider, &format!("session-{index}"), 203 - index))
        })
        .collect();
    let titles = ProviderKind::supported()
        .iter()
        .map(|provider| {
            (
                (*provider, "session-201".into()),
                Some("needle title".into()),
            )
        })
        .collect();
    let prompts = ProviderKind::supported()
        .iter()
        .map(|provider| {
            (
                (*provider, "session-202".into()),
                Some("needle prompt".into()),
            )
        })
        .collect();
    let query = LocalAgentSessionHistoryQueryService::new(
        Arc::new(FixedHistoryGateway {
            entries,
            titles,
            prompts,
            title_override: None,
            title_requests: Mutex::new(Vec::new()),
        }),
        Arc::new(FixedOwnershipQuery {
            requests: Mutex::new(vec![]),
            owned: HashSet::new(),
        }),
    );
    // When / Then
    for count in [1, 4] {
        let page = query
            .list(AgentSessionHistoryRequest {
                worktree_path: "/repo/worktree".into(),
                visible_count: count,
                search: crate::domain::search_term::SearchTerm::new("needle"),
            })
            .await
            .unwrap();
        assert!(page.items.is_empty());
        assert!(!page.has_more);
    }
}

#[tokio::test]
async fn test_履歴検索_区切りをまたいで所有されない一致だけを数え続きで読み取りを止める() {
    // Given
    for matches in [2, 3, 138] {
        let history = Arc::new(FixedHistoryGateway {
            entries: (0..201)
                .map(|i| metadata(ProviderKind::Codex, &format!("s-{i}"), 201 - i))
                .collect(),
            titles: (62..(63 + matches))
                .map(|i| {
                    (
                        (ProviderKind::Codex, format!("s-{i}")),
                        Some("needle".into()),
                    )
                })
                .collect(),
            prompts: HashMap::new(),
            title_requests: Mutex::new(vec![]),
            title_override: None,
        });
        let ownership = Arc::new(FixedOwnershipQuery {
            owned: HashSet::from([(ProviderKind::Codex, "s-62".into())]),
            requests: Mutex::new(vec![]),
        });
        let query = LocalAgentSessionHistoryQueryService::new(history.clone(), ownership.clone());
        // When
        let page = query
            .list(AgentSessionHistoryRequest {
                worktree_path: "/repo/worktree".into(),
                visible_count: 2,
                search: crate::domain::search_term::SearchTerm::new("needle"),
            })
            .await
            .unwrap();
        // Then
        assert_eq!(
            page.items
                .iter()
                .map(|x| x.provider_session_id.as_str())
                .collect::<Vec<_>>(),
            ["s-63", "s-64"]
        );
        assert_eq!(page.has_more, matches >= 3);
        assert_eq!(
            ownership
                .requests
                .lock()
                .unwrap()
                .iter()
                .map(|(_, id)| id.clone())
                .collect::<Vec<_>>(),
            (0..if matches >= 3 { 128 } else { 201 })
                .map(|i| format!("s-{i}"))
                .collect::<Vec<_>>()
        );
        let requests = history.title_requests.lock().unwrap();
        assert_eq!(requests[0].1.len(), 63);
        assert!(!requests[0].1.contains(&"s-62".to_string()));
        assert_eq!(requests[0].1.last().unwrap(), "s-63");
        assert_eq!(requests[1].1.first().unwrap(), "s-64");
        assert_eq!(requests.len(), if matches >= 3 { 2 } else { 4 });
    }
}

#[tokio::test]
async fn test_履歴照会_題の件数不一致と未要求idと重複idを拒否する() {
    // Given
    for ids in [vec!["a"], vec!["a", "unexpected"], vec!["a", "a"]] {
        for search in [None, crate::domain::search_term::SearchTerm::new("needle")] {
            let history = FixedHistoryGateway {
                entries: vec![
                    metadata(ProviderKind::Codex, "a", 2),
                    metadata(ProviderKind::Codex, "b", 1),
                ],
                titles: HashMap::new(),
                prompts: HashMap::new(),
                title_requests: Mutex::new(vec![]),
                title_override: Some(
                    ids.iter()
                        .map(|id| ProviderSessionTitleEntry {
                            provider_session_id: (*id).into(),
                            session_title: Some("needle".into()),
                            first_user_prompt: None,
                        })
                        .collect(),
                ),
            };
            let query = LocalAgentSessionHistoryQueryService::new(
                Arc::new(history),
                Arc::new(FixedOwnershipQuery {
                    owned: HashSet::new(),
                    requests: Mutex::new(vec![]),
                }),
            );
            // When
            let result = query
                .list(AgentSessionHistoryRequest {
                    worktree_path: "/repo/worktree".into(),
                    visible_count: 2,
                    search,
                })
                .await;
            // Then
            assert_eq!(result.unwrap_err(), AgentSessionHistoryQueryError::Corrupt);
        }
    }
}
