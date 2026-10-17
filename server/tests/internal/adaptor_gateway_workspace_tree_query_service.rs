use releashd::test_support::integration::persistence::LocalEventStore;
use releashd::test_support::integration::persistence::LocalEventStoreConfig;
use releashd::test_support::integration::providers::ProviderKind;
use releashd::test_support::integration::sessions::AgentSession;
use releashd::test_support::integration::sessions::AgentSessionRecoveryResult;
use releashd::test_support::integration::sessions::AgentSessionRepository;
use releashd::test_support::integration::sessions::AgentSessionTreeLocation;
use releashd::test_support::integration::sessions::AgentSessionUsecase;
use releashd::test_support::integration::sessions::LocalAgentSessionRepository;
use releashd::test_support::integration::workflow::seed_workflow_session_facts;
use releashd::test_support::integration::workflow::AgentSessionActivity;
use releashd::test_support::integration::workflow::ExecutionStatus;
use releashd::test_support::integration::workflow::ExecutionTreeLaunch;
use releashd::test_support::integration::workflow::NodeFact;
use releashd::test_support::integration::workflow::StopReceivedFact;
use releashd::test_support::integration::workflow::WorkflowSessionFactSeed;
use releashd::test_support::integration::workflow::WorkspaceNodeContentDto;
use releashd::test_support::integration::workspace::SqliteWorkspaceQueryService;
use releashd::test_support::integration::workspace::SqliteWorkspaceTreeRepository;
use releashd::test_support::integration::workspace::WorkspaceIdentity;
use releashd::test_support::integration::workspace::WorkspaceNodeKind;
use releashd::test_support::integration::workspace::WorkspaceQueryService;
use releashd::test_support::integration::workspace::WorkspaceTree;
use releashd::test_support::integration::workspace::WorkspaceTreeRepository;
use releashd::test_support::integration::workspace::WorkspaceVisibleNode;
use std::sync::Arc;

fn service(repository: &Arc<SqliteWorkspaceTreeRepository>) -> Arc<SqliteWorkspaceQueryService> {
    SqliteWorkspaceQueryService::with_repository(repository.clone())
}

async fn load_tree(
    repository: &SqliteWorkspaceTreeRepository,
    workspace: &WorkspaceIdentity,
) -> WorkspaceTree {
    repository
        .load_trees(std::slice::from_ref(workspace))
        .await
        .pop()
        .unwrap()
        .unwrap()
}

/// 画面に出す木の 1 項目。
#[derive(Debug, Clone, PartialEq)]
struct Row {
    kind: WorkspaceNodeKind,
    id: String,
    title: String,
    status: &'static str,
    children: Vec<Row>,
}

fn rows(items: &[WorkspaceVisibleNode<'_>]) -> Vec<Row> {
    items
        .iter()
        .map(|item| Row {
            kind: item.node().kind,
            id: item.id().to_string(),
            title: item.title().to_string(),
            status: item.node().status_classification.as_public_str(),
            children: rows(&item.children()),
        })
        .collect()
}

async fn visible_rows(
    repository: &SqliteWorkspaceTreeRepository,
    workspace: &WorkspaceIdentity,
) -> Vec<Row> {
    let tree = load_tree(repository, workspace).await;
    let visible = tree.visible();
    rows(&visible.roots())
}

async fn assert_working_session_projection(store: Arc<LocalEventStore>) {
    let workspace = WorkspaceIdentity::new("workspace-activity-read");
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query = service(&repository);

    let snapshot = visible_rows(&repository, &workspace).await;
    let node = &snapshot[0];
    assert_eq!(node.kind, WorkspaceNodeKind::WorkflowSession);
    assert_eq!(node.status, "active");
    let detail = query
        .node_detail(&workspace, &node.id)
        .await
        .unwrap()
        .expect("Session detail must exist");
    assert_eq!(detail.status, "completed");
    let detail_json = serde_json::to_string(&detail).unwrap();
    assert!(!detail_json.contains("\"activity\":"));
}

async fn assert_workflow_child_activity_projection(
    store: Arc<LocalEventStore>,
    expected_classification: &str,
) {
    let workspace = WorkspaceIdentity::new("/repo/workflow-child-activity");
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query = service(&repository);

    let snapshot = visible_rows(&repository, &workspace).await;
    let sequence = &snapshot[0];
    assert_eq!(sequence.kind, WorkspaceNodeKind::Sequence);
    let node = &sequence.children[0];
    assert_eq!(node.kind, WorkspaceNodeKind::WorkflowSession);
    assert_eq!(node.status, expected_classification);
    let detail = query
        .node_detail(&workspace, &node.id)
        .await
        .unwrap()
        .expect("workflow child Session detail must exist");
    assert_eq!(detail.status, "running");
}

#[tokio::test]
pub async fn test_workspace_tree_query_記録済み活動状態を一覧と詳細へ反映し再起動後も再現する() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
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
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    assert_working_session_projection(reopened).await;
}

#[tokio::test]
pub async fn test_workspace_tree_query_活動未観測の単独sessionは完了nodeと緑行になる() {
    // Given: create 後に活動事実を一度も記録していない単独 Session
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
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
        releashd::test_support::integration::workflow::read_tree_records(&store, session_id)
            .await
            .unwrap();
    assert!(!records
        .iter()
        .any(|record| matches!(record.fact, NodeFact::AgentActivityObserved(_))));

    // When: Workspace query service から一覧と詳細を読む
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query = service(&repository);
    let snapshot = visible_rows(&repository, &workspace).await;
    let node = &snapshot[0];
    assert_eq!(node.kind, WorkspaceNodeKind::WorkflowSession);
    let detail = query
        .node_detail(&workspace, &node.id)
        .await
        .unwrap()
        .expect("Session detail must exist");

    // Then: fold と projection を通った一覧は緑、Node は完了
    assert_eq!(node.status, "idle");
    assert_eq!(detail.status, "completed");
}

#[tokio::test]
pub async fn test_workspace_tree_query_resume直後の単独sessionは緑になる() {
    // Given: Working の後に正常終了し、provider resume が完了した単独 Session
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
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
    let query = service(&repository);
    let snapshot = visible_rows(&repository, &workspace).await;
    let node = &snapshot[0];
    assert_eq!(node.kind, WorkspaceNodeKind::WorkflowSession);
    let detail = query
        .node_detail(&workspace, &node.id)
        .await
        .unwrap()
        .expect("Session detail must exist");

    // Then: resume 後に新しい活動を観測するまでは緑
    assert_eq!(node.status, "idle");
    assert_eq!(detail.status, "completed");
}

