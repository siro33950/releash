use super::*;
use crate::adaptor::gateway::agent_session::LocalAgentSessionRepository;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::test_support::{
    seed_workflow_session_facts, WorkflowSessionFactSeed,
};
use crate::domain::agent_session::aggregates::{
    AgentSession, AgentSessionRecoveryResult, AgentSessionTreeLocation,
};
use crate::domain::agent_session::repository::AgentSessionRepository;
use crate::domain::failure::{BusinessFailure, Failure, TechnicalFailureNature};
use crate::domain::local_event::WorkflowExecutionMetadataRecord;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::{
    AgentSessionActivity, ExecutionOrigin, ExecutionStatus, ExecutionTreeArchiveRecord,
    ExecutionTreeArchiveSnapshot, NodeFact, StopReceivedFact, TokenUsage,
};
use crate::domain::workspace_tree::{
    WorkspaceNodeStatus, WorkspaceNodeStatusClassification, WorkspaceTreeNode,
};
use crate::usecase::agent_session::{
    AgentSessionOperationsDto, AgentSessionProviderDto, AgentSessionUsecase,
};

struct EmptyArchives;

#[async_trait::async_trait]
impl ExecutionTreeArchiveRepository for EmptyArchives {
    async fn location(
        &self,
        id: &str,
    ) -> Result<crate::domain::workflow::ExecutionTreeArchiveCandidate, WorkflowError> {
        Ok(crate::domain::workflow::ExecutionTreeArchiveCandidate {
            execution_id: id.into(),
            worktree_path: "/tmp/wt".into(),
            workspace_identity: "/tmp/wt".into(),
            repository_root: None,
        })
    }
    fn worktree_identity(&self, path: &str) -> Result<String, WorkflowError> {
        Ok(crate::domain::repository::normalize_repo_path(path))
    }
    async fn record_repository_root(&self, _: &str, _: &str, _: f64) -> Result<(), WorkflowError> {
        unreachable!()
    }
    async fn candidate_page(
        &self,
        _: Option<&str>,
    ) -> Result<Vec<crate::domain::workflow::ExecutionTreeArchiveCandidate>, WorkflowError> {
        unreachable!()
    }
    async fn legacy_session_archive_page(
        &self,
        _: Option<&str>,
    ) -> Result<Vec<crate::domain::workflow::ExecutionTreeArchiveRecord>, WorkflowError> {
        Ok(Vec::new())
    }

    async fn worktree_target_page(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Vec<crate::domain::workflow::ExecutionTreeArchiveCandidate>, WorkflowError> {
        unreachable!()
    }
    async fn target(
        &self,
        _: &str,
    ) -> Result<crate::domain::workflow::ExecutionTreeArchiveTarget, WorkflowError> {
        unreachable!()
    }
    fn legacy_archives(
        &self,
    ) -> Result<Vec<crate::domain::workflow::ExecutionTreeArchiveRecord>, WorkflowError> {
        Ok(Vec::new())
    }
    fn finish_legacy_migration(&self) -> Result<(), WorkflowError> {
        Ok(())
    }

    async fn archive(
        &self,
        _execution_id: &crate::domain::workflow::ExecutionTreeId,
        _archived_at: f64,
        _reason: &str,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }

    async fn restore(
        &self,
        _execution_id: &crate::domain::workflow::ExecutionTreeId,
        _restored_at: f64,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }

    async fn archive_snapshot_for(
        &self,
        _execution_ids: &[String],
    ) -> Result<ExecutionTreeArchiveSnapshot, WorkflowError> {
        Ok(ExecutionTreeArchiveSnapshot {
            records: Vec::new(),
        })
    }
}

struct ArchivedExecution {
    execution_id: String,
    archived_at: f64,
}

#[async_trait::async_trait]
impl ExecutionTreeArchiveRepository for ArchivedExecution {
    async fn location(
        &self,
        id: &str,
    ) -> Result<crate::domain::workflow::ExecutionTreeArchiveCandidate, WorkflowError> {
        Ok(crate::domain::workflow::ExecutionTreeArchiveCandidate {
            execution_id: id.into(),
            worktree_path: "/tmp/wt".into(),
            workspace_identity: "/tmp/wt".into(),
            repository_root: None,
        })
    }
    fn worktree_identity(&self, path: &str) -> Result<String, WorkflowError> {
        Ok(crate::domain::repository::normalize_repo_path(path))
    }
    async fn record_repository_root(&self, _: &str, _: &str, _: f64) -> Result<(), WorkflowError> {
        unreachable!()
    }
    async fn candidate_page(
        &self,
        _: Option<&str>,
    ) -> Result<Vec<crate::domain::workflow::ExecutionTreeArchiveCandidate>, WorkflowError> {
        unreachable!()
    }
    async fn legacy_session_archive_page(
        &self,
        _: Option<&str>,
    ) -> Result<Vec<crate::domain::workflow::ExecutionTreeArchiveRecord>, WorkflowError> {
        Ok(Vec::new())
    }

    async fn worktree_target_page(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Vec<crate::domain::workflow::ExecutionTreeArchiveCandidate>, WorkflowError> {
        unreachable!()
    }
    async fn target(
        &self,
        _: &str,
    ) -> Result<crate::domain::workflow::ExecutionTreeArchiveTarget, WorkflowError> {
        unreachable!()
    }
    fn legacy_archives(
        &self,
    ) -> Result<Vec<crate::domain::workflow::ExecutionTreeArchiveRecord>, WorkflowError> {
        Ok(Vec::new())
    }
    fn finish_legacy_migration(&self) -> Result<(), WorkflowError> {
        Ok(())
    }

    async fn archive(
        &self,
        _execution_id: &crate::domain::workflow::ExecutionTreeId,
        _archived_at: f64,
        _reason: &str,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }

    async fn restore(
        &self,
        _execution_id: &crate::domain::workflow::ExecutionTreeId,
        _restored_at: f64,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }

    async fn archive_snapshot_for(
        &self,
        execution_ids: &[String],
    ) -> Result<ExecutionTreeArchiveSnapshot, WorkflowError> {
        let records = execution_ids
            .contains(&self.execution_id)
            .then(|| ExecutionTreeArchiveRecord {
                execution_id: self.execution_id.clone(),
                archived_at: self.archived_at,
                archive_reason: "manual".to_string(),
            })
            .into_iter()
            .collect();
        Ok(ExecutionTreeArchiveSnapshot { records })
    }
}

async fn assert_working_session_projection(store: Arc<LocalEventStore>) {
    let workspace = WorkspaceIdentity::new("workspace-activity-read");
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query = SqliteWorkspaceQueryService::with_repository(
        crate::usecase::work_queue::shared().clone(),
        repository,
        Arc::new(EmptyArchives),
    );

    let snapshot = query.workspace_tree(&workspace).await.unwrap();
    let WorkspaceTreeItemDto::Node(node) = &snapshot.nodes[0] else {
        panic!("standalone Session must be projected as a node");
    };
    assert_eq!(node.status, "active");
    let detail = query
        .node_detail(&workspace, &node.id)
        .await
        .unwrap()
        .expect("Session detail must exist");
    assert_eq!(detail.status, "running");
    assert_eq!(detail.status_classification, "active");
    let snapshot_json = serde_json::to_string(&snapshot).unwrap();
    let detail_json = serde_json::to_string(&detail).unwrap();
    assert!(!snapshot_json.contains("\"activity\":"));
    assert!(!detail_json.contains("\"activity\":"));
}

async fn assert_workflow_child_activity_projection(
    store: Arc<LocalEventStore>,
    expected_classification: &str,
) {
    let workspace = WorkspaceIdentity::new("/repo/workflow-child-activity");
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query = SqliteWorkspaceQueryService::with_repository(
        crate::usecase::work_queue::shared().clone(),
        repository,
        Arc::new(EmptyArchives),
    );

    let snapshot = query.workspace_tree(&workspace).await.unwrap();
    let WorkspaceTreeItemDto::Sequence(sequence) = &snapshot.nodes[0] else {
        panic!("workflow root must be projected as a sequence");
    };
    let WorkspaceTreeItemDto::Node(node) = &sequence.children[0] else {
        panic!("workflow child Session must be projected as a node");
    };
    assert_eq!(node.status, expected_classification);
    let detail = query
        .node_detail(&workspace, &node.id)
        .await
        .unwrap()
        .expect("workflow child Session detail must exist");
    assert_eq!(detail.status, "running");
    assert_eq!(detail.status_classification, expected_classification);
}

#[tokio::test]
async fn test_workspace_tree_query_記録済み活動状態を一覧と詳細へ反映し再起動後も再現する() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let sessions =
        AgentSessionUsecase::new(Arc::new(LocalAgentSessionRepository::new(store.clone())));
    sessions
        .create(
            "standalone-activity-session",
            WorkspaceIdentity::new("workspace-activity-read"),
            "/repo/activity-read",
            ProviderKind::Codex,
            AgentSessionTreeLocation::session_tree_root("standalone-activity-session").unwrap(),
            "create-activity-read-session",
        )
        .await
        .unwrap();
    sessions
        .observe_activity(
            "standalone-activity-session",
            AgentSessionActivity::Working,
            "observe-working-for-read",
        )
        .await
        .unwrap();
    drop(sessions);

