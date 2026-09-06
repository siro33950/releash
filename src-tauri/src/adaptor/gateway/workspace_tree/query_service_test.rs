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
use crate::domain::local_event::WorkflowExecutionMetadataRecord;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::{
    AgentSessionActivity, ExecutionOrigin, ExecutionStatus, NodeFact, StopReceivedFact, TokenUsage,
    WorkflowExecutionArchiveSnapshot, WorkflowExecutionId, WorkflowExecutionManualArchiveRecord,
};
use crate::domain::workspace_tree::{
    WorkspaceNodeStatus, WorkspaceNodeStatusClassification, WorkspaceTreeNode,
};
use crate::usecase::agent_session::{
    AgentSessionOperationsDto, AgentSessionProviderDto, AgentSessionUsecase,
};

struct EmptyArchives;

impl WorkflowExecutionArchiveRepository for EmptyArchives {
    fn archive_manual(
        &self,
        _execution_id: &WorkflowExecutionId,
        _archived_at: f64,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }

    fn restore_manual(
        &self,
        _execution_id: &WorkflowExecutionId,
        _restored_at: f64,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }

    fn manual_archive_snapshot_for(
        &self,
        _execution_ids: &[String],
    ) -> Result<WorkflowExecutionArchiveSnapshot, WorkflowError> {
        Ok(WorkflowExecutionArchiveSnapshot {
            records: Vec::new(),
        })
    }
}

struct ArchivedExecution {
    execution_id: String,
    archived_at: f64,
}

impl WorkflowExecutionArchiveRepository for ArchivedExecution {
    fn archive_manual(
        &self,
        _execution_id: &WorkflowExecutionId,
        _archived_at: f64,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }

    fn restore_manual(
        &self,
        _execution_id: &WorkflowExecutionId,
        _restored_at: f64,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }

    fn manual_archive_snapshot_for(
        &self,
        execution_ids: &[String],
    ) -> Result<WorkflowExecutionArchiveSnapshot, WorkflowError> {
        let records = execution_ids
            .contains(&self.execution_id)
            .then(|| WorkflowExecutionManualArchiveRecord {
                execution_id: self.execution_id.clone(),
                archived_at: self.archived_at,
            })
            .into_iter()
            .collect();
        Ok(WorkflowExecutionArchiveSnapshot { records })
    }
}

fn assert_working_session_projection(store: Arc<LocalEventStore>) {
    let workspace = WorkspaceIdentity::new("workspace-activity-read");
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query = SqliteWorkspaceQueryService::with_repository(repository, Arc::new(EmptyArchives));

    let snapshot = query.workspace_tree(&workspace).unwrap();
    let WorkspaceTreeItemDto::Node(node) = &snapshot.nodes[0] else {
        panic!("standalone Session must be projected as a node");
    };
    assert_eq!(node.status, "active");
    let detail = query
        .node_detail(&workspace, &node.id)
        .unwrap()
        .expect("Session detail must exist");
    assert_eq!(detail.status, "running");
    assert_eq!(detail.status_classification, "active");
    let snapshot_json = serde_json::to_string(&snapshot).unwrap();
    let detail_json = serde_json::to_string(&detail).unwrap();
    assert!(!snapshot_json.contains("\"activity\":"));
    assert!(!detail_json.contains("\"activity\":"));
}