#[tokio::test]
pub async fn test_workspace_tree_query_workflow子sessionの活動状態を一覧と詳細へ反映し再起動後も再現する(
) {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
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
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
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
pub async fn test_workspace_tree_query_活動終了と再開の反復を一覧と詳細へ毎回反映する() {
    // Given: 実行中の単独 Session と同じ store を読む Workspace query
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
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
    let query = service(&repository);
    let projected_statuses = async || {
        let snapshot = visible_rows(&repository, &workspace).await;
        let node = &snapshot[0];
        assert_eq!(node.kind, WorkspaceNodeKind::WorkflowSession);
        let detail = query
            .node_detail(&workspace, &node.id)
            .await
            .unwrap()
            .expect("Session detail must exist");
        (node.status, detail.status)
    };

    // When: Working と AwaitingInstruction を2往復させる
    let transitions = [
        (AgentSessionActivity::Working, "active"),
        (AgentSessionActivity::AwaitingInstruction, "idle"),
        (AgentSessionActivity::Working, "active"),
        (AgentSessionActivity::AwaitingInstruction, "idle"),
        (AgentSessionActivity::Working, "active"),
    ];
    let mut observed_classifications = Vec::new();
    for (index, (activity, expected_classification)) in transitions.into_iter().enumerate() {
        let request_id = format!("activity-round-trip-{index}");
        sessions
            .observe_activity(session_id, activity, &request_id)
            .await
            .unwrap();

        // Then: 一覧の status は agent activity に追従し、Node は完了したまま
        let (tree_status, detail_status) = projected_statuses().await;
        assert_eq!(tree_status, expected_classification);
        assert_eq!(detail_status, "completed");
        observed_classifications.push(tree_status);
    }
    assert_eq!(
        observed_classifications,
        ["active", "idle", "active", "idle", "active"]
    );
}

#[tokio::test]
pub async fn test_workspace_tree_query_stop事実と後続活動を一覧と詳細へ反映し再起動後も再現する() {
    // Given: Working の活動観測がある実行中の単独 Session
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
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
    let meta = releashd::test_support::integration::workflow::read_tree_records(&store, session_id)
        .await
        .unwrap()
        .last()
        .unwrap()
        .meta
        .clone();
    let append_stop = async |timestamp_ms| {
        releashd::test_support::integration::workflow::append_single_fact(
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
        let query = service(&repository);
        let snapshot = visible_rows(&repository, &workspace).await;
        let node = &snapshot[0];
        assert_eq!(node.kind, WorkspaceNodeKind::WorkflowSession);
        let detail = query
            .node_detail(&workspace, &node.id)
            .await
            .unwrap()
            .expect("Session detail must exist");
        assert_eq!(detail.status, "completed");
        node.status
    };

    // When: 活動観測を追加せず StopReceived だけを追記する
    append_stop(10).await;

    // Then: 一覧は idle、詳細 Node は completed と導出され、再起動後も再現する
    assert_eq!(projected_classification(store.clone()).await, "idle");
    drop(sessions);
    drop(store);
    let reopened = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    assert_eq!(projected_classification(reopened.clone()).await, "idle");

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
        assert_eq!(projected_classification(reopened.clone()).await, "active");
        if index == 0 {
            releashd::test_support::integration::workflow::append_single_fact(
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
            assert_eq!(projected_classification(reopened.clone()).await, "idle");
        }
    }
}

#[tokio::test]
pub async fn launch区分が同じworktreeのworkflow一覧とsession一覧を分ける() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    seed_workflow_session_facts(
        &store,
        WorkflowSessionFactSeed {
            workflow_name: "workflow",
            request: "test",
            worktree_path: "/repo",
            provider: ProviderKind::Codex,
            workflow_execution_id: "00000000-0000-4000-8000-000000001705",
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
    let query = service(&repository);

    let workflow_ids = query
        .execution_summaries(Some(&WorkspaceIdentity::new("/repo")), None)
        .await
        .unwrap()
        .into_iter()
        .map(|summary| summary.execution_id)
        .collect::<Vec<_>>();
    let tree = load_tree(&repository, &WorkspaceIdentity::new("/repo")).await;
    let session_ids = tree
        .executions()
        .iter()
        .filter(|execution| execution.launched_as == ExecutionTreeLaunch::Session)
        .filter_map(|execution| execution.session.as_ref())
        .map(|session| session.id())
        .collect::<Vec<_>>();

    assert_eq!(workflow_ids, ["00000000-0000-4000-8000-000000001705"]);
    assert_eq!(session_ids, ["standalone-session"]);
}

#[tokio::test]
pub async fn test_workspace_tree_query_workspace同定子がworktreeと異なるsessionを一覧と行に含める()
{
    // Given: workspace identity と worktree path が異なる Session 起動由来の木
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
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

    // When: workspace identity から snapshot と Session 一覧を取得する
    let tree = load_tree(&repository, &workspace).await;
    let snapshot = rows(&tree.visible().roots());
    let sessions = tree
        .executions()
        .iter()
        .filter_map(|execution| execution.session.as_ref())
        .collect::<Vec<_>>();

    // Then: root の workspace identity を共通の対象キーとして解決する
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id(), "standalone-session");
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].kind, WorkspaceNodeKind::WorkflowSession);
    assert_eq!(snapshot[0].id, "standalone-session");
}

#[tokio::test]
pub async fn test_workspaceツリー投影_同じfoldのworkflow履歴と表示名が一致する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
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
    let workspace = WorkspaceIdentity::new("/repo");
    let repository = SqliteWorkspaceTreeRepository::new(store.clone());

    // When
    let snapshot = visible_rows(&repository, &workspace).await;
    let tree_title = snapshot[0].title.as_str();
    archive_aborted(&store, directory.path(), execution_id, 10.0, "manual").await;
    let archived = load_tree(&repository, &workspace).await;
    let history = archived.archived_workflows();

    // Then
    assert_eq!(tree_title, "01_author-spec");
    assert_ne!(tree_title, "main");
    assert!(archived.visible().roots().is_empty());
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].execution_id, execution_id);
    assert_eq!(history[0].workflow_name, tree_title);
}

#[tokio::test]
pub async fn test_workspaceツリー投影_単独agent_sessionのpublic_root表示名はsessionを保つ() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
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
    let query = service(&repository);

    // When
    let snapshot = visible_rows(&repository, &workspace).await;
    let detail = query
        .node_detail(&workspace, execution_id)
        .await
        .unwrap()
        .unwrap();

    // Then
    assert_eq!(snapshot[0].title, "session");
    assert_eq!(detail.title, "session");
}

#[tokio::test]
pub async fn test_workspaceノード詳細_public_rootと子nodeの名前はnodeのtitleを保つ() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
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
    let query = service(&repository);
    let root_node = repository
        .load_node(&workspace, execution_id)
        .await
        .unwrap()
        .unwrap();
    let child_node = releashd::test_support::integration::workspace::node_for_execution(
        &*repository,
        &workspace,
        child_execution_id,
    )
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