    assert_working_session_projection(store.clone()).await;
    drop(store);

    let reopened = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    assert_working_session_projection(reopened).await;
}

#[tokio::test]
async fn test_workspace_tree_query_活動未観測のsessionを一覧と詳細でattentionにする() {
    // Given: create 後に活動事実を一度も記録していない単独 Session
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("workspace-initial-activity");
    let session_id = "standalone-initial-activity";
    let sessions =
        AgentSessionUsecase::new(Arc::new(LocalAgentSessionRepository::new(store.clone())));
    sessions
        .create(
            session_id,
            workspace.clone(),
            "/repo/initial-activity",
            ProviderKind::Codex,
            AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
            "create-initial-activity-session",
        )
        .await
        .unwrap();
    let records =
        crate::adaptor::gateway::workflow::fact_log::read_tree_records(&store, session_id)
            .await
            .unwrap();
    assert!(!records
        .iter()
        .any(|record| matches!(record.fact, NodeFact::AgentActivityObserved(_))));

    // When: Workspace query service から一覧と詳細を読む
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query = SqliteWorkspaceQueryService::with_repository(
        crate::usecase::work_queue::shared().clone(),
        repository,
        Arc::new(EmptyArchives),
    );
    let snapshot = query.workspace_tree(&workspace).await.unwrap();
    let WorkspaceTreeItemDto::Node(node) = &snapshot.nodes[0] else {
        panic!("standalone Session must be projected as a node");
    };
    let detail = query
        .node_detail(&workspace, &node.id)
        .await
        .unwrap()
        .expect("Session detail must exist");

    // Then: fold と projection を通った一覧・詳細の双方が attention になる
    assert_eq!(node.status, "attention");
    assert_eq!(detail.status, "running");
    assert_eq!(detail.status_classification, "attention");
}

#[tokio::test]
async fn test_workspace_tree_query_resume直後のsessionを一覧と詳細でattentionにする() {
    // Given: Working の後に正常終了し、provider resume が完了した単独 Session
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("workspace-resumed-activity");
    let session_id = "standalone-resumed-activity";
    let sessions =
        AgentSessionUsecase::new(Arc::new(LocalAgentSessionRepository::new(store.clone())));
    sessions
        .create(
            session_id,
            workspace.clone(),
            "/repo/resumed-activity",
            ProviderKind::Claude,
            AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
            "create-resumed-activity-session",
        )
        .await
        .unwrap();
    sessions
        .associate_provider_session(
            session_id,
            "provider-resumed-activity",
            None,
            "associate-resumed-activity-session",
        )
        .await
        .unwrap();
    sessions
        .observe_activity(
            session_id,
            AgentSessionActivity::Working,
            "observe-working-before-resume",
        )
        .await
        .unwrap();
    sessions
        .observe_process_exit(session_id, Some(0), "observe-exit-before-resume")
        .await
        .unwrap();
    sessions
        .complete_resume(
            session_id,
            AgentSessionRecoveryResult::Succeeded,
            "complete-provider-resume",
        )
        .await
        .unwrap();

    // When: Workspace query service から一覧と詳細を読む
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query = SqliteWorkspaceQueryService::with_repository(
        crate::usecase::work_queue::shared().clone(),
        repository,
        Arc::new(EmptyArchives),
    );
    let snapshot = query.workspace_tree(&workspace).await.unwrap();
    let WorkspaceTreeItemDto::Node(node) = &snapshot.nodes[0] else {
        panic!("standalone Session must be projected as a node");
    };
    let detail = query
        .node_detail(&workspace, &node.id)
        .await
        .unwrap()
        .expect("Session detail must exist");

    // Then: resume 後に新しい活動を観測するまでは一覧・詳細の双方が attention になる
    assert_eq!(node.status, "attention");
    assert_eq!(detail.status, "running");
    assert_eq!(detail.status_classification, "attention");
}

#[tokio::test]
async fn test_workspace_tree_query_workflow子sessionの活動状態を一覧と詳細へ反映し再起動後も再現する(
) {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    seed_workflow_session_facts(
        &store,
        WorkflowSessionFactSeed {
            workflow_name: "workflow-child-activity",
            request: "test child activity",
            worktree_path: "/repo/workflow-child-activity",
            provider: ProviderKind::Codex,
            workflow_execution_id: "00000000-0000-4000-8000-000000001700",
            node_execution_id: "workflow-child-node",
            session_id: "workflow-child-session",
            initial_instruction_admitted: true,
        },
    )
    .await
    .unwrap();
    let sessions =
        AgentSessionUsecase::new(Arc::new(LocalAgentSessionRepository::new(store.clone())));
    sessions
        .observe_activity(
            "workflow-child-session",
            AgentSessionActivity::Working,
            "observe-workflow-child-working",
        )
        .await
        .unwrap();
    drop(sessions);

    assert_workflow_child_activity_projection(store.clone(), "active").await;
    drop(store);

    let reopened = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    assert_workflow_child_activity_projection(reopened.clone(), "active").await;

    let sessions =
        AgentSessionUsecase::new(Arc::new(LocalAgentSessionRepository::new(reopened.clone())));
    sessions
        .observe_activity(
            "workflow-child-session",
            AgentSessionActivity::AwaitingAnswer,
            "observe-workflow-child-awaiting-answer",
        )
        .await
        .unwrap();
    assert_workflow_child_activity_projection(reopened, "attention").await;
}

#[tokio::test]
async fn test_workspace_tree_query_活動終了と再開の反復を一覧と詳細へ毎回反映する() {
    // Given: 実行中の単独 Session と同じ store を読む Workspace query
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("workspace-activity-round-trip");
    let session_id = "standalone-activity-round-trip";
    let sessions =
        AgentSessionUsecase::new(Arc::new(LocalAgentSessionRepository::new(store.clone())));
    sessions
        .create(
            session_id,
            workspace.clone(),
            "/repo/activity-round-trip",
            ProviderKind::Claude,
            AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
            "create-activity-round-trip",
        )
        .await
        .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query = SqliteWorkspaceQueryService::with_repository(
        crate::usecase::work_queue::shared().clone(),
        repository,
        Arc::new(EmptyArchives),
    );
    let projected_statuses = async || {
        let snapshot = query.workspace_tree(&workspace).await.unwrap();
        let WorkspaceTreeItemDto::Node(node) = &snapshot.nodes[0] else {
            panic!("standalone Session must be projected as a node");
        };
        let detail = query
            .node_detail(&workspace, &node.id)
            .await
            .unwrap()
            .expect("Session detail must exist");
        (
            node.status.clone(),
            detail.status,
            detail.status_classification,
        )
    };

    // When: Working と AwaitingInstruction を2往復させる
    let transitions = [
        (AgentSessionActivity::Working, "active"),
        (AgentSessionActivity::AwaitingInstruction, "attention"),
        (AgentSessionActivity::Working, "active"),
        (AgentSessionActivity::AwaitingInstruction, "attention"),
        (AgentSessionActivity::Working, "active"),
    ];
    let mut observed_classifications = Vec::new();
    for (index, (activity, expected_classification)) in transitions.into_iter().enumerate() {
        let request_id = format!("activity-round-trip-{index}");
        sessions
            .observe_activity(session_id, activity, &request_id)
            .await
            .unwrap();

        // Then: 一覧の status と詳細の statusClassification が毎回追従する
        let (tree_status, detail_status, detail_classification) = projected_statuses().await;
        assert_eq!(tree_status, expected_classification);
        assert_eq!(detail_status, "running");
        assert_eq!(detail_classification, expected_classification);
        observed_classifications.push(tree_status);
    }
    assert_eq!(
        observed_classifications,
        ["active", "attention", "active", "attention", "active"]
    );
}

#[tokio::test]
async fn test_workspace_tree_query_stop事実と後続活動を一覧と詳細へ反映し再起動後も再現する() {
    // Given: Working の活動観測がある実行中の単独 Session
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("workspace-stop-activity-read");
    let session_id = "standalone-stop-activity-read";
    let sessions =
        AgentSessionUsecase::new(Arc::new(LocalAgentSessionRepository::new(store.clone())));
    sessions
        .create(
            session_id,
            workspace.clone(),
            "/repo/stop-activity-read",
            ProviderKind::Codex,
            AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
            "create-stop-activity-read-session",
        )
        .await
        .unwrap();
    sessions
        .observe_activity(
            session_id,
            AgentSessionActivity::Working,
            "observe-working-before-stop",
        )
        .await
        .unwrap();
    let meta = crate::adaptor::gateway::workflow::fact_log::read_tree_records(&store, session_id)
        .await
        .unwrap()
        .last()
        .unwrap()
        .meta
        .clone();
    let append_stop = async |timestamp_ms| {
        crate::adaptor::gateway::workflow::fact_log::append_single_fact(
            &store,
            &meta,
            &NodeFact::StopReceived(StopReceivedFact {
                result_summary: None,
                token_usage: None,
            }),
            timestamp_ms,
        )
        .await
        .unwrap();
    };
    let projected_classification = async |store: Arc<LocalEventStore>| {
        let repository = SqliteWorkspaceTreeRepository::new(store);
        let query = SqliteWorkspaceQueryService::with_repository(
            crate::usecase::work_queue::shared().clone(),
            repository,
            Arc::new(EmptyArchives),
        );
        let snapshot = query.workspace_tree(&workspace).await.unwrap();
        let WorkspaceTreeItemDto::Node(node) = &snapshot.nodes[0] else {
            panic!("standalone Session must be projected as a node");
        };
        let detail = query
            .node_detail(&workspace, &node.id)
            .await
            .unwrap()
            .expect("Session detail must exist");
        assert_eq!(detail.status, "running");
        (node.status.clone(), detail.status_classification)
    };

    // When: 活動観測を追加せず StopReceived だけを追記する
    append_stop(10).await;

    // Then: 一覧と詳細は Stop 事実から attention を導出し、再起動後も再現する
    assert_eq!(
        projected_classification(store.clone()).await,
        ("attention".to_string(), "attention".to_string())
    );
    drop(sessions);
    drop(store);
    let reopened = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    assert_eq!(
        projected_classification(reopened.clone()).await,
        ("attention".to_string(), "attention".to_string())
    );

    // When / Then: 後続の Working が青へ戻し、再度の Stop と Working にも同じく追従する
    let sessions =
        AgentSessionUsecase::new(Arc::new(LocalAgentSessionRepository::new(reopened.clone())));
    for index in 0..2 {
        sessions
            .observe_activity(
                session_id,
                AgentSessionActivity::Working,
                &format!("observe-working-after-stop-{index}"),
            )
            .await
            .unwrap();
        assert_eq!(
            projected_classification(reopened.clone()).await,
            ("active".to_string(), "active".to_string())
        );
        if index == 0 {
            crate::adaptor::gateway::workflow::fact_log::append_single_fact(
                &reopened,
                &meta,
                &NodeFact::StopReceived(StopReceivedFact {
                    result_summary: None,
                    token_usage: None,
                }),
                20,
            )
            .await
            .unwrap();
            assert_eq!(
                projected_classification(reopened.clone()).await,
                ("attention".to_string(), "attention".to_string())
            );
        }
    }
}

#[tokio::test]
async fn launch区分が同じworktreeのworkflow一覧とsession一覧を分ける() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    seed_workflow_session_facts(
        &store,
        WorkflowSessionFactSeed {
            workflow_name: "workflow",
            request: "test",
            worktree_path: "/repo",
            provider: ProviderKind::Codex,
            workflow_execution_id: "workflow-tree",
            node_execution_id: "workflow-node",
            session_id: "workflow-session",
            initial_instruction_admitted: true,
        },
    )
    .await
    .unwrap();
    LocalAgentSessionRepository::new(store.clone())
        .create(
            AgentSession::create(
                "standalone-session",
                WorkspaceIdentity::new("/repo"),
                "/repo",
                ProviderKind::Codex,
                AgentSessionTreeLocation::session_tree_root("standalone-session").unwrap(),
            )
            .unwrap(),
            "standalone-create",
        )
        .await
        .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query = SqliteWorkspaceQueryService::with_repository(
        crate::usecase::work_queue::shared().clone(),
        repository.clone(),
        Arc::new(EmptyArchives),
    );