fn assert_workflow_child_activity_projection(
    store: Arc<LocalEventStore>,
    expected_classification: &str,
) {
    let workspace = WorkspaceIdentity::new("/repo/workflow-child-activity");
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query = SqliteWorkspaceQueryService::with_repository(repository, Arc::new(EmptyArchives));

    let snapshot = query.workspace_tree(&workspace).unwrap();
    let WorkspaceTreeItemDto::Sequence(sequence) = &snapshot.nodes[0] else {
        panic!("workflow root must be projected as a sequence");
    };
    let WorkspaceTreeItemDto::Node(node) = &sequence.children[0] else {
        panic!("workflow child Session must be projected as a node");
    };
    assert_eq!(node.status, expected_classification);
    let detail = query
        .node_detail(&workspace, &node.id)
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

    assert_working_session_projection(store.clone());
    drop(store);

    let reopened = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    assert_working_session_projection(reopened);
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
        crate::adaptor::gateway::workflow::fact_log::read_tree_records(&store, session_id).unwrap();
    assert!(!records
        .iter()
        .any(|record| matches!(record.fact, NodeFact::AgentActivityObserved(_))));

    // When: Workspace query service から一覧と詳細を読む
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query = SqliteWorkspaceQueryService::with_repository(repository, Arc::new(EmptyArchives));
    let snapshot = query.workspace_tree(&workspace).unwrap();
    let WorkspaceTreeItemDto::Node(node) = &snapshot.nodes[0] else {
        panic!("standalone Session must be projected as a node");
    };
    let detail = query
        .node_detail(&workspace, &node.id)
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
    let query = SqliteWorkspaceQueryService::with_repository(repository, Arc::new(EmptyArchives));
    let snapshot = query.workspace_tree(&workspace).unwrap();
    let WorkspaceTreeItemDto::Node(node) = &snapshot.nodes[0] else {
        panic!("standalone Session must be projected as a node");
    };
    let detail = query
        .node_detail(&workspace, &node.id)
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

    assert_workflow_child_activity_projection(store.clone(), "active");
    drop(store);

    let reopened = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    assert_workflow_child_activity_projection(reopened.clone(), "active");

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
    assert_workflow_child_activity_projection(reopened, "attention");
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
    let query = SqliteWorkspaceQueryService::with_repository(repository, Arc::new(EmptyArchives));
    let projected_statuses = || {
        let snapshot = query.workspace_tree(&workspace).unwrap();
        let WorkspaceTreeItemDto::Node(node) = &snapshot.nodes[0] else {
            panic!("standalone Session must be projected as a node");
        };
        let detail = query
            .node_detail(&workspace, &node.id)
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
        let (tree_status, detail_status, detail_classification) = projected_statuses();
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
        .unwrap()
        .last()
        .unwrap()
        .meta
        .clone();
    let append_stop = |timestamp_ms| {
        crate::adaptor::gateway::workflow::fact_log::append_single_fact(
            &store,
            &meta,
            &NodeFact::StopReceived(StopReceivedFact {
                result_summary: None,
                token_usage: None,
            }),
            timestamp_ms,
        )
        .unwrap();
    };
    let projected_classification = |store: Arc<LocalEventStore>| {
        let repository = SqliteWorkspaceTreeRepository::new(store);
        let query =
            SqliteWorkspaceQueryService::with_repository(repository, Arc::new(EmptyArchives));
        let snapshot = query.workspace_tree(&workspace).unwrap();
        let WorkspaceTreeItemDto::Node(node) = &snapshot.nodes[0] else {
            panic!("standalone Session must be projected as a node");
        };
        let detail = query
            .node_detail(&workspace, &node.id)
            .unwrap()
            .expect("Session detail must exist");
        assert_eq!(detail.status, "running");
        (node.status.clone(), detail.status_classification)
    };

    // When: 活動観測を追加せず StopReceived だけを追記する
    append_stop(10);

    // Then: 一覧と詳細は Stop 事実から attention を導出し、再起動後も再現する
    assert_eq!(
        projected_classification(store.clone()),
        ("attention".to_string(), "attention".to_string())
    );
    drop(sessions);
    drop(store);
    let reopened = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    assert_eq!(
        projected_classification(reopened.clone()),
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
            projected_classification(reopened.clone()),
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
            .unwrap();
            assert_eq!(
                projected_classification(reopened.clone()),
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
    let query =
        SqliteWorkspaceQueryService::with_repository(repository.clone(), Arc::new(EmptyArchives));

    let workflow_ids = query
        .execution_summaries(Some(&WorkspaceIdentity::new("/repo")), None, None)
        .unwrap()
        .into_iter()
        .map(|summary| summary.execution_id)
        .collect::<Vec<_>>();
    let tree_ids = repository
        .folded_workspace_trees("/repo")
        .unwrap()
        .into_iter()
        .map(|(tree, _)| tree.aggregate.id.clone())
        .collect::<Vec<_>>();
    let session_ids = crate::adaptor::gateway::agent_session::workspace_session_items(
        &repository.fact_backend(),
        &tree_ids,
        "/repo",
    )
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
    let query =
        SqliteWorkspaceQueryService::with_repository(repository.clone(), Arc::new(EmptyArchives));

    // When: workspace identity から snapshot と Session 一覧を取得する
    let snapshot = query.workspace_tree(&workspace).unwrap();
    let tree_ids = repository
        .folded_workspace_trees(workspace.as_str())
        .unwrap()
        .into_iter()
        .map(|(tree, _)| tree.aggregate.id.clone())
        .collect::<Vec<_>>();
    let sessions = crate::adaptor::gateway::agent_session::workspace_session_items(
        &repository.fact_backend(),
        &tree_ids,
        workspace.as_str(),
    )
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

#[test]
fn test_workspaceツリー投影_同じfoldのworkflow履歴と表示名が一致する() {
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
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let tree_query =
        SqliteWorkspaceQueryService::with_repository(repository.clone(), Arc::new(EmptyArchives));
    let history_query = SqliteWorkspaceQueryService::with_repository(
        repository,
        Arc::new(ArchivedExecution {
            execution_id: execution_id.to_string(),
            archived_at: 10.0,
        }),
    );

    // When
    let snapshot = tree_query
        .workspace_tree(&WorkspaceIdentity::new("/repo"))
        .unwrap();
    let history = history_query
        .workflow_history(&WorkspaceIdentity::new("/repo"))
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
    let query = SqliteWorkspaceQueryService::with_repository(repository, Arc::new(EmptyArchives));

    // When
    let snapshot = query.workspace_tree(&workspace).unwrap();
    let detail = query
        .node_detail(&workspace, execution_id)
        .unwrap()
        .unwrap();
    let tree_json = serde_json::to_value(snapshot.nodes).unwrap();

    // Then
    assert_eq!(tree_json[0]["title"], "session");
    assert_eq!(detail.title, "session");
}

#[test]
fn test_workspaceノード詳細_public_rootと子nodeの名前はnodeのtitleを保つ() {
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
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query =
        SqliteWorkspaceQueryService::with_repository(repository.clone(), Arc::new(EmptyArchives));
    let root_node = repository
        .load_node(&workspace, execution_id)
        .unwrap()
        .unwrap();
    let child_node = repository
        .load_node_by_node_execution_id(child_execution_id)
        .unwrap()
        .unwrap();

    // When
    let root_detail = query
        .node_detail(&workspace, execution_id)
        .unwrap()
        .unwrap();
    let child_detail = query
        .node_detail(&workspace, &child_node.id)
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
        can_close: false,
        can_stop: false,
        can_resume: false,
        resume_eligible: false,
        recovery_owner_reason: None,
        resume_unavailable_reason: None,
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
    owner.can_stop = true;
    owner.can_abort = true;
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
            can_resume: false,
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
    assert_eq!(json[0]["workflowCapabilities"]["canStop"], true);
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
    assert!(json[0]["workflowCapabilities"].is_null());
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
    assert_eq!(json[0]["workflowCapabilities"]["canStop"], true);
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
    failed.status = WorkspaceNodeStatus::Failed;
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
        assert_eq!(status, "failure");
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
            WorkspaceNodeStatus::Paused,
            WorkspaceNodeStatusClassification::Idle,
            "paused",
            "idle",
        ),
        (
            WorkspaceNodeStatus::Failed,
            WorkspaceNodeStatusClassification::Failure,
            "failed",
            "failure",
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
fn test_workspaceツリー契約_recovery_fenceありでも操作capabilityとresume不能理由を維持する() {
    // Given
    let execution_id = "workflow-execution";
    let owner = tree_owner(execution_id);
    let mut sequence = child_node(
        "sequence",
        execution_id,
        execution_id,
        WorkspaceNodeKind::Sequence,
        "main",
    );
    sequence.status = WorkspaceNodeStatus::Completed;
    let mut approval = child_node(
        "approval",
        "sequence",
        execution_id,
        WorkspaceNodeKind::WorkflowSession,
        "approval",
    );
    approval.status = WorkspaceNodeStatus::Waiting;
    approval.can_approve = true;
    let mut failed = child_node(
        "failed",
        "sequence",
        execution_id,
        WorkspaceNodeKind::WorkflowCommand,
        "failed",
    );
    failed.sibling_order = 1;
    failed.status = WorkspaceNodeStatus::Failed;
    failed.can_retry = true;
    let mut running = child_node(
        "running",
        "sequence",
        execution_id,
        WorkspaceNodeKind::WorkflowCommand,
        "running",
    );
    running.sibling_order = 2;
    let mut fenced_paused = child_node(
        "fenced-paused",
        "sequence",
        execution_id,
        WorkspaceNodeKind::WorkflowSession,
        "fenced-paused",
    );
    fenced_paused.sibling_order = 3;
    fenced_paused.status = WorkspaceNodeStatus::Paused;
    fenced_paused.recovery_owner_reason = Some("recovery fence".to_string());
    let tree = WorkspaceTree::restore(
        "/repo",
        vec![owner, sequence, approval, failed, running, fenced_paused],
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
    assert_eq!(json[0]["workflowCapabilities"]["canStop"], true);
    assert_eq!(json[0]["workflowCapabilities"]["canResume"], false);
    assert_eq!(
        json[0]["workflowCapabilities"]["resumeUnavailableReason"],
        "recovery fence"
    );
    assert_eq!(json[0]["workflowCapabilities"]["canAbort"], true);
    assert_eq!(json[0]["workflowCapabilities"]["canArchive"], false);
    assert_eq!(json[0]["children"][0]["capabilities"]["canApprove"], true);
    assert_eq!(json[0]["children"][1]["capabilities"]["canRetry"], true);
}

#[test]
fn test_workspaceツリー契約_pausedでもresume可否とresume不能理由を維持する() {
    // Given
    let execution_id = "paused-workflow-execution";
    let owner = tree_owner(execution_id);
    let mut paused = child_node(
        "paused",
        execution_id,
        execution_id,
        WorkspaceNodeKind::WorkflowSession,
        "paused",
    );
    paused.status = WorkspaceNodeStatus::Paused;
    paused.session_id = Some("paused-agent-session".to_string());
    paused.resume_eligible = true;
    let tree = WorkspaceTree::restore("/repo", vec![owner, paused]).unwrap();

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
    assert_eq!(json[0]["workflowCapabilities"]["canStop"], false);
    assert_eq!(json[0]["workflowCapabilities"]["canResume"], true);
    assert!(json[0]["workflowCapabilities"]["resumeUnavailableReason"].is_null());
}

#[test]
fn test_workspaceツリー契約_completedでも終了時capabilityを維持する() {
    // Given
    let execution_id = "completed-workflow-execution";
    let mut owner = tree_owner(execution_id);
    owner.status = WorkspaceNodeStatus::Completed;
    owner.can_stop = false;
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
    assert_eq!(json[0]["workflowCapabilities"]["canStop"], false);
    assert_eq!(json[0]["workflowCapabilities"]["canResume"], false);
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
        interruption_reason: None,
        resume_from_node: None,
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

#[test]
fn test_workspace読取_未対応の親または自身の定義があってもcommand出力とsession参照を取得できる() {
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
        );
        crate::adaptor::gateway::workflow::test_support::seed_unavailable_definition(
            &store,
            "00000000-0000-4000-8000-000000001745",
            "/other",
            "main",
        );
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
                .unwrap()
                .unwrap();
            let session = repository
                .load_node_by_node_execution_id("00000000-0000-4000-8000-000000001744-session")
                .unwrap()
                .unwrap();
            let query =
                SqliteWorkspaceQueryService::with_repository(repository, Arc::new(EmptyArchives));
            let workspace = WorkspaceIdentity::new("/repo");

            // When
            let snapshot = query.workspace_tree(&workspace).unwrap();
            let command_detail = query.node_detail(&workspace, &command.id).unwrap().unwrap();
            let session_detail = query.node_detail(&workspace, &session.id).unwrap().unwrap();

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
            if unavailable == "command" {
                assert_eq!(command_detail.status, "unresolved");
                assert!(!command_detail.capabilities.can_retry);
                assert!(command_detail.recovery_reason.is_some());
            }
        }
    }
}