#[tokio::test]
pub async fn test_workspace読取_未対応定義がabort済みでもcommand出力とsession参照を取得できる() {
    // Given
    for unavailable in ["main", "command", "session", "unused"] {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().into(),
            std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
        ))
        .unwrap();
        releashd::test_support::integration::workflow::seed_unavailable_definition(
            &store,
            "00000000-0000-4000-8000-000000001744",
            "/repo",
            unavailable,
        )
        .await;
        releashd::test_support::integration::workflow::seed_unavailable_definition(
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
            releashd::test_support::integration::workflow::append_single_fact(
                &store,
                &releashd::test_support::integration::workflow::NodeFactMeta {
                    tree_id: tree.into(),
                    node_execution_id: tree.into(),
                    parent_id: None,
                    node_name: "main".into(),
                    kind: releashd::test_support::integration::workflow::NodeKindName::Sequence,
                    attempt: 1,
                },
                &releashd::test_support::integration::workflow::NodeFact::AbortRequested(
                    Default::default(),
                ),
                10_000,
            )
            .await
            .unwrap();
        }
        {
            let repository = SqliteWorkspaceTreeRepository::new(store.clone());
            let command = releashd::test_support::integration::workspace::node_for_execution(
                &*repository,
                &releashd::test_support::integration::workspace::WorkspaceIdentity::new("/repo"),
                "00000000-0000-4000-8000-000000001744-command",
            )
            .await
            .unwrap()
            .unwrap();
            let session = releashd::test_support::integration::workspace::node_for_execution(
                &*repository,
                &releashd::test_support::integration::workspace::WorkspaceIdentity::new("/repo"),
                "00000000-0000-4000-8000-000000001744-session",
            )
            .await
            .unwrap()
            .unwrap();
            let query = service(&repository);
            let workspace = WorkspaceIdentity::new("/repo");

            // When
            let snapshot = visible_rows(&repository, &workspace).await;
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
            assert!(!snapshot.is_empty());
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

#[tokio::test]
pub async fn test_archive履歴_手動とworktree消失の事実の時刻と理由をそのまま投影する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store.clone());
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
        archive_aborted(&store, directory.path(), id, at, reason).await;
    }
    // When
    let tree = load_tree(&repository, &WorkspaceIdentity::new("/repo")).await;
    let history = tree.archived_workflows();
    // Then
    assert_eq!(history.len(), 2);
    for (id, at, reason) in fixtures {
        let item = history.iter().find(|item| item.execution_id == id).unwrap();
        let archive = item.archive.as_ref().unwrap();
        assert_eq!(archive.archived_at, at);
        assert_eq!(archive.archive_reason, reason);
        assert_eq!(item.status, ExecutionStatus::Aborted);
    }
    assert_eq!(history[0].execution_id, fixtures[1].0);
}

/// 実行木を abort してから archive する。
async fn archive_aborted(
    store: &Arc<LocalEventStore>,
    data_dir: &std::path::Path,
    execution_id: &str,
    archived_at: f64,
    reason: &str,
) {
    use releashd::test_support::integration::workflow::ExecutionTreeArchiveFactRepository;
    use releashd::test_support::integration::workflow::ExecutionTreeArchiveRepository;
    let root =
        releashd::test_support::integration::workflow::read_tree_records(store, execution_id)
            .await
            .unwrap()
            .remove(0);
    releashd::test_support::integration::workflow::append_single_fact(
        store,
        &root.meta,
        &NodeFact::AbortRequested(Default::default()),
        2,
    )
    .await
    .unwrap();
    ExecutionTreeArchiveFactRepository::new(store.clone(), data_dir)
        .archive(
            &releashd::test_support::integration::workflow::ExecutionTreeId::new(execution_id)
                .unwrap(),
            archived_at,
            reason,
        )
        .await
        .unwrap();
}

#[tokio::test]
pub async fn test_workflow単一取得_単独sessionをworkflow_summaryとして返さない() {
    use releashd::test_support::integration::workflow::SessionExecutionTreeRootFacts;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let session = "00000000-0000-4000-8000-000000000991";
    let workflow = "00000000-0000-4000-8000-000000000992";
    let facts =
        SessionExecutionTreeRootFacts::new(session, "/repo", "/repo", ProviderKind::Codex, None)
            .unwrap();
    releashd::test_support::integration::workflow::append_fact_batch_for_seed(
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
    let query = service(&SqliteWorkspaceTreeRepository::new(store));
    // When / Then
    let ids = query
        .execution_summaries(None, None)
        .await
        .unwrap()
        .into_iter()
        .map(|execution| execution.execution_id)
        .collect::<Vec<_>>();
    assert!(!ids.iter().any(|id| id == session));
    assert!(ids.iter().any(|id| id == workflow));
}

#[tokio::test]
pub async fn test_workspace読取_実経路で失敗分類を保持する() {
    use releashd::test_support::integration::persistence::ReadFailure;
    use releashd::test_support::integration::transport::classified_error;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store.clone());
    let query = service(&repository);
    let workspace = WorkspaceIdentity::new("/repo");
    for (failure, expected) in ReadFailure::cases() {
        for mode in 0..2 {
            let expected = if mode != 0
                && matches!(failure, ReadFailure::Sqlite(rusqlite::ffi::SQLITE_IOERR))
            {
                connectrpc::ErrorCode::Internal
            } else {
                expected
            };
            store.fail_next_read(failure.clone());
            // When
            let result = if mode == 1 {
                repository
                    .load_trees(std::slice::from_ref(&workspace))
                    .await
                    .pop()
                    .unwrap()
                    .map(|_| ())
            } else {
                query.execution_records(None, None).await.map(|_| ())
            };
            // Then
            assert_eq!(classified_error(result.unwrap_err()).code, expected);
        }
    }
}