    let workflow_ids = query
        .execution_summaries(Some(&WorkspaceIdentity::new("/repo")), None, None)
        .await
        .unwrap()
        .into_iter()
        .map(|summary| summary.execution_id)
        .collect::<Vec<_>>();
    let tree_ids = repository
        .folded_workspace_trees("/repo")
        .await
        .unwrap()
        .into_iter()
        .map(|(tree, _)| tree.aggregate.id.clone())
        .collect::<Vec<_>>();
    let session_ids = crate::adaptor::gateway::agent_session::workspace_session_items(
        &repository.fact_backend(),
        &tree_ids,
        "/repo",
    )
    .await
    .unwrap()
    .into_iter()
    .map(|session| session.id)
    .collect::<Vec<_>>();

    assert_eq!(workflow_ids, ["workflow-tree"]);
    assert_eq!(session_ids, ["standalone-session"]);
}

#[tokio::test]
async fn test_workspace_tree_query_workspace同定子がworktreeと異なるsessionを一覧と行に含める() {
    // Given: workspace identity と worktree path が異なる Session 起動由来の木
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("workspace-1");
    LocalAgentSessionRepository::new(store.clone())
        .create(
            AgentSession::create(
                "standalone-session",
                workspace.clone(),
                "/repo/.worktrees/feature",
                ProviderKind::Codex,
                AgentSessionTreeLocation::session_tree_root("standalone-session").unwrap(),
            )
            .unwrap(),
            "standalone-create",
        )
        .await
        .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query = SqliteWorkspaceQueryService::with_repository(
        crate::usecase::work_queue::shared().clone(),
        repository.clone(),
        Arc::new(EmptyArchives),
    );

    // When: workspace identity から snapshot と Session 一覧を取得する
    let snapshot = query.workspace_tree(&workspace).await.unwrap();
    let tree_ids = repository
        .folded_workspace_trees(workspace.as_str())
        .await
        .unwrap()
        .into_iter()
        .map(|(tree, _)| tree.aggregate.id.clone())
        .collect::<Vec<_>>();
    let sessions = crate::adaptor::gateway::agent_session::workspace_session_items(
        &repository.fact_backend(),
        &tree_ids,
        workspace.as_str(),
    )
    .await
    .unwrap();

    // Then: root の workspace identity を共通の対象キーとして解決する
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "standalone-session");
    assert_eq!(snapshot.nodes.len(), 1);
    assert!(matches!(
        &snapshot.nodes[0],
        WorkspaceTreeItemDto::Node(node) if node.id == "standalone-session"
    ));
}

#[tokio::test]
async fn test_workspaceツリー投影_同じfoldのworkflow履歴と表示名が一致する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let execution_id = "00000000-0000-4000-8000-000000001662";
    seed_workflow_session_facts(
        &store,
        WorkflowSessionFactSeed {
            workflow_name: "01_author-spec",
            request: "test",
            worktree_path: "/repo",
            provider: ProviderKind::Codex,
            workflow_execution_id: execution_id,
            node_execution_id: "workflow-node",
            session_id: "workflow-session",
            initial_instruction_admitted: true,
        },
    )
    .await
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let tree_query = SqliteWorkspaceQueryService::with_repository(
        crate::usecase::work_queue::shared().clone(),
        repository.clone(),
        Arc::new(EmptyArchives),
    );
    let history_query = SqliteWorkspaceQueryService::with_repository(
        crate::usecase::work_queue::shared().clone(),
        repository,
        Arc::new(ArchivedExecution {
            execution_id: execution_id.to_string(),
            archived_at: 10.0,
        }),
    );

    // When
    let snapshot = tree_query
        .workspace_tree(&WorkspaceIdentity::new("/repo"))
        .await
        .unwrap();
    let history = history_query
        .workflow_history(&WorkspaceIdentity::new("/repo"))
        .await
        .unwrap();
    let tree_json = serde_json::to_value(snapshot.nodes).unwrap();
    let tree_title = tree_json[0]["title"].as_str().unwrap();

    // Then
    assert_eq!(tree_title, "01_author-spec");
    assert_ne!(tree_title, "main");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].execution_id, execution_id);
    assert_eq!(history[0].title, tree_title);
}

