use std::sync::Arc;

use crate::test_support_git::create_initial_commit;
use crate::test_support_git::create_test_repo;
use releash_lib::test_support::integration::platform::build_repository_usecase;
use releash_lib::test_support::integration::platform::build_workflow_usecase_and_store;
use releash_lib::test_support::integration::platform::Fetched;
use releash_lib::test_support::integration::providers::ProviderKind;
use releash_lib::test_support::integration::workflow::RepoPathsManagedWorktreeGateway;
use releash_lib::test_support::integration::workflow::SessionExecutionTreeRootFacts;
use releash_lib::test_support::integration::workflow::WorkflowError;
use releash_lib::test_support::integration::workflow::WorkflowUsecase;
use releash_lib::test_support::integration::workspace::WorkspaceIdentity;
use releash_lib::test_support::integration::workspace::WorkspaceTree;

#[tokio::test]
pub async fn test_archive_restore認可_別名は受理し非管理対象と別worktreeは拒否する() {
    // Given
    let (repo_dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    let directory = tempfile::tempdir().unwrap();
    let worktree = directory.path().join("managed-worktree");
    repo.worktree("managed-worktree", &worktree, None).unwrap();
    let worktree = worktree.canonicalize().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_path = outside.path().canonicalize().unwrap();
    let managed_id = "agent-session-00000000000040008000000000000001";
    let unmanaged_id = "agent-session-00000000000040008000000000000002";
    let (mut usecase, store) =
        build_workflow_usecase_and_store(directory.path().join("data"), None);
    usecase.test_replace_managed_worktree_gateway(Arc::new(RepoPathsManagedWorktreeGateway::new(
        Arc::new(build_repository_usecase()),
        vec![repo_dir.path().to_str().unwrap().to_string()],
    )));
    for (id, path) in [(managed_id, &worktree), (unmanaged_id, &outside_path)] {
        let path = path.to_str().unwrap();
        let facts =
            SessionExecutionTreeRootFacts::new(id, path, path, ProviderKind::Codex, None).unwrap();
        releash_lib::test_support::integration::workflow::append_fact_batch_for_seed(
            &store,
            &facts.into_facts(),
            1,
            id,
        )
        .unwrap();
    }

    // When / Then
    for (path, id, expected_error) in [
        (worktree.join("."), managed_id, None),
        (worktree.clone(), managed_id, None),
        (
            repo_dir.path().to_path_buf(),
            managed_id,
            Some("execution tree worktree does not match"),
        ),
        (
            outside_path,
            unmanaged_id,
            Some("worktree_path is not a configured git worktree"),
        ),
    ] {
        let result = usecase
            .authorize_archive_target(path.to_str().unwrap(), id)
            .await;
        match expected_error {
            Some(message) => assert!(result.unwrap_err().to_string().contains(message)),
            None => result.unwrap(),
        }
    }
}

/// worktree ごとに、読めた実行木か失敗を返す。
#[derive(Default)]
struct Trees {
    results:
        parking_lot::Mutex<std::collections::HashMap<String, Result<WorkspaceTree, WorkflowError>>>,
}

impl Trees {
    fn set(&self, path: &str, result: Result<WorkspaceTree, WorkflowError>) {
        self.results.lock().insert(path.to_string(), result);
    }
}

#[async_trait::async_trait]
impl releash_lib::test_support::integration::workspace::WorkspaceTreeRepository for Trees {
    async fn load_trees(
        &self,
        workspace_identities: &[WorkspaceIdentity],
    ) -> Vec<Result<WorkspaceTree, WorkflowError>> {
        let results = self.results.lock();
        workspace_identities
            .iter()
            .map(|identity| results[identity.as_str()].clone())
            .collect()
    }

    async fn load_node_by_session_id(
        &self,
        _: &releash_lib::test_support::integration::workspace::WorkspaceIdentity,
        _: &str,
    ) -> Result<
        Option<releash_lib::test_support::integration::workspace::WorkspaceTreeNode>,
        releash_lib::test_support::integration::platform::LocalEventQueryError,
    > {
        Ok(None)
    }

    async fn load_node(
        &self,
        _: &WorkspaceIdentity,
        _: &str,
    ) -> Result<
        Option<releash_lib::test_support::integration::workspace::WorkspaceTreeNode>,
        releash_lib::test_support::integration::platform::LocalEventQueryError,
    > {
        Ok(None)
    }

    async fn load_node_by_node_execution_id(
        &self,
        _: &str,
    ) -> Result<
        Option<releash_lib::test_support::integration::workspace::WorkspaceTreeNode>,
        releash_lib::test_support::integration::platform::LocalEventQueryError,
    > {
        Ok(None)
    }
}

#[tokio::test]
pub async fn test_一覧の実行木_初回読取失敗を未取得と区別する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let (mut usecase, _store) =
        build_workflow_usecase_and_store(directory.path().join("data"), None);
    let trees = Arc::new(Trees::default());
    usecase.test_replace_workspace_tree_repository(trees.clone());
    // 一覧が渡す場所は走査で読めたものなので、フォルダが無くても（削除中でも）読む。
    let paths = [
        "/repo-worktrees/a".to_string(),
        "/repo-worktrees/b".to_string(),
    ];
    let (a, b) = (paths[0].as_str(), paths[1].as_str());
    trees.set(a, Ok(WorkspaceTree::empty(a)));
    trees.set(b, Err(WorkflowError::external("store busy")));

    // When
    let first = usecase.retained_workspace_trees(&paths).await;

    // Then
    assert_eq!(first[0], Fetched::ready(WorkspaceTree::empty(a)));
    assert!(!first[1].loaded());
    assert!(first[1]
        .error
        .as_ref()
        .map(|failure| failure.message.as_str())
        .unwrap()
        .contains("store busy"));
}