#[tokio::test]
pub async fn test_session選択_記録済みsessionとnodeのdtoを返し対象外は無しを返す() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    seed_workflow_session_facts(
        &store,
        WorkflowSessionFactSeed {
            workflow_name: "selection",
            request: "select",
            worktree_path: "/repo/selection",
            provider: ProviderKind::Codex,
            workflow_execution_id: "00000000-0000-4000-8000-000000001800",
            node_execution_id: "selection-node",
            session_id: "selection-session",
            initial_instruction_admitted: true,
        },
    )
    .await
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);

    let workspace = WorkspaceIdentity::new("/repo/selection");
    let tree = load_tree(&repository, &workspace).await;
    let expected = tree
        .nodes()
        .iter()
        .find(|node| node.session_id.as_deref() == Some("selection-session"))
        .unwrap()
        .id
        .clone();
    let selected = repository
        .load_node_by_session_id(&workspace, "selection-session")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selected.session_id.as_deref(), Some("selection-session"));
    assert_eq!(selected.id, expected);
    assert!(repository
        .load_node_by_session_id(&workspace, "missing")
        .await
        .unwrap()
        .is_none());
    assert!(repository
        .load_node_by_session_id(&WorkspaceIdentity::new("/other"), "selection-session")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
pub async fn test_command詳細_記録から実行環境と過去attemptの出力を読める() {
    use releashd::test_support::integration::workflow::{
        append_facts_for_events, ExecutionOrigin, NodeKindName, WorkflowDefinition, WorkflowEvent,
    };
    for child in [false, true] {
        // Given
        let directory = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().into(),
            Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
        ))
        .unwrap();
        let tree_id = "00000000-0000-4000-8000-000000001205";
        let old_id = format!("{tree_id}-command");
        let new_id = format!("{tree_id}-command-retry");
        let parent_id = format!("{tree_id}-sequence");
        let parent = child.then(|| {
            releashd::test_support::integration::workflow::ExecutionParentRef::sequence_child(
                parent_id.clone(),
            )
        });
        let node_name = if child { "child" } else { "main" };
        let source = if child {
            "name: retry\ndescription: test\nnodes:\n  main: {sequence: {children: [{child: {}}]}}\n  child: {command: 'printf kept'}\n"
        } else {
            "name: retry\ndescription: test\nnodes:\n  main: {command: 'printf kept'}\n"
        };
        let definition: WorkflowDefinition = serde_saphyr::from_str(source).unwrap();
        let mut events = vec![
            WorkflowEvent::ExecutionStarted {
                repository_root: None,
                execution_id: tree_id.into(),
                workflow_name: "retry".into(),
                worktree_path: "/repo".into(),
                created_from: ExecutionOrigin::DesktopUi,
                request: String::new(),
                definition,
                timestamp: 1.0,
            },
            WorkflowEvent::NodeStarted {
                execution_id: tree_id.into(),
                node_execution_id: old_id.clone(),
                node_name: node_name.into(),
                kind: NodeKindName::Command,
                attempt: 1,
                parent: parent.clone(),
                worktree: None,
                timestamp: 2.0,
            },
            WorkflowEvent::CommandSpawned {
                execution_id: tree_id.into(),
                node_execution_id: old_id.clone(),
                display_command: "printf kept".into(),
                shell: Some("/bin/sh".into()),
                cwd: Some("/repo".into()),
                timestamp: 3.0,
            },
            WorkflowEvent::ArtifactProduced {
                execution_id: tree_id.into(),
                node_execution_id: old_id.clone(),
                node_name: node_name.into(),
                contract: None,
                value: serde_json::json!({"stdout":"kept", "stderr":"", "exit_code":0, "duration":10}),
                request_id: None,
                submitted_at: None,
                timestamp: 4.0,
            },
            WorkflowEvent::NodeRetryRequested {
                execution_id: tree_id.into(),
                node_execution_id: old_id.clone(),
                timestamp: 9.0,
            },
            WorkflowEvent::NodeStarted {
                execution_id: tree_id.into(),
                node_execution_id: new_id.clone(),
                node_name: node_name.into(),
                kind: NodeKindName::Command,
                attempt: 2,
                parent: parent.clone(),
                worktree: None,
                timestamp: 10.0,
            },
            WorkflowEvent::CommandSpawned {
                execution_id: tree_id.into(),
                node_execution_id: new_id.clone(),
                display_command: "printf retry".into(),
                shell: Some("/bin/sh".into()),
                cwd: Some("/repo".into()),
                timestamp: 11.0,
            },
            WorkflowEvent::ArtifactProduced {
                execution_id: tree_id.into(),
                node_execution_id: new_id.clone(),
                node_name: node_name.into(),
                contract: None,
                value: serde_json::json!({"stdout":"retry", "stderr":"failure", "exit_code":2, "duration":20}),
                request_id: None,
                submitted_at: None,
                timestamp: 12.0,
            },
        ];
        if child {
            events.insert(
                1,
                WorkflowEvent::NodeStarted {
                    execution_id: tree_id.into(),
                    node_execution_id: parent_id,
                    node_name: "main".into(),
                    kind: NodeKindName::Sequence,
                    attempt: 1,
                    parent: None,
                    worktree: None,
                    timestamp: 1.5,
                },
            );
        }
        append_facts_for_events(&store, &events).await.unwrap();
        // When / Then
        for repository in [
            SqliteWorkspaceTreeRepository::new(store.clone()),
            SqliteWorkspaceTreeRepository::new(store.clone()),
        ] {
            let workspace = WorkspaceIdentity::new("/repo");
            for (id, stdout, exit_code, reason) in [
                (&old_id, "kept", 0, "Workflow node aborted"),
                (&new_id, "retry", 2, "Command exited with code 2"),
            ] {
                let node = releashd::test_support::integration::workspace::node_for_execution(
                    &*repository,
                    &workspace,
                    id,
                )
                .await
                .unwrap()
                .unwrap();
                let detail = service(&repository)
                    .node_detail(&workspace, &node.id)
                    .await
                    .unwrap()
                    .unwrap();
                if id == &new_id {
                    assert_eq!(detail.past_attempts.len(), 1);
                    let old = releashd::test_support::integration::workspace::node_for_execution(
                        &*repository,
                        &workspace,
                        &old_id,
                    )
                    .await
                    .unwrap()
                    .unwrap();
                    assert_eq!(detail.past_attempts[0].id, old.id);
                }
                assert_eq!(detail.execution_id.as_deref(), Some(tree_id));
                assert!(detail.can_abort);
                assert_eq!(detail.error_reason.as_deref(), Some(reason));
                let WorkspaceNodeContentDto::Command(content) = detail.content else {
                    panic!("command expected")
                };
                assert_eq!(content.shell.as_deref(), Some("/bin/sh"));
                assert_eq!(content.cwd.as_deref(), Some("/repo"));
                let result = content.result.unwrap();
                assert_eq!(result.stdout, stdout);
                assert_eq!(result.exit_code, exit_code);
            }
        }
    }
}