#[tokio::test]
async fn test_workspaceツリー投影_単独agent_sessionのpublic_root表示名はsessionを保つ() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("/repo");
    let execution_id = "standalone-session";
    LocalAgentSessionRepository::new(store.clone())
        .create(
            AgentSession::create(
                execution_id,
                workspace.clone(),
                workspace.as_str(),
                ProviderKind::Codex,
                AgentSessionTreeLocation::session_tree_root(execution_id).unwrap(),
            )
            .unwrap(),
            "standalone-create",
        )
        .await
        .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query = SqliteWorkspaceQueryService::with_repository(
        crate::usecase::work_queue::shared().clone(),
        repository,
        Arc::new(EmptyArchives),
    );

    // When
    let snapshot = query.workspace_tree(&workspace).await.unwrap();
    let detail = query
        .node_detail(&workspace, execution_id)
        .await
        .unwrap()
        .unwrap();
    let tree_json = serde_json::to_value(snapshot.nodes).unwrap();

    // Then
    assert_eq!(tree_json[0]["title"], "session");
    assert_eq!(detail.title, "session");
}

#[tokio::test]
async fn test_workspaceノード詳細_public_rootと子nodeの名前はnodeのtitleを保つ() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("/repo");
    let execution_id = "00000000-0000-4000-8000-000000001663";
    let child_execution_id = "workflow-node";
    seed_workflow_session_facts(
        &store,
        WorkflowSessionFactSeed {
            workflow_name: "01_author-spec",
            request: "test",
            worktree_path: workspace.as_str(),
            provider: ProviderKind::Codex,
            workflow_execution_id: execution_id,
            node_execution_id: child_execution_id,
            session_id: "workflow-session",
            initial_instruction_admitted: true,
        },
    )
    .await
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query = SqliteWorkspaceQueryService::with_repository(
        crate::usecase::work_queue::shared().clone(),
        repository.clone(),
        Arc::new(EmptyArchives),
    );
    let root_node = repository
        .load_node(&workspace, execution_id)
        .await
        .unwrap()
        .unwrap();
    let child_node = repository
        .load_node_by_node_execution_id(child_execution_id)
        .await
        .unwrap()
        .unwrap();

    // When
    let root_detail = query
        .node_detail(&workspace, execution_id)
        .await
        .unwrap()
        .unwrap();
    let child_detail = query
        .node_detail(&workspace, &child_node.id)
        .await
        .unwrap()
        .unwrap();

    // Then
    assert_eq!(root_node.title, "main");
    assert_eq!(root_detail.title, root_node.title);
    assert_eq!(child_node.title, "impl");
    assert_eq!(child_detail.title, child_node.title);
}

fn node() -> WorkspaceTreeNode {
    WorkspaceTreeNode {
        process_presence: Default::default(),
        can_resume_session: false,
        worktree: None,
        id: "node".to_string(),
        parent_id: None,
        sibling_order: 0,
        kind: WorkspaceNodeKind::WorkflowSession,
        title: "Review".to_string(),
        status: WorkspaceNodeStatus::Waiting,
        status_classification: WorkspaceNodeStatusClassification::Attention,
        activity: Some(crate::domain::workflow::AgentSessionActivity::AwaitingInstruction),
        error_reason: None,
        updated_at_bits: 1.0f64.to_bits(),
        execution_id: None,
        node_execution_id: None,
        node_name: None,
        attempt: Some(1),
        retry_predecessor_id: None,
        past_attempt_ids: Vec::new(),
        is_retry_history: false,
        completion_signals: Default::default(),
        has_artifact: false,
        session_id: None,
        can_rename: false,
        can_approve: true,
        can_retry: false,
        can_abort: false,
        can_archive: false,
        display_command: None,
        command_result: None,
        dynamic_fanout: false,
    }
}

fn tree_owner(execution_id: &str) -> WorkspaceTreeNode {
    let mut owner = node();
    owner.id = execution_id.to_string();
    owner.kind = WorkspaceNodeKind::Workflow;
    owner.activity = None;
    owner.title = "Workflow owner".to_string();
    owner.status = WorkspaceNodeStatus::Running;
    owner.execution_id = Some(execution_id.to_string());
    owner.node_execution_id = None;
    owner.node_name = None;
    owner.attempt = None;
    owner.can_approve = false;
    owner.can_abort = true;
    owner.can_archive = true;
    owner
}

fn tree_owner_with_title(execution_id: &str, title: &str) -> WorkspaceTreeNode {
    let mut owner = tree_owner(execution_id);
    owner.title = title.to_string();
    owner
}

fn child_node(
    id: &str,
    parent_id: &str,
    execution_id: &str,
    kind: WorkspaceNodeKind,
    title: &str,
) -> WorkspaceTreeNode {
    let mut child = node();
    child.id = id.to_string();
    child.parent_id = Some(parent_id.to_string());
    child.kind = kind;
    child.activity = (kind == WorkspaceNodeKind::WorkflowSession)
        .then(crate::domain::workflow::AgentSessionActivity::default);
    child.title = title.to_string();
    child.status = WorkspaceNodeStatus::Running;
    child.execution_id = Some(execution_id.to_string());
    child.node_execution_id = Some(format!("{id}-execution"));
    child.node_name = Some(title.to_string());
    child.can_approve = false;
    child
}

fn open_session(id: &str) -> AgentSessionItemDto {
    AgentSessionItemDto {
        id: id.to_string(),
        workspace_identity: "/repo".to_string(),
        worktree_path: "/repo".to_string(),
        workspace_worktree_path: "/repo".to_string(),
        provider: AgentSessionProviderDto::Codex,
        tree_location: crate::usecase::agent_session::AgentSessionTreeLocationDto {
            tree_id: id.to_string(),
            node_execution_id: id.to_string(),
        },
        lifecycle: AgentSessionLifecycleDto::Open,
        provider_session_id: None,
        transcript_ref: None,
        operations: AgentSessionOperationsDto {
            can_archive: true,
            can_restore: false,
            can_delete: false,
        },
        last_exit_abnormal: false,
    }
}

#[test]
fn sequence_and_fanout_are_distinct_recursive_branches_under_the_public_root() {
    let execution_id = "workflow-execution";
    let owner = tree_owner(execution_id);
    let sequence = child_node(
        "sequence",
        execution_id,
        execution_id,
        WorkspaceNodeKind::Sequence,
        "main",
    );
    let fanout = child_node(
        "fanout",
        "sequence",
        execution_id,
        WorkspaceNodeKind::Fanout,
        "reviews",
    );
    let command = child_node(
        "command",
        "fanout",
        execution_id,
        WorkspaceNodeKind::WorkflowCommand,
        "lint",
    );
    let tree = WorkspaceTree::restore("/repo", vec![owner, sequence, fanout, command]).unwrap();

    let json = serde_json::to_value(project_tree(
        &tree,
        &HashSet::new(),
        &HashSet::from([execution_id.to_string()]),
        &[],
    ))
    .unwrap();

    assert_eq!(json[0]["kind"], "sequence");
    assert_eq!(json[0]["id"], execution_id);
    assert_eq!(json[0]["status"], "active");
    assert!(json[0]["workflowCapabilities"].get("canStop").is_none());
    assert!(json[0]["workflowCapabilities"].get("canResume").is_none());
    assert_eq!(json[0]["workflowCapabilities"]["canAbort"], true);
    assert_eq!(json[0]["children"][0]["kind"], "fanout");
    assert_eq!(json[0]["children"][0]["status"], "active");
    assert_eq!(json[0]["children"][0]["children"][0]["kind"], "node");
    assert_eq!(json[0]["children"][0]["children"][0]["status"], "active");
}

#[test]
fn standalone_session_is_a_public_node_root_with_backend_lifecycle_capabilities() {
    let execution_id = "session-tree";
    let owner = tree_owner(execution_id);
    let mut session = child_node(
        "session-node",
        execution_id,
        execution_id,
        WorkspaceNodeKind::WorkflowSession,
        "Codex Session",
    );
    session.session_id = Some("session-ref".to_string());
    let tree = WorkspaceTree::restore("/repo", vec![owner, session]).unwrap();

    let json = serde_json::to_value(project_tree(
        &tree,
        &HashSet::new(),
        &HashSet::new(),
        &[open_session("session-ref")],
    ))
    .unwrap();

    assert_eq!(json[0]["kind"], "node");
    assert_eq!(json[0]["id"], execution_id);
    assert_eq!(json[0]["contentKind"], "session");
    assert_eq!(json[0]["sessionCapabilities"]["sessionRef"], "session-ref");
    assert_eq!(json[0]["sessionCapabilities"]["canArchive"], true);
    assert_eq!(json[0]["sessionCapabilities"]["canDelete"], false);
    assert_eq!(json[0]["workflowCapabilities"]["canArchive"], true);
    assert_eq!(json[0]["workflowCapabilities"]["canAbort"], true);
}