#[tokio::test]
pub async fn test_一覧の実行木_取り直しの失敗で前の木を残し回復した木の失敗を解除する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let (mut usecase, _store) =
        build_workflow_usecase_and_store(directory.path().join("data"), None);
    let trees = Arc::new(Trees::default());
    usecase.test_replace_workspace_tree_repository(trees.clone());
    // 一覧が渡す場所は走査で読めたものなので、フォルダが無くても（削除中でも）読む。
    let paths = [
        "/repo-worktrees/a".to_string(),
        "/repo-worktrees/b".to_string(),
    ];
    let (a, b) = (paths[0].as_str(), paths[1].as_str());
    trees.set(a, Ok(WorkspaceTree::empty(a)));
    trees.set(b, Err(WorkflowError::external("store busy")));

    usecase.retained_workspace_trees(&paths).await;
    // When
    trees.set(a, Err(WorkflowError::external("store busy")));
    trees.set(b, Ok(WorkspaceTree::empty(b)));
    let second = usecase.retained_workspace_trees(&paths).await;

    // Then
    assert_eq!(second[0].value, Some(WorkspaceTree::empty(a)));
    assert!(second[0]
        .error
        .as_ref()
        .map(|failure| failure.message.as_str())
        .unwrap()
        .contains("store busy"));
    assert_eq!(second[1], Fetched::ready(WorkspaceTree::empty(b)));
}

#[tokio::test]
pub async fn test_一覧の実行木_一覧から外れた木の保持を捨てる() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let (mut usecase, _store) =
        build_workflow_usecase_and_store(directory.path().join("data"), None);
    let trees = Arc::new(Trees::default());
    usecase.test_replace_workspace_tree_repository(trees.clone());
    // 一覧が渡す場所は走査で読めたものなので、フォルダが無くても（削除中でも）読む。
    let paths = [
        "/repo-worktrees/a".to_string(),
        "/repo-worktrees/b".to_string(),
    ];
    let (a, b) = (paths[0].as_str(), paths[1].as_str());
    trees.set(a, Ok(WorkspaceTree::empty(a)));
    trees.set(b, Err(WorkflowError::external("store busy")));

    usecase.retained_workspace_trees(&paths).await;
    trees.set(a, Err(WorkflowError::external("store busy")));
    trees.set(b, Ok(WorkspaceTree::empty(b)));
    usecase.retained_workspace_trees(&paths).await;

    // When
    usecase.retained_workspace_trees(&paths[1..]).await;
    let returned = usecase.retained_workspace_trees(&paths).await;

    // Then
    assert!(!returned[0].loaded());
    assert!(returned[0].error.is_some());
}