#[tokio::test]
pub async fn test_command起動失敗_差分読取と再読込で詳細と過去attemptへ理由を配る() {
    use releashd::test_support::integration::workflow::{
        append_facts_for_events, ExecutionOrigin, NodeExecutionFailureKind, NodeKindName,
        WorkflowDefinition, WorkflowEvent,
    };
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let tree_id = "00000000-0000-4000-8000-000000002053";
    let old_id = format!("{tree_id}-command");
    let new_id = format!("{tree_id}-retry");
    let definition: WorkflowDefinition = serde_saphyr::from_str(
        "name: retry\ndescription: test\nnodes:\n  main: {command: 'true'}\n",
    )
    .unwrap();
    append_facts_for_events(
        &store,
        &[
            WorkflowEvent::ExecutionStarted {
                repository_root: None,
                execution_id: tree_id.into(),
                workflow_name: "retry".into(),
                worktree_path: "/repo".into(),
                created_from: ExecutionOrigin::DesktopUi,
                request: String::new(),
                definition,
                timestamp: 1.0,
            },
            WorkflowEvent::NodeStarted {
                execution_id: tree_id.into(),
                node_execution_id: old_id.clone(),
                node_name: "main".into(),
                kind: NodeKindName::Command,
                attempt: 1,
                parent: None,
                worktree: None,
                timestamp: 2.0,
            },
        ],
    )
    .await
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store.clone());
    let workspace = WorkspaceIdentity::new("/repo");
    let initial = load_tree(&repository, &workspace).await;
    assert!(initial
        .nodes()
        .iter()
        .all(|node| node.error_reason.is_none()));
    let reason = "failed to spawn command: directory not found";
    append_facts_for_events(
        &store,
        &[WorkflowEvent::NodeFailed {
            execution_id: tree_id.into(),
            node_execution_id: old_id.clone(),
            node_name: "main".into(),
            attempt: 1,
            reason: reason.into(),
            failure_kind: NodeExecutionFailureKind::InfrastructureCrash,
            retry_count: None,
            timestamp: 3.0,
        }],
    )
    .await
    .unwrap();
    // When / Then
    for retried in [false, true] {
        if retried {
            append_facts_for_events(
                &store,
                &[
                    WorkflowEvent::NodeRetryRequested {
                        execution_id: tree_id.into(),
                        node_execution_id: old_id.clone(),
                        timestamp: 4.0,
                    },
                    WorkflowEvent::NodeStarted {
                        execution_id: tree_id.into(),
                        node_execution_id: new_id.clone(),
                        node_name: "main".into(),
                        kind: NodeKindName::Command,
                        attempt: 2,
                        parent: None,
                        worktree: None,
                        timestamp: 5.0,
                    },
                ],
            )
            .await
            .unwrap();
        }
        for reader in [
            repository.clone(),
            SqliteWorkspaceTreeRepository::new(store.clone()),
        ] {
            let tree = load_tree(&reader, &workspace).await;
            let old = tree
                .nodes()
                .iter()
                .find(|node| node.node_execution_id.as_deref() == Some(&old_id))
                .unwrap();
            assert_eq!(old.failure_reason().as_deref(), Some(reason));
            assert_eq!(old.is_retry_history, retried);
            let detail = service(&reader)
                .node_detail(&workspace, &old.id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(detail.error_reason.as_deref(), Some(reason));
            let WorkspaceNodeContentDto::Command(command) = detail.content else {
                panic!("command expected")
            };
            assert!(command.result.is_none());
            if retried {
                let current = tree
                    .nodes()
                    .iter()
                    .find(|node| node.node_execution_id.as_deref() == Some(&new_id))
                    .unwrap();
                assert!(current.past_attempt_ids.contains(&old.id));
                assert_eq!(current.failure_reason(), None);
            } else {
                assert_eq!(detail.status, "running");
            }
        }
    }
}

#[tokio::test]
pub async fn test_command詳細_完了済み実行のabort不可をrepositoryから配る() {
    use releashd::test_support::integration::workflow::{
        append_facts_for_events, ExecutionOrigin, NodeKindName, WorkflowDefinition, WorkflowEvent,
    };
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let tree_id = "00000000-0000-4000-8000-000000001205";
    let node_id = format!("{tree_id}-command");
    let definition: WorkflowDefinition = serde_saphyr::from_str(
        "name: completed\ndescription: test\nnodes:\n  main: {command: 'printf kept'}\n",
    )
    .unwrap();
    append_facts_for_events(
        &store,
        &[
            WorkflowEvent::ExecutionStarted {
                repository_root: None,
                execution_id: tree_id.into(),
                workflow_name: "completed".into(),
                worktree_path: "/repo".into(),
                created_from: ExecutionOrigin::DesktopUi,
                request: String::new(),
                definition,
                timestamp: 1.0,
            },
            WorkflowEvent::NodeStarted {
                execution_id: tree_id.into(),
                node_execution_id: node_id.clone(),
                node_name: "main".into(),
                kind: NodeKindName::Command,
                attempt: 1,
                parent: None,
                worktree: None,
                timestamp: 2.0,
            },
            WorkflowEvent::ExecutionCompleted {
                execution_id: tree_id.into(),
                total_token_usage: Default::default(),
                timestamp: 3.0,
            },
        ],
    )
    .await
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let workspace = WorkspaceIdentity::new("/repo");
    let node = releashd::test_support::integration::workspace::node_for_execution(
        &*repository,
        &workspace,
        &node_id,
    )
    .await
    .unwrap()
    .unwrap();
    // When
    let detail = service(&repository)
        .node_detail(&workspace, &node.id)
        .await
        .unwrap()
        .unwrap();
    // Then
    assert_eq!(detail.execution_id.as_deref(), Some(tree_id));
    assert!(!detail.can_abort);
}

#[tokio::test]
pub async fn test_node_artifact購読_同名の周回と再提出をnode実行idで分離する() {
    use releashd::test_support::integration::workflow::{
        append_single_fact, ArtifactProducedFact, NodeFactMeta, NodeKindName,
        WorkflowGetOutputResult,
    };
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let meta = |id: &str| NodeFactMeta {
        tree_id: "tree".into(),
        node_execution_id: id.into(),
        parent_id: Some("root".into()),
        node_name: "loop".into(),
        kind: NodeKindName::Session,
        attempt: 1,
    };
    for (id, value, timestamp) in [
        ("first", "old", 1000),
        ("second", "second", 2000),
        ("first", "first", 3000),
    ] {
        append_single_fact(
            &store,
            &meta(id),
            &NodeFact::ArtifactProduced(ArtifactProducedFact {
                contract: Some("result".into()),
                value: serde_json::json!({"value":value}),
                request_id: Some(value.into()),
            }),
            timestamp,
        )
        .await
        .unwrap();
    }
    let query = service(&SqliteWorkspaceTreeRepository::new(store));
    // When / Then
    for (id, expected) in [("first", "first"), ("second", "second")] {
        let WorkflowGetOutputResult::Submitted {
            structured_output,
            contract,
            request_id,
            ..
        } = query.node_artifact(id).await.unwrap()
        else {
            panic!("Artifact missing")
        };
        assert_eq!(structured_output, serde_json::json!({"value":expected}));
        assert_eq!(contract.as_deref(), Some("result"));
        assert_eq!(request_id.as_deref(), Some(expected));
    }
    assert_eq!(
        query.node_artifact("missing").await.unwrap(),
        WorkflowGetOutputResult::NotSubmitted
    );
    let connection =
        rusqlite::Connection::open(directory.path().join("local-event-store.sqlite3")).unwrap();
    connection.execute("UPDATE node_events SET detail = 'broken' WHERE node_execution_id = 'first' AND event_type = 'artifact_produced'", []).unwrap();
    assert!(matches!(
        query.node_artifact("first").await,
        Err(releashd::test_support::integration::workflow::WorkflowError::CorruptStoredState(_))
    ));
}