#[test]
fn leaf_workflow_root_keeps_workflow_capabilities_on_the_node() {
    let execution_id = "leaf-workflow";
    let owner = tree_owner(execution_id);
    let leaf = child_node(
        "leaf",
        execution_id,
        execution_id,
        WorkspaceNodeKind::WorkflowCommand,
        "main",
    );
    let tree = WorkspaceTree::restore("/repo", vec![owner, leaf]).unwrap();

    let json = serde_json::to_value(project_tree(
        &tree,
        &HashSet::new(),
        &HashSet::from([execution_id.to_string()]),
        &[],
    ))
    .unwrap();

    assert_eq!(json[0]["kind"], "node");
    assert_eq!(json[0]["id"], execution_id);
    assert!(json[0]["workflowCapabilities"].get("canStop").is_none());
    assert!(json[0]["workflowCapabilities"].get("canResume").is_none());
    assert_eq!(json[0]["workflowCapabilities"]["canAbort"], true);
}

fn assert_public_root_title(kind: WorkspaceNodeKind) {
    // Given
    let execution_id = "workflow-execution";
    let owner = tree_owner_with_title(execution_id, "01_author-spec");
    let public_root = child_node("root", execution_id, execution_id, kind, "main");
    let tree = WorkspaceTree::restore("/repo", vec![owner, public_root]).unwrap();

    // When
    let json = serde_json::to_value(project_tree(
        &tree,
        &HashSet::new(),
        &HashSet::from([execution_id.to_string()]),
        &[],
    ))
    .unwrap();

    // Then
    assert_eq!(json[0]["title"], "01_author-spec");
    assert_ne!(json[0]["title"], "main");
}

#[test]
fn test_workspaceツリー投影_sequenceのpublic_root表示名にworkflow名を返す() {
    assert_public_root_title(WorkspaceNodeKind::Sequence);
}

#[test]
fn test_workspaceツリー投影_fanoutのpublic_root表示名にworkflow名を返す() {
    assert_public_root_title(WorkspaceNodeKind::Fanout);
}

#[test]
fn test_workspaceツリー投影_sessionのpublic_root表示名にworkflow名を返す() {
    assert_public_root_title(WorkspaceNodeKind::WorkflowSession);
}

#[test]
fn test_workspaceツリー投影_commandのpublic_root表示名にworkflow名を返す() {
    assert_public_root_title(WorkspaceNodeKind::WorkflowCommand);
}

#[test]
fn test_workspaceツリー投影_異なるworkflow名の複数実行を表示名で判別できる() {
    // Given
    let execution_a = "execution-a";
    let execution_b = "execution-b";
    let owner_a = tree_owner_with_title(execution_a, "01_author-spec");
    let owner_b = tree_owner_with_title(execution_b, "03_full-review");
    let root_a = child_node(
        "root-a",
        execution_a,
        execution_a,
        WorkspaceNodeKind::Sequence,
        "main",
    );
    let root_b = child_node(
        "root-b",
        execution_b,
        execution_b,
        WorkspaceNodeKind::Sequence,
        "main",
    );
    let tree = WorkspaceTree::restore("/repo", vec![owner_a, root_a, owner_b, root_b]).unwrap();

    // When
    let json = serde_json::to_value(project_tree(
        &tree,
        &HashSet::new(),
        &HashSet::from([execution_a.to_string(), execution_b.to_string()]),
        &[],
    ))
    .unwrap();

    // Then
    let row_a = json
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == execution_a)
        .unwrap();
    let row_b = json
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == execution_b)
        .unwrap();
    assert_eq!(row_a["title"], "01_author-spec");
    assert_eq!(row_b["title"], "03_full-review");
    assert_ne!(row_a["title"], row_b["title"]);
}

#[test]
fn test_workspaceツリー投影_public_root以外はnode名を表示名に保つ() {
    // Given
    let execution_id = "workflow-execution";
    let owner = tree_owner_with_title(execution_id, "01_author-spec");
    let public_root = child_node(
        "root",
        execution_id,
        execution_id,
        WorkspaceNodeKind::Sequence,
        "main",
    );
    let child_sequence = child_node(
        "child-sequence",
        "root",
        execution_id,
        WorkspaceNodeKind::Sequence,
        "prepare",
    );
    let mut child_fanout = child_node(
        "child-fanout",
        "root",
        execution_id,
        WorkspaceNodeKind::Fanout,
        "reviews",
    );
    child_fanout.sibling_order = 1;
    let mut child_session = child_node(
        "child-session",
        "root",
        execution_id,
        WorkspaceNodeKind::WorkflowSession,
        "author",
    );
    child_session.sibling_order = 2;
    let mut child_command = child_node(
        "child-command",
        "root",
        execution_id,
        WorkspaceNodeKind::WorkflowCommand,
        "lint",
    );
    child_command.sibling_order = 3;
    let tree = WorkspaceTree::restore(
        "/repo",
        vec![
            owner,
            public_root,
            child_sequence,
            child_fanout,
            child_session,
            child_command,
        ],
    )
    .unwrap();

    // When
    let json = serde_json::to_value(project_tree(
        &tree,
        &HashSet::new(),
        &HashSet::from([execution_id.to_string()]),
        &[],
    ))
    .unwrap();

    // Then
    assert_eq!(json[0]["title"], "01_author-spec");
    assert_eq!(json[0]["children"][0]["title"], "prepare");
    assert_eq!(json[0]["children"][1]["title"], "reviews");
    assert_eq!(json[0]["children"][2]["title"], "author");
    assert_eq!(json[0]["children"][3]["title"], "lint");
}

#[test]
fn test_workflow_session_node_detail_session_surfaceを公開する() {
    let mut workflow_session = node();
    workflow_session.session_id = Some("agent-session-1".to_string());

    let detail = serde_json::to_value(node_detail(workflow_session)).unwrap();

    assert_eq!(
        detail["content"]["kind"],
        serde_json::Value::String("session".to_string())
    );
    assert_eq!(
        detail["content"]["sessionId"],
        serde_json::Value::String("agent-session-1".to_string())
    );
}

#[test]
fn workflow_node_detail_exposes_backend_owned_signal_and_capabilities_without_attempt() {
    let mut workflow_session = node();
    workflow_session.node_execution_id = Some("node-execution-1".to_string());
    workflow_session.execution_id = Some("execution-1".to_string());
    workflow_session.node_name = Some("Review".to_string());
    workflow_session.session_id = Some("agent-session-1".to_string());
    workflow_session.completion_signals =
        crate::domain::workflow::NodeCompletionSignalState::StopReceived;
    workflow_session.has_artifact = false;
    workflow_session.can_retry = true;
    workflow_session.can_rename = true;

    let detail = serde_json::to_value(node_detail(workflow_session)).unwrap();

    assert!(detail.get("attempt").is_none());
    assert_eq!(detail["submitReceived"], false);
    assert_eq!(detail["stopReceived"], true);
    assert_eq!(detail["waitingFor"], "submit");
    assert_eq!(detail["hasArtifact"], false);
    assert_eq!(detail["capabilities"]["canRetry"], true);
    assert_eq!(detail["capabilities"]["canRename"], true);
}

#[test]
fn test_workspaceツリー契約_nodeとsequenceとfanoutは4分類だけを返す() {
    // Given
    let execution_id = "workflow-execution";
    let owner = tree_owner(execution_id);
    let sequence = child_node(
        "sequence",
        execution_id,
        execution_id,
        WorkspaceNodeKind::Sequence,
        "main",
    );
    let fanout = child_node(
        "fanout",
        "sequence",
        execution_id,
        WorkspaceNodeKind::Fanout,
        "reviews",
    );
    let mut failed = child_node(
        "failed",
        "fanout",
        execution_id,
        WorkspaceNodeKind::WorkflowCommand,
        "lint",
    );
    failed.process_presence = crate::domain::workflow::NodeProcessPresence::ConfirmedAbsent;
    let tree = WorkspaceTree::restore("/repo", vec![owner, sequence, fanout, failed]).unwrap();

    // When
    let json = serde_json::to_value(project_tree(
        &tree,
        &HashSet::new(),
        &HashSet::from([execution_id.to_string()]),
        &[],
    ))
    .unwrap();

    // Then
    for status in [
        &json[0]["status"],
        &json[0]["children"][0]["status"],
        &json[0]["children"][0]["children"][0]["status"],
    ] {
        assert_eq!(status, "attention");
        assert!(![
            "running",
            "paused",
            "failed",
            "waiting",
            "aborted",
            "completed",
            "interrupted",
        ]
        .contains(&status.as_str().unwrap()));
    }
}