/// 実行中の session を 1 つ持つ workflow を、実在する worktree に作る。worktree の場所を返す。
async fn seeded_workflow(
    failures: Arc<releash_lib::test_support::integration::platform::FailureRecordStore>,
    execution_id: &str,
) -> (tempfile::TempDir, WorkflowUsecase, String) {
    use releash_lib::test_support::integration::workflow::seed_workflow_session_facts;
    use releash_lib::test_support::integration::workflow::WorkflowSessionFactSeed;
    let directory = tempfile::tempdir().unwrap();
    let (mut usecase, store) =
        build_workflow_usecase_and_store(directory.path().join("data"), None);
    usecase.test_replace_failure_repository(failures);
    let worktree = directory
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    seed_workflow_session_facts(
        &store,
        WorkflowSessionFactSeed {
            workflow_name: "background-failure",
            request: "test",
            worktree_path: &worktree,
            provider: ProviderKind::Codex,
            workflow_execution_id: execution_id,
            node_execution_id: "workflow-node",
            session_id: "workflow-session",
            initial_instruction_admitted: true,
        },
    )
    .await
    .unwrap();
    releash_lib::test_support::integration::sessions::AgentSessionUsecase::new(Arc::new(
        releash_lib::test_support::integration::sessions::LocalAgentSessionRepository::new(store),
    ))
    .observe_activity(
        "workflow-session",
        releash_lib::test_support::integration::workflow::AgentSessionActivity::Working,
        "observe-working",
    )
    .await
    .unwrap();
    (directory, usecase, worktree)
}

#[tokio::test]
pub async fn test_workspaceツリー_背景失敗の対象と理由を表示し成功後は解除する() {
    use releash_lib::test_support::integration::platform::BusinessFailure;
    use releash_lib::test_support::integration::platform::Failure;
    use releash_lib::test_support::integration::platform::FailureKey;
    use releash_lib::test_support::integration::platform::TechnicalFailureNature;
    use releash_lib::test_support::integration::platform::WorkFailure;

    // Given
    let execution_id = "00000000-0000-4000-8000-000000001701";
    let failures =
        Arc::new(releash_lib::test_support::integration::platform::FailureRecordStore::default());
    let (_directory, usecase, worktree) = seeded_workflow(failures.clone(), execution_id).await;
    let statuses = |tree: &WorkspaceTree| {
        let visible = tree.visible();
        let root = visible.roots()[0];
        let node = root.children()[0];
        (
            root.node().status_classification.as_public_str(),
            node.node().status_classification.as_public_str(),
            node.node().error_reason.clone(),
            node.id().to_string(),
        )
    };
    let before = usecase.workspace_tree(&worktree).await.unwrap();
    let (_, node_status, error_reason, node_id) = statuses(&before);
    assert_eq!(node_status, "active");
    assert!(error_reason.is_none());
    for target in [node_id.as_str(), "workflow-node", execution_id] {
        let key = FailureKey::new("workflow_recovery", target);
        // When / Then
        failures.observe(
            &key,
            WorkFailure {
                kind: Failure::Technical(TechnicalFailureNature::Cancelled),
                message: "cancelled".into(),
            },
        );
        assert_eq!(usecase.workspace_tree(&worktree).await.unwrap(), before);
        failures.observe(
            &key,
            WorkFailure {
                kind: Failure::Business(BusinessFailure::Other),
                message: "repair required".into(),
            },
        );
        let failed = usecase.workspace_tree(&worktree).await.unwrap();
        let (root_status, node_status, error_reason, _) = statuses(&failed);
        assert_eq!(root_status, "attention");
        assert_eq!(node_status, "attention");
        assert_eq!(error_reason.as_deref(), Some("repair required"));
        failures.resolve(&key);
        assert_eq!(usecase.workspace_tree(&worktree).await.unwrap(), before);
    }
}

#[tokio::test]
pub async fn test_実行木の選択_選択先が画面に出す木にあるかを返す() {
    // Given
    let execution_id = "00000000-0000-4000-8000-000000001702";
    let (_directory, usecase, worktree) = seeded_workflow(Default::default(), execution_id).await;

    let node_id = usecase
        .workspace_tree(&worktree)
        .await
        .unwrap()
        .visible()
        .roots()[0]
        .children()[0]
        .id()
        .to_string();

    // When
    let (tree, selected) = usecase
        .workspace_tree_selection(&worktree, &node_id)
        .await
        .unwrap();
    let (missing_tree, missing) = usecase
        .workspace_tree_selection(&worktree, "removed-node")
        .await
        .unwrap();

    // Then: 選択先にできるのは葉の Node で、無くなった Node は木に無いと返る
    assert!(selected);
    assert!(!missing);
    assert_eq!(tree, missing_tree);
    assert_eq!(tree.visible().roots()[0].id(), execution_id);
}