#[tokio::test]
pub async fn test_sessionタブ詳細_id所在から名前と一覧と同じ色を読み別workspaceを拒否する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("workspace-session-tab");
    let session_id = "session-tab";
    let sessions =
        AgentSessionUsecase::new(Arc::new(LocalAgentSessionRepository::new(store.clone())));
    sessions
        .create(
            session_id,
            workspace.clone(),
            "workspace-session-tab".into(),
            ProviderKind::Codex,
            AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
            "create-session-tab",
        )
        .await
        .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store.clone());
    let query = service(&repository);
    use releashd::test_support::integration::sessions::{
        AgentSessionQueryService, LocalAgentSessionQueryService,
    };
    let item = LocalAgentSessionQueryService::new(store.clone())
        .get(session_id)
        .await
        .unwrap()
        .unwrap();
    // When / Then
    for (activity, expected) in [
        (AgentSessionActivity::Working, "active"),
        (AgentSessionActivity::AwaitingAnswer, "attention"),
        (AgentSessionActivity::AwaitingInstruction, "idle"),
    ] {
        sessions
            .observe_activity(session_id, activity, &format!("observe-{expected}"))
            .await
            .unwrap();
        let read = query
            .session_node_detail(
                &workspace,
                &item.tree_location.tree_id,
                &item.tree_location.node_execution_id,
            )
            .await
            .unwrap()
            .unwrap();
        let detail = read.detail;
        assert_eq!(detail.classification, expected);
        let tree = load_tree(&repository, &workspace).await;
        let visible = tree.visible();
        let row = visible.roots()[0];
        assert_eq!(detail.title, row.title());
        assert_eq!(
            detail.classification,
            row.node().status_classification.as_public_str()
        );
        assert!(detail.capabilities.can_rename);
        assert!(
            matches!(detail.content, WorkspaceNodeContentDto::Session(ref value) if value.session_id.as_deref() == Some(session_id))
        );
    }
    use releashd::test_support::integration::platform::{
        Failure, FailureKey, FailureRecordStore, TechnicalFailureNature, WorkFailure,
    };
    let failures = Arc::new(FailureRecordStore::default());
    let workflow =
        releashd::test_support::integration::platform::build_workflow_services_with_gateways(
            failures.clone(),
            directory.path(),
            Arc::new(
                releashd::test_support::integration::workflow::PassthroughManagedWorktreeGateway,
            ),
            Arc::new(
                releashd::test_support::integration::workflow::NoopWorkflowExternalEditorGateway,
            ),
            store,
            None,
            None,
        )
        .0;
    // When
    failures.observe(
        &FailureKey::new("session-tab", &item.tree_location.node_execution_id),
        WorkFailure {
            kind: Failure::Technical(TechnicalFailureNature::Other),
            message: "background failure".into(),
        },
    );
    let detail = workflow
        .get_session_node_detail(&item)
        .await
        .unwrap()
        .unwrap();
    // Then
    assert_eq!(detail.classification, "attention");
    assert!(query
        .session_node_detail(
            &WorkspaceIdentity::new("other-workspace"),
            &item.tree_location.tree_id,
            &item.tree_location.node_execution_id,
        )
        .await
        .unwrap()
        .is_none());
    assert!(query
        .session_node_detail(&workspace, "missing", &item.tree_location.node_execution_id,)
        .await
        .unwrap()
        .is_none());
    assert!(query
        .session_node_detail(&workspace, &item.tree_location.tree_id, "missing",)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
pub async fn test_準備中の会話詳細_起動失敗の理由を返し紐付け後は準備中の失敗を残さない() {
    use releashd::test_support::integration::workflow::{
        append_single_fact, read_tree_records, NodeExecutionFailureKind, RuntimeFailureObservedFact,
    };
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let tree_id = "00000000-0000-4000-8000-000000002064";
    seed_workflow_session_facts(
        &store,
        WorkflowSessionFactSeed {
            workflow_name: "preparing",
            request: "test",
            worktree_path: "/repo",
            provider: ProviderKind::Codex,
            workflow_execution_id: tree_id,
            node_execution_id: "pending-node",
            session_id: "pending-agent",
            initial_instruction_admitted: false,
        },
    )
    .await
    .unwrap();
    let attached = read_tree_records(&store, tree_id)
        .await
        .unwrap()
        .into_iter()
        .find(|record| matches!(record.fact, NodeFact::SessionAttached(_)))
        .unwrap();
    let connection =
        rusqlite::Connection::open(directory.path().join("local-event-store.sqlite3")).unwrap();
    connection
        .execute(
            "DELETE FROM node_events WHERE tree_id = ? AND event_type = 'session_attached'",
            [tree_id],
        )
        .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store.clone());
    let tree = load_tree(&repository, &WorkspaceIdentity::new("/repo")).await;
    let node = tree
        .nodes()
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some("pending-node"))
        .unwrap();
    let query = service(&repository);
    assert!(query
        .node_detail(&WorkspaceIdentity::new("/repo"), &node.id)
        .await
        .unwrap()
        .unwrap()
        .error_reason
        .is_none());
    // When
    append_single_fact(
        &store,
        &attached.meta,
        &NodeFact::RuntimeFailureObserved(RuntimeFailureObservedFact {
            reason: "provider could not start".into(),
            failure_kind: NodeExecutionFailureKind::InfrastructureCrash,
        }),
        5000,
    )
    .await
    .unwrap();
    let detail = query
        .node_detail(&WorkspaceIdentity::new("/repo"), &node.id)
        .await
        .unwrap()
        .unwrap();
    // Then
    assert_eq!(
        detail.error_reason.as_deref(),
        Some("provider could not start")
    );
    assert!(
        matches!(detail.content, WorkspaceNodeContentDto::Session(content) if content.session_id.is_none())
    );
    // When
    append_single_fact(&store, &attached.meta, &attached.fact, 6000)
        .await
        .unwrap();
    let detail = query
        .node_detail(&WorkspaceIdentity::new("/repo"), &node.id)
        .await
        .unwrap()
        .unwrap();
    // Then
    assert!(detail.error_reason.is_none());
    assert!(
        matches!(detail.content, WorkspaceNodeContentDto::Session(content) if content.session_id.as_deref() == Some("pending-agent"))
    );
}