#[test]
fn test_workspaceノード詳細契約_詳細状態と5分類を同時に返す() {
    // Given
    let cases = [
        (
            WorkspaceNodeStatus::Running,
            WorkspaceNodeStatusClassification::Active,
            "running",
            "active",
        ),
        (
            WorkspaceNodeStatus::Waiting,
            WorkspaceNodeStatusClassification::Attention,
            "waiting",
            "attention",
        ),
        (
            WorkspaceNodeStatus::Aborted,
            WorkspaceNodeStatusClassification::Idle,
            "aborted",
            "idle",
        ),
        (
            WorkspaceNodeStatus::Completed,
            WorkspaceNodeStatusClassification::Idle,
            "completed",
            "idle",
        ),
        (
            WorkspaceNodeStatus::Running,
            WorkspaceNodeStatusClassification::Unbound,
            "running",
            "unbound",
        ),
    ];

    // When / Then
    for (status, classification, expected_status, expected_classification) in cases {
        let mut current = node();
        current.status = status;
        current.status_classification = classification;
        let detail = serde_json::to_value(node_detail(current)).unwrap();
        assert_eq!(detail["status"], expected_status);
        assert_eq!(detail["statusClassification"], expected_classification);
        assert_ne!(detail["status"], "interrupted");
        assert_ne!(detail["statusClassification"], "interrupted");
    }
}

#[test]
fn test_workspaceツリー契約_can_renameとunboundをdomain_nodeからそのまま公開する() {
    let execution_id = "workflow-execution";
    let owner = tree_owner(execution_id);
    let mut session = child_node(
        "session",
        execution_id,
        execution_id,
        WorkspaceNodeKind::WorkflowSession,
        "session",
    );
    session.status_classification = WorkspaceNodeStatusClassification::Unbound;
    session.can_rename = true;
    let tree = WorkspaceTree::restore("/repo", vec![owner, session]).unwrap();

    let json = serde_json::to_value(project_tree(
        &tree,
        &HashSet::new(),
        &HashSet::from([execution_id.to_string()]),
        &[],
    ))
    .unwrap();

    assert_eq!(json[0]["status"], "unbound");
    assert_eq!(json[0]["capabilities"]["canRename"], false);
    assert!(json[0].get("workflowCapabilities").is_some());
}

#[test]
fn test_workspaceツリー契約_completedでも終了時capabilityを維持する() {
    // Given
    let execution_id = "completed-workflow-execution";
    let mut owner = tree_owner(execution_id);
    owner.status = WorkspaceNodeStatus::Completed;
    owner.can_abort = false;
    owner.can_archive = true;
    let mut completed = child_node(
        "completed",
        execution_id,
        execution_id,
        WorkspaceNodeKind::WorkflowCommand,
        "completed",
    );
    completed.status = WorkspaceNodeStatus::Completed;
    let tree = WorkspaceTree::restore("/repo", vec![owner, completed]).unwrap();

    // When
    let json = serde_json::to_value(project_tree(
        &tree,
        &HashSet::new(),
        &HashSet::from([execution_id.to_string()]),
        &[],
    ))
    .unwrap();

    // Then
    assert_eq!(json[0]["status"], "idle");
    assert!(json[0]["workflowCapabilities"].get("canStop").is_none());
    assert!(json[0]["workflowCapabilities"].get("canResume").is_none());
    assert_eq!(json[0]["workflowCapabilities"]["canAbort"], false);
    assert_eq!(json[0]["workflowCapabilities"]["canArchive"], true);
}

#[test]
fn execution_summary_rejects_non_finite_timestamp() {
    let record = WorkflowExecutionMetadataRecord {
        execution_id: "execution".to_string(),
        workflow_name: "workflow".to_string(),
        status: ExecutionStatus::Running,
        worktree_path: "/repo".to_string(),
        current_node: None,
        created_from: ExecutionOrigin::DesktopUi,
        started_at_bits: f64::NAN.to_bits(),
        updated_at_bits: 1.0f64.to_bits(),
        completed_at_bits: None,
        error_reason: None,
        total_token_usage: TokenUsage::default(),
    };
    assert!(matches!(
        execution_summary(record),
        Err(WorkflowError::CorruptStoredState(_))
    ));
}

#[test]
fn test_workspace_query_error_corruptをcorrupt_stored_stateへ写像する() {
    // Given
    let correlation_id = "workspace-corrupt-correlation";

    // When
    let error = query_error(crate::domain::local_event::LocalEventQueryError::Corrupt {
        correlation_id: correlation_id.to_string(),
    });

    // Then
    assert_eq!(
        error,
        WorkflowError::CorruptStoredState(format!(
            "store corrupt (correlation_id={correlation_id})"
        ))
    );
}

#[test]
fn unrepresentable_page_offset_falls_back_to_the_first_record() {
    assert_eq!(
        sqlite_page_bounds(Some(WorkflowPageRequest::new(usize::MAX, usize::MAX))),
        (i64::MAX, 0)
    );
    assert_eq!(sqlite_page_bounds(None), (i64::MAX, 0));
}

#[tokio::test]
async fn test_workspace読取_未対応定義がabort済みでもcommand出力とsession参照を取得できる() {
    use crate::domain::workspace_tree::WorkspaceTreeRepository;
    // Given
    for unavailable in ["main", "command", "session", "unused"] {
        let directory = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into()))
                .unwrap();
        crate::adaptor::gateway::workflow::test_support::seed_unavailable_definition(
            &store,
            "00000000-0000-4000-8000-000000001744",
            "/repo",
            unavailable,
        )
        .await;
        crate::adaptor::gateway::workflow::test_support::seed_unavailable_definition(
            &store,
            "00000000-0000-4000-8000-000000001745",
            "/other",
            "main",
        )
        .await;
        for tree in [
            "00000000-0000-4000-8000-000000001744",
            "00000000-0000-4000-8000-000000001745",
        ] {
            crate::adaptor::gateway::workflow::fact_log::append_single_fact(
                &store,
                &crate::domain::workflow::NodeFactMeta {
                    tree_id: tree.into(),
                    node_execution_id: tree.into(),
                    parent_id: None,
                    node_name: "main".into(),
                    kind: crate::domain::workflow::NodeKindName::Sequence,
                    attempt: 1,
                },
                &crate::domain::workflow::NodeFact::AbortRequested(Default::default()),
                10_000,
            )
            .await
            .unwrap();
        }
        let read_store =
            crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore::open(
                directory.path(),
            )
            .unwrap();
        for repository in [
            SqliteWorkspaceTreeRepository::new(store.clone()),
            SqliteWorkspaceTreeRepository::new_read_only(read_store),
        ] {
            let command = repository
                .load_node_by_node_execution_id("00000000-0000-4000-8000-000000001744-command")
                .await
                .unwrap()
                .unwrap();
            let session = repository
                .load_node_by_node_execution_id("00000000-0000-4000-8000-000000001744-session")
                .await
                .unwrap()
                .unwrap();
            let query = SqliteWorkspaceQueryService::with_repository(
                crate::usecase::work_queue::shared().clone(),
                repository,
                Arc::new(EmptyArchives),
            );
            let workspace = WorkspaceIdentity::new("/repo");

            // When
            let snapshot = query.workspace_tree(&workspace).await.unwrap();
            let command_detail = query
                .node_detail(&workspace, &command.id)
                .await
                .unwrap()
                .unwrap();
            let session_detail = query
                .node_detail(&workspace, &session.id)
                .await
                .unwrap()
                .unwrap();

            // Then
            assert!(!snapshot.nodes.is_empty());
            let WorkspaceNodeContentDto::Command(content) = command_detail.content else {
                panic!("command content must remain available")
            };
            assert_eq!(content.display_command.as_deref(), Some("printf kept"));
            assert_eq!(content.result.unwrap().stdout, "kept");
            let WorkspaceNodeContentDto::Session(content) = session_detail.content else {
                panic!("session reference must remain available")
            };
            assert_eq!(
                content.session_id.as_deref(),
                Some("00000000-0000-4000-8000-000000001744-session")
            );
            assert_eq!(session_detail.status, "aborted");
            assert!(!command_detail.capabilities.can_retry);
            assert!(!session_detail.capabilities.can_resume_session);
        }
    }
}

#[test]
fn test_隔離node詳細_実行中と成果物なし終端でもbranchとpathを公開する() {
    // Given
    let expected = crate::domain::workflow::IsolatedWorktree::for_attempt("/repo", "isolated", 2);
    for status in [WorkspaceNodeStatus::Running, WorkspaceNodeStatus::Aborted] {
        let mut node = child_node(
            "isolated",
            "execution",
            "execution",
            WorkspaceNodeKind::WorkflowSession,
            "work",
        );
        node.status = status;
        node.worktree = Some(expected.clone());
        // When
        let detail = serde_json::to_value(node_detail(node)).unwrap();
        // Then
        assert_eq!(detail["worktree"]["branch"], expected.branch);
        assert_eq!(detail["worktree"]["path"], expected.path);
        assert_eq!(detail["status"], status.as_public_str());
        assert_eq!(detail["hasArtifact"], false);
        assert_eq!(detail["recoveryReason"], serde_json::Value::Null);
    }
}

