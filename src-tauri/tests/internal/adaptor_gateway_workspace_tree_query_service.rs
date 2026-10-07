use releash_lib::test_support::integration::persistence::LocalEventStore;
use releash_lib::test_support::integration::persistence::LocalEventStoreConfig;
use releash_lib::test_support::integration::providers::ProviderKind;
use releash_lib::test_support::integration::sessions::AgentSession;
use releash_lib::test_support::integration::sessions::AgentSessionRecoveryResult;
use releash_lib::test_support::integration::sessions::AgentSessionRepository;
use releash_lib::test_support::integration::sessions::AgentSessionTreeLocation;
use releash_lib::test_support::integration::sessions::AgentSessionUsecase;
use releash_lib::test_support::integration::sessions::LocalAgentSessionRepository;
use releash_lib::test_support::integration::workflow::seed_workflow_session_facts;
use releash_lib::test_support::integration::workflow::AgentSessionActivity;
use releash_lib::test_support::integration::workflow::ExecutionStatus;
use releash_lib::test_support::integration::workflow::ExecutionTreeLaunch;
use releash_lib::test_support::integration::workflow::NodeFact;
use releash_lib::test_support::integration::workflow::StopReceivedFact;
use releash_lib::test_support::integration::workflow::WorkflowSessionFactSeed;
use releash_lib::test_support::integration::workflow::WorkspaceNodeContentDto;
use releash_lib::test_support::integration::workspace::SqliteWorkspaceQueryService;
use releash_lib::test_support::integration::workspace::SqliteWorkspaceTreeRepository;
use releash_lib::test_support::integration::workspace::WorkspaceIdentity;
use releash_lib::test_support::integration::workspace::WorkspaceNodeKind;
use releash_lib::test_support::integration::workspace::WorkspaceQueryService;
use releash_lib::test_support::integration::workspace::WorkspaceTree;
use releash_lib::test_support::integration::workspace::WorkspaceTreeRepository;
use releash_lib::test_support::integration::workspace::WorkspaceVisibleNode;
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
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
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
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
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
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
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
        releash_lib::test_support::integration::workflow::read_tree_records(&store, session_id)
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
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
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
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
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
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
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
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
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
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
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
    let meta =
        releash_lib::test_support::integration::workflow::read_tree_records(&store, session_id)
            .await
            .unwrap()
            .last()
            .unwrap()
            .meta
            .clone();
    let append_stop = async |timestamp_ms| {
        releash_lib::test_support::integration::workflow::append_single_fact(
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
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
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
            releash_lib::test_support::integration::workflow::append_single_fact(
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
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
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
        .execution_summaries(Some(&WorkspaceIdentity::new("/repo")), None, None)
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
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
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
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
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
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
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
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
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
    let child_node = releash_lib::test_support::integration::workspace::node_for_execution(
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
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ))
        .unwrap();
        releash_lib::test_support::integration::workflow::seed_unavailable_definition(
            &store,
            "00000000-0000-4000-8000-000000001744",
            "/repo",
            unavailable,
        )
        .await;
        releash_lib::test_support::integration::workflow::seed_unavailable_definition(
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
            releash_lib::test_support::integration::workflow::append_single_fact(
                &store,
                &releash_lib::test_support::integration::workflow::NodeFactMeta {
                    tree_id: tree.into(),
                    node_execution_id: tree.into(),
                    parent_id: None,
                    node_name: "main".into(),
                    kind: releash_lib::test_support::integration::workflow::NodeKindName::Sequence,
                    attempt: 1,
                },
                &releash_lib::test_support::integration::workflow::NodeFact::AbortRequested(
                    Default::default(),
                ),
                10_000,
            )
            .await
            .unwrap();
        }
        let read_store =
            releash_lib::test_support::integration::persistence::LocalEventReadStore::open(
                directory.path(),
                std::sync::Arc::new(
                    releash_lib::test_support::integration::platform::RetryLimiter::new(),
                ),
            )
            .unwrap();
        for repository in [
            SqliteWorkspaceTreeRepository::new(store.clone()),
            SqliteWorkspaceTreeRepository::new_read_only(read_store),
        ] {
            let command = releash_lib::test_support::integration::workspace::node_for_execution(
                &*repository,
                &releash_lib::test_support::integration::workspace::WorkspaceIdentity::new("/repo"),
                "00000000-0000-4000-8000-000000001744-command",
            )
            .await
            .unwrap()
            .unwrap();
            let session = releash_lib::test_support::integration::workspace::node_for_execution(
                &*repository,
                &releash_lib::test_support::integration::workspace::WorkspaceIdentity::new("/repo"),
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
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
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
    use releash_lib::test_support::integration::workflow::ExecutionTreeArchiveFactRepository;
    use releash_lib::test_support::integration::workflow::ExecutionTreeArchiveRepository;
    let root =
        releash_lib::test_support::integration::workflow::read_tree_records(store, execution_id)
            .await
            .unwrap()
            .remove(0);
    releash_lib::test_support::integration::workflow::append_single_fact(
        store,
        &root.meta,
        &NodeFact::AbortRequested(Default::default()),
        2,
    )
    .await
    .unwrap();
    ExecutionTreeArchiveFactRepository::new(store.clone(), data_dir)
        .archive(
            &releash_lib::test_support::integration::workflow::ExecutionTreeId::new(execution_id)
                .unwrap(),
            archived_at,
            reason,
        )
        .await
        .unwrap();
}

#[tokio::test]
pub async fn test_workflow単一取得_単独sessionをworkflow_summaryとして返さない() {
    use releash_lib::test_support::integration::workflow::SessionExecutionTreeRootFacts;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let session = "00000000-0000-4000-8000-000000000991";
    let workflow = "00000000-0000-4000-8000-000000000992";
    let facts =
        SessionExecutionTreeRootFacts::new(session, "/repo", "/repo", ProviderKind::Codex, None)
            .unwrap();
    releash_lib::test_support::integration::workflow::append_fact_batch_for_seed(
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

#[tokio::test]
pub async fn test_workspace読取_実経路で失敗分類を保持する() {
    use releash_lib::test_support::integration::persistence::ReadFailure;
    use releash_lib::test_support::integration::transport::classified_error;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store.clone());
    let query = service(&repository);
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
                repository
                    .load_trees(std::slice::from_ref(&workspace))
                    .await
                    .pop()
                    .unwrap()
                    .map(|_| ())
            } else {
                query.execution_records(None, None, None).await.map(|_| ())
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
        Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
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