#[tokio::test]
pub async fn test_sessionタブ詳細_実行木と異なるrootの終端を子にも適用する() {
    use releashd::test_support::integration::workflow::{append_single_fact, AbortRequestedFact};
    // Given
    for terminal in [
        NodeFact::ExecutionCompleted,
        NodeFact::AbortRequested(AbortRequestedFact::default()),
    ] {
        let (_directory, store, parent, child) =
            crate::adaptor_gateway_workspace_tree_repository::delegate_fixture(
                "/repo",
                AgentSessionActivity::AwaitingInstruction,
            )
            .await;
        assert_ne!(parent.tree_id, parent.node_execution_id);
        let repository = SqliteWorkspaceTreeRepository::new(store.clone());
        let query = service(&repository);
        assert!(
            query
                .session_node_detail(
                    &WorkspaceIdentity::new("/repo"),
                    &parent.tree_id,
                    &child.node_execution_id
                )
                .await
                .unwrap()
                .unwrap()
                .detail
                .can_abort
        );
        // When
        append_single_fact(&store, &parent, &terminal, 9000)
            .await
            .unwrap();
        // Then
        for meta in [&parent, &child] {
            let list = releashd::test_support::integration::workspace::node_for_execution(
                &*repository,
                &WorkspaceIdentity::new("/repo"),
                &meta.node_execution_id,
            )
            .await
            .unwrap()
            .unwrap();
            let detail = query
                .session_node_detail(
                    &WorkspaceIdentity::new("/repo"),
                    &parent.tree_id,
                    &meta.node_execution_id,
                )
                .await
                .unwrap()
                .unwrap()
                .detail;
            let listed_detail = query
                .node_detail(&WorkspaceIdentity::new("/repo"), &list.id)
                .await
                .unwrap()
                .unwrap();
            assert!(!detail.can_abort);
            assert_eq!(detail.can_abort, listed_detail.can_abort);
            assert_eq!(detail.status, listed_detail.status);
            assert_eq!(detail.updated_at, listed_detail.updated_at);
            assert_eq!(detail.error_reason, listed_detail.error_reason);
            assert_eq!(detail.capabilities, listed_detail.capabilities);
            assert_eq!(
                detail.classification,
                list.status_classification.as_public_str()
            );
        }
    }
}

#[tokio::test]
pub async fn test_sessionタブ詳細_保存定義と異なる終了履歴を復元し稼働中の不整合は拒否する() {
    use releashd::test_support::integration::workflow::append_single_fact;
    // Given
    let (directory, store, parent, child) =
        crate::adaptor_gateway_workspace_tree_repository::delegate_fixture(
            "/repo",
            AgentSessionActivity::AwaitingInstruction,
        )
        .await;
    let connection =
        rusqlite::Connection::open(directory.path().join("local-event-store.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE node_events SET node_name = 'missing' WHERE node_execution_id = ?1",
            [&child.node_execution_id],
        )
        .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store.clone());
    let query = service(&repository);
    // When / Then
    assert!(query
        .session_node_detail(
            &WorkspaceIdentity::new("/repo"),
            &parent.tree_id,
            &child.node_execution_id
        )
        .await
        .is_err());
    // When
    append_single_fact(&store, &parent, &NodeFact::ExecutionCompleted, 9000)
        .await
        .unwrap();
    // Then
    for meta in [&parent, &child] {
        let detail = query
            .session_node_detail(
                &WorkspaceIdentity::new("/repo"),
                &parent.tree_id,
                &meta.node_execution_id,
            )
            .await
            .unwrap()
            .unwrap()
            .detail;
        let listed = releashd::test_support::integration::workspace::node_for_execution(
            &*repository,
            &WorkspaceIdentity::new("/repo"),
            &meta.node_execution_id,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(detail.status, "completed");
        assert_eq!(detail.title, listed.title);
        assert_eq!(
            detail.classification,
            listed.status_classification.as_public_str()
        );
        assert!(!detail.can_abort);
    }
}

#[tokio::test]
pub async fn test_旧会話参照照会_追記された紐付けと再試行と削除を反映する() {
    use releashd::test_support::integration::workflow::{
        append_single_fact, ExecutionOrigin, NodeDefinition, NodeFactMeta, NodeKindName,
        SessionAttachedFact, StartedFact, TreeRootFact, WorkflowDefinition,
    };
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let tree = "00000000-0000-4000-8000-000000002080";
    let first = NodeFactMeta {
        tree_id: tree.into(),
        node_execution_id: "00000000-0000-4000-8000-000000002081".into(),
        node_name: "main".into(),
        kind: NodeKindName::Session,
        parent_id: None,
        attempt: 1,
    };
    let mut next = first.clone();
    next.node_execution_id = "00000000-0000-4000-8000-000000002082".into();
    next.attempt = 2;
    append_single_fact(
        &store,
        &first,
        &NodeFact::Started(StartedFact {
            root: Some(Box::new(TreeRootFact {
                repository_root: None,
                workspace_identity: "/repo".into(),
                worktree_path: "/repo".into(),
                created_from: ExecutionOrigin::Cli,
                request: String::new(),
                workflow_name: "main".into(),
                definition: Some(WorkflowDefinition {
                    entry: "main".into(),
                    nodes: vec![NodeDefinition {
                        name: "main".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                launched_as: ExecutionTreeLaunch::Workflow,
            })),
            parent: None,
            worktree: None,
        }),
        1000,
    )
    .await
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store.clone());
    let query = service(&repository);
    let ids = vec![
        tree.into(),
        first.node_execution_id.clone(),
        "missing".into(),
    ];
    // When / Then
    for _ in 0..2 {
        let result = query
            .resolve_session_nodes(&WorkspaceIdentity::new("/repo"), &ids)
            .await;
        assert!(result.sessions.is_empty());
        assert!(result.failures.is_empty());
    }
    let attach = |id: &str| {
        NodeFact::SessionAttached(SessionAttachedFact {
            session_id: id.into(),
            provider_session_id: None,
            transcript_ref: None,
            initial_instruction_admitted: false,
        })
    };
    // When
    append_single_fact(&store, &first, &attach("first-agent"), 2000)
        .await
        .unwrap();
    let before = load_tree(&repository, &WorkspaceIdentity::new("/repo")).await;
    let alias = before
        .nodes()
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some(&first.node_execution_id))
        .unwrap()
        .id
        .clone();
    let mut ids = ids;
    ids.push(alias.clone());
    let result = query
        .resolve_session_nodes(&WorkspaceIdentity::new("/repo"), &ids)
        .await;
    // Then
    assert_eq!(result.sessions[tree], "first-agent");
    assert_eq!(result.sessions[&alias], "first-agent");
    // When
    for (meta, fact) in [
        (&first, NodeFact::RetryRequested),
        (
            &next,
            NodeFact::Started(StartedFact {
                root: None,
                parent: None,
                worktree: None,
            }),
        ),
        (&next, attach("next-agent")),
    ] {
        append_single_fact(&store, meta, &fact, 3000).await.unwrap();
    }
    let result = query
        .resolve_session_nodes(&WorkspaceIdentity::new("/repo"), &ids)
        .await;
    // Then
    assert_eq!(result.sessions[tree], "next-agent");
    assert_eq!(result.sessions[&first.node_execution_id], "first-agent");
    assert_eq!(result.sessions[&alias], "first-agent");
    assert!(result.failures.is_empty());
    // When
    rusqlite::Connection::open(directory.path().join("local-event-store.sqlite3"))
        .unwrap()
        .execute("DELETE FROM node_events WHERE tree_id = ?1", [tree])
        .unwrap();
    let result = query
        .resolve_session_nodes(&WorkspaceIdentity::new("/repo"), &ids)
        .await;
    // Then
    assert!(result.sessions.is_empty());
    assert!(result.failures.is_empty());
}

#[tokio::test]
pub async fn test_sessionタブ詳細_再試行した親の旧委任子と動的並列のスコープを保つ() {
    use releashd::test_support::integration::workflow::{
        append_single_fact, composite_dynamic_fanout_facts, retried_delegate_parent_facts,
        scoped_dynamic_fanout_facts, stop_first_delegate_facts,
    };
    // Given
    for records in [
        retried_delegate_parent_facts(),
        scoped_dynamic_fanout_facts(),
        composite_dynamic_fanout_facts(),
        stop_first_delegate_facts(),
    ] {
        let retry = records.iter().any(|record| record.meta.attempt == 2);
        let target = records
            .iter()
            .find(|record| {
                if retry {
                    record.meta.attempt == 2
                } else {
                    record.meta.node_name == "main"
                }
            })
            .unwrap()
            .meta
            .clone();
        let directory = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().into(),
            Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
        ))
        .unwrap();
        let repository = SqliteWorkspaceTreeRepository::new(store.clone());
        let query = service(&repository);
        let boundary = records.len() - if retry { 3 } else { 4 };
        for (index, record) in records.iter().enumerate() {
            append_single_fact(&store, &record.meta, &record.fact, record.timestamp_ms)
                .await
                .unwrap();
            if index + 1 != boundary && index + 1 != records.len() {
                continue;
            }
            // When
            let read = query
                .session_node_detail(
                    &WorkspaceIdentity::new("/repo"),
                    &target.tree_id,
                    &target.node_execution_id,
                )
                .await
                .unwrap()
                .unwrap();
            let list = releashd::test_support::integration::workspace::node_for_execution(
                &*repository,
                &WorkspaceIdentity::new("/repo"),
                &target.node_execution_id,
            )
            .await
            .unwrap()
            .unwrap();
            let listed_detail = query
                .node_detail(&WorkspaceIdentity::new("/repo"), &list.id)
                .await
                .unwrap()
                .unwrap();
            // Then
            assert_eq!(read.detail.status, listed_detail.status);
            assert_eq!(read.detail.updated_at, listed_detail.updated_at);
            assert_eq!(read.detail.error_reason, listed_detail.error_reason);
            assert_eq!(read.detail.capabilities, listed_detail.capabilities);
            assert_eq!(read.detail.submit_received, listed_detail.submit_received);
            assert_eq!(read.detail.stop_received, listed_detail.stop_received);
            assert_eq!(
                read.detail.classification,
                list.status_classification.as_public_str()
            );
            assert_eq!(
                read.detail.classification,
                if index + 1 == boundary {
                    "active"
                } else {
                    "attention"
                }
            );
        }
    }
}

#[tokio::test]
pub async fn test_旧会話参照照会_保持した動的並列の親と出現順から追記した子を解決する() {
    use releashd::test_support::integration::workflow::{
        append_single_fact, scoped_dynamic_fanout_facts, NodeKindName, SessionAttachedFact,
    };
    let records = scoped_dynamic_fanout_facts();
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store.clone());
    let query = service(&repository);
    let boundary = records
        .iter()
        .position(|record| record.meta.node_name == "inner")
        .unwrap();
    for record in &records[..boundary] {
        append_single_fact(&store, &record.meta, &record.fact, record.timestamp_ms)
            .await
            .unwrap();
    }
    let workspace = WorkspaceIdentity::new("/repo");
    let unresolved = vec!["missing".into()];
    let initial = query.resolve_session_nodes(&workspace, &unresolved).await;
    assert!(initial.failures.is_empty());
    let mut expected = std::collections::HashMap::new();
    for record in &records[boundary..] {
        append_single_fact(&store, &record.meta, &record.fact, record.timestamp_ms)
            .await
            .unwrap();
        if matches!(record.fact, NodeFact::Started(_)) && record.meta.kind == NodeKindName::Session
        {
            let session = format!("agent-{}", record.meta.node_execution_id);
            append_single_fact(
                &store,
                &record.meta,
                &NodeFact::SessionAttached(SessionAttachedFact {
                    session_id: session.clone(),
                    provider_session_id: None,
                    transcript_ref: None,
                    initial_instruction_admitted: false,
                }),
                record.timestamp_ms,
            )
            .await
            .unwrap();
            expected.insert(record.meta.node_execution_id.clone(), session);
        }
    }
    let list = load_tree(&repository, &workspace).await;
    let ids: Vec<_> = list
        .nodes()
        .iter()
        .filter(|node| {
            node.node_execution_id
                .as_ref()
                .is_some_and(|id| expected.contains_key(id))
        })
        .map(|node| node.id.clone())
        .collect();
    assert_eq!(ids.len(), 2);
    let resolved = query.resolve_session_nodes(&workspace, &ids).await;
    assert!(resolved.failures.is_empty());
    for node in list.nodes().iter().filter(|node| ids.contains(&node.id)) {
        assert_eq!(
            resolved.sessions[&node.id],
            expected[node.node_execution_id.as_ref().unwrap()]
        );
    }
    // When: an earlier occurrence is corrupt, later children must not inherit its alias.
    let first = expected.keys().min().unwrap();
    rusqlite::Connection::open(directory.path().join("local-event-store.sqlite3")).unwrap()
        .execute("UPDATE node_events SET detail = 'broken' WHERE node_execution_id = ?1 AND event_type = 'started'", [first]).unwrap();
    let query = service(&repository);
    let root = &records[0].meta.node_execution_id;
    let mut damaged_ids = ids;
    damaged_ids.push(root.clone());
    let damaged = query.resolve_session_nodes(&workspace, &damaged_ids).await;
    assert_eq!(damaged.sessions.len(), 1);
    assert_eq!(damaged.sessions[root], "dynamic-parent-agent");
    assert_eq!(damaged.failures.len(), 2);
}