#[test]
fn test_隔離合成子の表示_空のchildrenや終端でもそのattemptのbranchとpathを返す() {
    for kind in [WorkspaceNodeKind::Sequence, WorkspaceNodeKind::Fanout] {
        for status in [
            WorkspaceNodeStatus::Running,
            WorkspaceNodeStatus::Completed,
            WorkspaceNodeStatus::Aborted,
        ] {
            // Given
            let owner = tree_owner("tree");
            let mut composite = child_node("composite", "tree", "tree", kind, "main");
            let expected = crate::domain::workflow::IsolatedWorktree::for_attempt(
                "/repo",
                "composite-execution",
                2,
            );
            composite.attempt = Some(2);
            composite.status = status;
            composite.worktree = Some(expected.clone());
            let tree = WorkspaceTree::restore("/repo", vec![owner, composite]).unwrap();
            // When
            let items = serde_json::to_value(project_tree(
                &tree,
                &HashSet::new(),
                &HashSet::from(["tree".into()]),
                &[],
            ))
            .unwrap();
            // Then
            assert_eq!(items[0]["worktree"]["branch"], expected.branch);
            assert_eq!(items[0]["worktree"]["path"], expected.path);
            assert!(items[0]["children"].as_array().unwrap().is_empty());
        }
    }
}

#[test]
fn test_delegate_親session配下に発火順の子を表示し子の部分木も保持する() {
    // Given
    let owner = tree_owner("tree");
    let parent = child_node(
        "parent",
        "tree",
        "tree",
        WorkspaceNodeKind::WorkflowSession,
        "implement",
    );
    let mut first = child_node(
        "first",
        "parent",
        "tree",
        WorkspaceNodeKind::WorkflowSession,
        "verify",
    );
    first.sibling_order = 0;
    first.attempt = Some(1);
    first.status = WorkspaceNodeStatus::Completed;
    let mut second = child_node(
        "second",
        "parent",
        "tree",
        WorkspaceNodeKind::Sequence,
        "checks",
    );
    second.sibling_order = 1;
    second.attempt = Some(2);
    let nested = child_node(
        "nested",
        "second",
        "tree",
        WorkspaceNodeKind::WorkflowCommand,
        "check",
    );
    let tree = WorkspaceTree::restore("/repo", vec![owner, parent, first, second, nested]).unwrap();
    // When
    let result = project_tree(&tree, &HashSet::new(), &HashSet::from(["tree".into()]), &[]);
    let json = serde_json::to_value(result).unwrap();
    // Then
    assert_eq!(json[0]["kind"], "node");
    assert_eq!(json[0]["children"][0]["title"], "verify");
    assert_eq!(json[0]["children"][1]["kind"], "sequence");
    assert_eq!(json[0]["children"][1]["children"][0]["title"], "check");
}

#[test]
fn test_delegate_親の過去attemptに当該attemptのchild部分木を投影する() {
    // Given
    let owner = tree_owner("tree");
    let mut past = child_node(
        "past",
        "tree",
        "tree",
        WorkspaceNodeKind::WorkflowSession,
        "implement",
    );
    past.attempt = Some(1);
    past.status = WorkspaceNodeStatus::Aborted;
    let mut current = child_node(
        "current",
        "tree",
        "tree",
        WorkspaceNodeKind::WorkflowSession,
        "implement",
    );
    current.attempt = Some(2);
    current.sibling_order = 1;
    current.retry_predecessor_id = Some("past-execution".into());
    let prior_child = child_node(
        "prior-checks",
        "past",
        "tree",
        WorkspaceNodeKind::Sequence,
        "checks",
    );
    let prior_leaf = child_node(
        "prior-judge",
        "prior-checks",
        "tree",
        WorkspaceNodeKind::WorkflowSession,
        "judge",
    );
    let current_child = child_node(
        "current-checks",
        "current",
        "tree",
        WorkspaceNodeKind::WorkflowSession,
        "checks",
    );
    let tree = WorkspaceTree::restore(
        "/repo",
        vec![owner, past, current, prior_child, prior_leaf, current_child],
    )
    .unwrap();
    // When
    let result = serde_json::to_value(project_tree(
        &tree,
        &HashSet::new(),
        &HashSet::from(["tree".into()]),
        &[],
    ))
    .unwrap();
    // Then
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result[0]["children"][0]["id"], "current-checks");
    assert_eq!(result[0]["pastAttempts"][0]["id"], "past");
    assert_eq!(result[0]["pastAttempts"][0]["kind"], "node");
    assert_eq!(result[0]["pastAttempts"][0]["contentKind"], "session");
    assert_eq!(
        result[0]["pastAttempts"][0]["children"][0]["id"],
        "prior-checks"
    );
    assert_eq!(
        result[0]["pastAttempts"][0]["children"][0]["children"][0]["id"],
        "prior-judge"
    );
}

#[test]
fn test_過去attempt_子のないsessionとcommandも通常行と同じkindを返す() {
    for (kind, content_kind) in [
        (WorkspaceNodeKind::WorkflowSession, "session"),
        (WorkspaceNodeKind::WorkflowCommand, "command"),
    ] {
        // Given
        let owner = tree_owner("tree");
        let mut past = child_node("past", "tree", "tree", kind, "work");
        past.attempt = Some(1);
        past.status = WorkspaceNodeStatus::Aborted;
        let mut current = child_node("current", "tree", "tree", kind, "work");
        current.attempt = Some(2);
        current.sibling_order = 1;
        current.retry_predecessor_id = Some("past-execution".into());
        let tree = WorkspaceTree::restore("/repo", vec![owner, past, current]).unwrap();
        // When
        let result = serde_json::to_value(project_tree(
            &tree,
            &HashSet::new(),
            &HashSet::from(["tree".into()]),
            &[],
        ))
        .unwrap();
        // Then
        let past = &result[0]["pastAttempts"][0];
        assert_eq!(past["id"], "past");
        assert_eq!(past["kind"], "node");
        assert_eq!(past["contentKind"], content_kind);
        assert!(past.get("children").is_none());
        assert_eq!(past["pastAttempts"], serde_json::json!([]));
    }
}

#[tokio::test]
async fn test_archive履歴_手動とworktree消失の事実の時刻と理由をそのまま投影する() {
    use crate::adaptor::gateway::workflow::{fact_log, ExecutionTreeArchiveFactRepository};
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
    let archives = Arc::new(ExecutionTreeArchiveFactRepository::new(
        store.clone(),
        directory.path(),
    ));
    let query = SqliteWorkspaceQueryService::with_repository(
        crate::usecase::work_queue::shared().clone(),
        SqliteWorkspaceTreeRepository::new(store.clone()),
        archives.clone(),
    );
    let fixtures = [
        ("00000000-0000-4000-8000-000000000901", 12.345678, "manual"),
        (
            "00000000-0000-4000-8000-000000000902",
            98.765432,
            "worktree_removed",
        ),
    ];
    for (id, at, reason) in fixtures {
        seed_workflow_session_facts(
            &store,
            WorkflowSessionFactSeed {
                workflow_name: "history",
                request: "test",
                worktree_path: "/repo",
                provider: ProviderKind::Codex,
                workflow_execution_id: id,
                node_execution_id: &format!("node-{id}"),
                session_id: &format!("session-{id}"),
                initial_instruction_admitted: true,
            },
        )
        .await
        .unwrap();
        let root = fact_log::read_tree_records(&store, id)
            .await
            .unwrap()
            .remove(0);
        fact_log::append_single_fact(
            &store,
            &root.meta,
            &NodeFact::AbortRequested(Default::default()),
            2,
        )
        .await
        .unwrap();
        archives
            .archive(
                &crate::domain::workflow::ExecutionTreeId::new(id).unwrap(),
                at,
                reason,
            )
            .await
            .unwrap();
    }
    // When
    let history = query
        .workflow_history(&WorkspaceIdentity::new("/repo"))
        .await
        .unwrap();
    // Then
    assert_eq!(history.len(), 2);
    for (id, at, reason) in fixtures {
        let item = history.iter().find(|item| item.execution_id == id).unwrap();
        assert_eq!(item.archived_at, at);
        assert_eq!(item.archive_reason, reason);
        assert_eq!(item.status, "aborted");
    }
    assert_eq!(history[0].execution_id, fixtures[1].0);
}