#[tokio::test]
pub async fn test_session更新時刻_兄弟の受理済み操作を含め未受理の終了と承認では進まない() {
    use releashd::test_support::integration::workflow::{
        append_single_fact, session_timestamp_facts,
    };
    let records = session_timestamp_facts();
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store.clone());
    let query = service(&repository);
    let workspace = WorkspaceIdentity::new("/repo");
    for record in &records {
        append_single_fact(&store, &record.meta, &record.fact, record.timestamp_ms)
            .await
            .unwrap();
        if record.seq < 8 {
            continue;
        }
        let detail = query
            .session_node_detail(&workspace, &record.meta.tree_id, "node-main")
            .await
            .unwrap()
            .unwrap()
            .detail;
        let list = releashd::test_support::integration::workspace::node_for_execution(
            &*repository,
            &workspace,
            "node-main",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(detail.updated_at, list.updated_at(), "seq {}", record.seq);
        assert_eq!(detail.error_reason, list.failure_reason());
        let expected = match record.seq {
            8 => 8.0,
            9 | 10 => 9.0,
            11..=14 => record.seq as f64,
            15..=18 => 14.0,
            19 => 19.0,
            20 => 20.0,
            21.. => 21.0,
            _ => unreachable!(),
        };
        assert_eq!(detail.updated_at, expected, "seq {}", record.seq);
    }
}