#[tokio::test]
async fn test_workflow単一取得_単独sessionをworkflow_summaryとして返さない() {
    use crate::domain::workflow::SessionExecutionTreeRootFacts;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
    let session = "00000000-0000-4000-8000-000000000991";
    let workflow = "00000000-0000-4000-8000-000000000992";
    let facts =
        SessionExecutionTreeRootFacts::new(session, "/repo", "/repo", ProviderKind::Codex, None)
            .unwrap();
    crate::adaptor::gateway::workflow::fact_log::append_fact_batch_for_seed(
        &store,
        &facts.into_facts(),
        1,
        session,
    )
    .unwrap();
    seed_workflow_session_facts(
        &store,
        WorkflowSessionFactSeed {
            workflow_name: "workflow",
            request: "test",
            worktree_path: "/repo",
            provider: ProviderKind::Codex,
            workflow_execution_id: workflow,
            node_execution_id: "node",
            session_id: "workflow-session",
            initial_instruction_admitted: true,
        },
    )
    .await
    .unwrap();
    let query = SqliteWorkspaceQueryService::with_repository(
        crate::usecase::work_queue::shared().clone(),
        SqliteWorkspaceTreeRepository::new(store),
        Arc::new(EmptyArchives),
    );
    // When / Then
    assert!(query.execution_summary(session).await.unwrap().is_none());
    assert_eq!(
        query
            .execution_summary(workflow)
            .await
            .unwrap()
            .unwrap()
            .execution_id,
        workflow
    );
}

#[test]
fn test_workspace_query_結果不明と期限切れの分類を保持する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use crate::domain::local_event::LocalEventQueryError;
    use connectrpc::ErrorCode;
    // Given
    for (error, expected) in [
        (
            LocalEventQueryError::CanonicalWriterRequired,
            ErrorCode::Internal,
        ),
        (
            LocalEventQueryError::Technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                message: "deadline exceeded".into(),
            }),
            ErrorCode::DeadlineExceeded,
        ),
        (LocalEventQueryError::QueryBusy, ErrorCode::Unavailable),
    ] {
        // When / Then
        assert_eq!(query_error(error).connect_code(), expected);
    }
}

#[test]
fn test_workspace_query_store以外の失敗はmainと同じ変種を返す() {
    use crate::adaptor::controller::api::error::ApiError;
    use crate::adaptor::presenter::connect::ConnectFailure;
    use crate::domain::local_event::{
        LocalEventQueryError, SafeOperationFailure, SessionOperationFailureKind,
    };

    for error in [
        LocalEventQueryError::CanonicalWriterRequired,
        LocalEventQueryError::StorageAccessRequired {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::StorageUnavailable,
                TechnicalFailureNature::Other,
                "access required",
                "id",
            ),
        },
    ] {
        let error = query_error(error);
        assert!(matches!(error, WorkflowError::External(_)));
        assert_eq!(error.connect_code(), connectrpc::ErrorCode::Internal);
        assert_eq!(ApiError::from(error).status.as_u16(), 500);
    }
}

#[tokio::test]
async fn test_workspace読取_実経路で失敗分類を保持する() {
    use crate::adaptor::gateway::local_event_store::test_helpers::ReadFailure;
    use crate::adaptor::presenter::connect::classified_error;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store.clone());
    let query = SqliteWorkspaceQueryService::with_repository(
        crate::usecase::work_queue::shared().clone(),
        repository,
        Arc::new(EmptyArchives),
    );
    let workspace = WorkspaceIdentity::new("/repo");
    for (failure, expected) in ReadFailure::cases() {
        for tree in [false, true] {
            let expected =
                if tree && matches!(failure, ReadFailure::Sqlite(rusqlite::ffi::SQLITE_IOERR)) {
                    connectrpc::ErrorCode::Internal
                } else {
                    expected
                };
            store.fail_next_read(failure.clone());
            // When
            let result = if tree {
                query.workspace_tree(&workspace).await.map(|_| ())
            } else {
                query.execution_records(None, None, None).await.map(|_| ())
            };
            // Then
            assert_eq!(classified_error(result.unwrap_err()).code, expected);
        }
    }
}

#[test]
fn test_store問い合わせエラー_停止の分類を保持する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use crate::common::operation_context::OperationStopped;
    // Given
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        // When
        let error = query_error(crate::domain::local_event::LocalEventQueryError::Technical(
            stopped.into(),
        ));
        // Then
        assert_eq!(
            error.connect_code(),
            crate::domain::failure::TechnicalFailure::from(stopped).connect_code()
        );
        assert!(matches!(error, WorkflowError::Technical(value) if value == stopped.into()));
    }
}

#[tokio::test]
async fn test_workspaceツリー投影_背景失敗の対象と理由を表示し成功後は解除する() {
    use crate::common::retry::RetryBackoff;
    use crate::usecase::work_queue::{work_queue_tests::queue, WorkFailure, WorkKey};

    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let execution_id = "00000000-0000-4000-8000-000000001701";
    seed_workflow_session_facts(
        &store,
        WorkflowSessionFactSeed {
            workflow_name: "background-failure",
            request: "test",
            worktree_path: "/repo",
            provider: ProviderKind::Codex,
            workflow_execution_id: execution_id,
            node_execution_id: "workflow-node",
            session_id: "workflow-session",
            initial_instruction_admitted: true,
        },
    )
    .await
    .unwrap();
    AgentSessionUsecase::new(Arc::new(LocalAgentSessionRepository::new(store.clone())))
        .observe_activity(
            "workflow-session",
            AgentSessionActivity::Working,
            "observe-working",
        )
        .await
        .unwrap();
    let queue = queue();
    let query = SqliteWorkspaceQueryService::with_repository(
        queue.clone(),
        SqliteWorkspaceTreeRepository::new(store),
        Arc::new(EmptyArchives),
    );
    let workspace = WorkspaceIdentity::new("/repo");
    let before = query.workspace_tree(&workspace).await.unwrap();
    let WorkspaceTreeItemDto::Sequence(root) = &before.nodes[0] else {
        panic!("workflow root");
    };
    let WorkspaceTreeItemDto::Node(node) = &root.children[0] else {
        panic!("workflow node");
    };
    assert_eq!(node.status, "active");
    assert!(node.error_reason.is_none());
    for target in [node.id.as_str(), "workflow-node", execution_id] {
        let key = WorkKey::new("workflow_recovery", target);
        // When / Then
        queue
            .observe(
                &key,
                &WorkFailure {
                    kind: Failure::Technical(TechnicalFailureNature::Cancelled),
                    message: "cancelled".into(),
                },
            )
            .await;
        assert_eq!(query.workspace_tree(&workspace).await.unwrap(), before);
        queue
            .observe(
                &key,
                &WorkFailure {
                    kind: Failure::Business(BusinessFailure::Other),
                    message: "repair required".into(),
                },
            )
            .await;
        let snapshot = query.workspace_tree(&workspace).await.unwrap();
        let WorkspaceTreeItemDto::Sequence(root) = &snapshot.nodes[0] else {
            panic!("workflow root");
        };
        let WorkspaceTreeItemDto::Node(node) = &root.children[0] else {
            panic!("workflow node");
        };
        assert_eq!(root.status, "attention");
        assert_eq!(node.status, "attention");
        assert_eq!(node.error_reason.as_deref(), Some("repair required"));
        queue
            .execute(key, RetryBackoff::RECOVERY, |_| async { Ok(()) })
            .await
            .unwrap();
        assert_eq!(query.workspace_tree(&workspace).await.unwrap(), before);
    }
}

#[tokio::test]
async fn test_失敗対象の読取_node行以外はstoreに依存せず未知の行はそのまま返す() {
    use crate::adaptor::gateway::local_event_store::test_helpers::ReadFailure;
    use crate::adaptor::presenter::connect::classified_error;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
    let query = SqliteWorkspaceQueryService::with_repository(
        crate::usecase::work_queue::shared().clone(),
        SqliteWorkspaceTreeRepository::new(store.clone()),
        Arc::new(EmptyArchives),
    );
    let missing = "node-w-00000000000040008000000000001932-missing";
    // When / Then
    for (failure, expected) in ReadFailure::cases() {
        let expected = if matches!(failure, ReadFailure::Sqlite(rusqlite::ffi::SQLITE_IOERR)) {
            connectrpc::ErrorCode::Internal
        } else {
            expected
        };
        store.fail_next_read(failure);
        for target in [
            "*",
            "/repo",
            "startup-completion-probe",
            "node-w-invalid-missing",
        ] {
            assert_eq!(query.failure_targets(target).await.unwrap(), [target]);
        }
        assert_eq!(
            classified_error(query.failure_targets(missing).await.unwrap_err()).code,
            expected
        );
    }
    assert_eq!(query.failure_targets(missing).await.unwrap(), [missing]);
}
