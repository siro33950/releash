use crate::adaptor_gateway_workflow_workflow_host_test_helpers::archive_fixture;
use crate::adaptor_gateway_workflow_workflow_host_test_helpers::archive_fixture_with_resolver;
use crate::adaptor_gateway_workflow_workflow_host_test_helpers::archive_workflow;
use releashd::test_support::integration::subscriptions::StateChangeSource;
use releashd::test_support::integration::workflow::ExecutionStatus;
use releashd::test_support::integration::workflow::ExecutionTreeArchiveRepository;
use releashd::test_support::integration::workflow::ManagedWorktreeResolver;
use releashd::test_support::integration::workflow::ManagedWorktreeResolverError;
use releashd::test_support::integration::workflow::NodeFact;
use releashd::test_support::integration::workflow::SessionExecutionTreeRootFacts;
use releashd::test_support::integration::workflow::WorkflowError;
use std::sync::Arc;

#[tokio::test]
pub async fn test_実行木archive_abortと自然完了の競合だけを終了状態の再読取で解消する() {
    use releashd::test_support::integration::persistence::LocalEventStore;

    use releashd::test_support::integration::workflow::AbortExecutionCommand;
    use releashd::test_support::integration::workflow::NodeFact;
    use releashd::test_support::integration::workflow::WorkflowAbortExecutionGateway;
    use releashd::test_support::integration::workflow::WorkflowAbortExecutionUsecase;
    use releashd::test_support::integration::workflow::WorkflowError;

    struct CompletingAbort {
        store: Arc<LocalEventStore>,
        complete: bool,
        error: WorkflowError,
    }
    #[async_trait::async_trait]
    impl WorkflowAbortExecutionGateway for CompletingAbort {
        async fn abort_execution(
            &self,
            command: AbortExecutionCommand,
        ) -> Result<(), WorkflowError> {
            if self.complete {
                let records = releashd::test_support::integration::workflow::read_tree_records(
                    &self.store,
                    &command.execution_id,
                )
                .await
                .unwrap();
                let meta = &records[0].meta;
                for kind in ["submit_received", "stop_received"] {
                    releashd::test_support::integration::workflow::append_single_fact(
                        &self.store,
                        meta,
                        &releashd::test_support::integration::workflow::decode(kind, "{}").unwrap(),
                        2000,
                    )
                    .await
                    .unwrap();
                }
            }
            Err(self.error.clone())
        }
    }

    for complete in [false, true] {
        for error in [
            WorkflowError::invalid_state("already terminal"),
            WorkflowError::NotFound("runtime already released".into()),
            WorkflowError::external("abort persistence failed"),
        ] {
            // Given
            let mut fixture = archive_fixture();
            let id = archive_workflow(&fixture).await;
            fixture
                .runtime
                .test_replace_abort_execution(WorkflowAbortExecutionUsecase::new(Arc::new(
                    CompletingAbort {
                        store: fixture.store.clone(),
                        complete,
                        error: error.clone(),
                    },
                )));
            // When
            let result = fixture.runtime.archive_execution_tree(&id).await;
            // Then
            let succeeds = complete && !matches!(error, WorkflowError::External(_));
            if succeeds {
                result.unwrap();
                assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
            } else {
                assert_eq!(result, Err(error));
            }
            assert_eq!(
                fixture.repository.target(&id).await.unwrap().status,
                if complete {
                    ExecutionStatus::Completed
                } else {
                    ExecutionStatus::Running
                }
            );
            let archives = fixture
                .repository
                .archive_snapshot_for(std::slice::from_ref(&id))
                .await
                .unwrap();
            assert_eq!(archives.records.len(), usize::from(succeeds));
            assert!(
                !releashd::test_support::integration::workflow::read_tree_records(
                    &fixture.store,
                    &id
                )
                .await
                .unwrap()
                .iter()
                .any(|record| matches!(record.fact, NodeFact::AbortRequested(_)))
            );
        }
    }
}

#[tokio::test]
pub async fn test_実行木変更の受理_削除との競合と排他記録の保存不能を区別する() {
    use releashd::test_support::integration::repository::FileWorktreeOperationLocks;

    use releashd::test_support::integration::platform::WorktreeOperations;
    // Given
    let mut fixture = archive_fixture();
    let directory = tempfile::tempdir().unwrap();
    fixture
        .runtime
        .test_replace_worktree_operations(Arc::new(WorktreeOperations::new(Arc::new(
            FileWorktreeOperationLocks::new(directory.path()),
        ))));
    let deletion = fixture
        .runtime
        .test_worktree_operations()
        .delete("/repo")
        .await
        .unwrap();
    // When / Then
    assert!(matches!(
        fixture.runtime.begin_worktree_mutation("/repo"),
        Err(WorkflowError::Conflict(_))
    ));
    drop(deletion);
    std::fs::remove_dir_all(directory.path().join("worktree-operations")).unwrap();
    std::fs::write(directory.path().join("worktree-operations"), "unavailable").unwrap();
    assert!(matches!(
        fixture.runtime.begin_worktree_mutation("/repo"),
        Err(WorkflowError::External(_))
    ));
}

#[tokio::test]
pub async fn test_実行木archive_同じworktreeのworkflowと複数sessionをすべて片付ける() {
    use releashd::test_support::integration::workflow::SessionExecutionTreeRootFacts;
    // Given
    let fixture = archive_fixture();
    let workflow = archive_workflow(&fixture).await;
    let ids = [
        "agent-session-00000000000040008000000000000111",
        "agent-session-00000000000040008000000000000112",
    ];
    for id in ids {
        let facts = SessionExecutionTreeRootFacts::new(
            id,
            "/missing/worktree",
            "/missing/worktree",
            releashd::test_support::integration::providers::ProviderKind::Codex,
            None,
        )
        .unwrap();
        releashd::test_support::integration::workflow::append_fact_batch_for_seed(
            &fixture.store,
            &facts.into_facts(),
            1,
            id,
        )
        .unwrap();
        fixture
            .sessions
            .live_sessions
            .lock()
            .unwrap()
            .insert(id.into());
    }
    // When
    fixture
        .runtime
        .archive_worktree("/missing/worktree")
        .await
        .unwrap();
    // Then
    for id in [workflow.as_str(), ids[0], ids[1]] {
        assert_eq!(
            fixture.repository.target(id).await.unwrap().status,
            ExecutionStatus::Aborted
        );
    }
    assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
}

#[tokio::test]
pub async fn test_実行木restore_所属worktreeが利用不可なら事実を追記しない() {
    struct UnavailableWorktree;
    #[async_trait::async_trait]
    impl ManagedWorktreeResolver for UnavailableWorktree {
        async fn resolve(&self, _: String) -> Result<String, ManagedWorktreeResolverError> {
            Err(ManagedWorktreeResolverError::Validation(
                "worktree unavailable".into(),
            ))
        }
    }
    // Given
    let fixture = archive_fixture_with_resolver(Arc::new(UnavailableWorktree));
    let id = archive_workflow(&fixture).await;
    fixture.runtime.archive_execution_tree(&id).await.unwrap();
    let before =
        releashd::test_support::integration::workflow::read_tree_records(&fixture.store, &id)
            .await
            .unwrap();
    // When
    assert!(fixture.runtime.restore_execution_tree(&id).await.is_err());
    // Then
    assert_eq!(
        releashd::test_support::integration::workflow::read_tree_records(&fixture.store, &id)
            .await
            .unwrap(),
        before
    );
}

#[tokio::test]
pub async fn test_終了済み実行木_archiveとrestore成功後だけ所属worktreeの購読を更新する() {
    use releashd::test_support::integration::subscriptions::StateChangeSource;
    // Given
    let mut fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    fixture
        .runtime
        .abort_execution(
            releashd::test_support::integration::workflow::AbortExecutionCommand {
                execution_id: id.clone(),
                expected_node_name: None,
            },
        )
        .await
        .unwrap();
    let publisher = releashd::test_support::integration::subscriptions::test_subscriptions();
    let mut changes = releashd::test_support::integration::subscriptions::changes(&publisher);
    fixture.runtime = fixture.runtime.with_state_publisher(publisher);
    // When / Then
    fixture.runtime.archive_execution_tree(&id).await.unwrap();
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::Worktree("/missing/worktree".into())
    );
    fixture.runtime.restore_execution_tree(&id).await.unwrap();
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::Worktree("/missing/worktree".into())
    );
    assert!(fixture
        .runtime
        .archive_execution_tree("missing")
        .await
        .is_err());
    assert!(fixture
        .runtime
        .restore_execution_tree("missing")
        .await
        .is_err());
    assert!(changes.try_recv().is_err());
}

#[tokio::test]
pub async fn test_実行木archiveとrestore_workspace識別子と異なるworktreeパスを通知する() {
    // Given
    let mut fixture = archive_fixture();
    let id = "agent-session-00000000000040008000000000000113";
    let facts = SessionExecutionTreeRootFacts::new(
        id,
        "/workspace",
        "/missing/worktree",
        releashd::test_support::integration::providers::ProviderKind::Codex,
        None,
    )
    .unwrap();
    let meta = facts.meta.clone();
    releashd::test_support::integration::workflow::append_fact_batch_for_seed(
        &fixture.store,
        &facts.into_facts(),
        1,
        id,
    )
    .unwrap();
    releashd::test_support::integration::workflow::append_single_fact(
        &fixture.store,
        &meta,
        &NodeFact::AbortRequested(Default::default()),
        2,
    )
    .await
    .unwrap();
    let target = fixture.repository.target(id).await.unwrap();
    assert_ne!(target.workspace_identity, target.worktree_path);
    let publisher = releashd::test_support::integration::subscriptions::test_subscriptions();
    let mut changes = releashd::test_support::integration::subscriptions::changes(&publisher);
    fixture.runtime = fixture.runtime.with_state_publisher(publisher);
    // When / Then
    fixture.runtime.archive_execution_tree(id).await.unwrap();
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::Worktree(target.worktree_path.clone())
    );
    fixture.runtime.restore_execution_tree(id).await.unwrap();
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::Worktree(target.worktree_path)
    );
    assert!(changes.try_recv().is_err());
}

#[tokio::test]
async fn test_実行木archiveとrestore_sessionタブを増減して選択を保つ() {
    use releashd::test_support::integration::platform::{
        PaneLayout, PaneTab, PaneTabKind, StreamVersion, WorkspaceState,
    };
    use releashd::test_support::integration::repository::WorkspaceStateRepository;
    use releashd::test_support::integration::workspace::{
        WorkspaceIdentity, WorkspaceTreeRepository,
    };
    // Given
    let fixture = archive_fixture();
    let execution_id = archive_workflow(&fixture).await;
    let target = fixture.repository.target(&execution_id).await.unwrap();
    let tree = fixture
        .trees
        .load_execution_tree(
            &WorkspaceIdentity::new(&target.workspace_identity),
            &execution_id,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(tree.nodes().iter().any(|node| node.session_id.is_some()));
    let selected = PaneTab {
        id: "selected".into(),
        kind: PaneTabKind::Terminal,
        node_id: None,
        agent_session_id: None,
        file_path: None,
        file_mode: None,
        file_base: None,
    };
    let mut state = WorkspaceState {
        panes: Some(
            PaneLayout::pane(
                "pane".into(),
                vec![selected.clone()],
                Some(selected.id.clone()),
            )
            .unwrap(),
        ),
        ..WorkspaceState::default()
    };
    state.add_execution_tabs(&tree);
    fixture
        .workspace_states
        .save(
            &target.workspace_identity,
            state.clone(),
            StreamVersion::zero(),
        )
        .await
        .unwrap();
    // When
    fixture
        .runtime
        .archive_execution_tree(&execution_id)
        .await
        .unwrap();
    // Then
    let (_, archived) = fixture
        .workspace_states
        .read(&target.workspace_identity)
        .await
        .unwrap();
    let mut expected = state.clone();
    expected.panes =
        Some(PaneLayout::pane("pane".into(), vec![selected], Some("selected".into())).unwrap());
    assert_eq!(archived, Some(expected));
    // When
    fixture
        .runtime
        .restore_execution_tree(&execution_id)
        .await
        .unwrap();
    // Then
    let (_, restored) = fixture
        .workspace_states
        .read(&target.workspace_identity)
        .await
        .unwrap();
    assert_eq!(restored, Some(state));
}

#[tokio::test]
async fn test_実行木archiveとrestore_sessionタブ保存に失敗しても所属worktreeの購読を更新し失敗を返す(
) {
    use releashd::test_support::integration::platform::{
        StreamVersion, TechnicalFailure, TechnicalFailureNature, WorkspaceState,
        WorkspaceStateError,
    };
    use releashd::test_support::integration::repository::WorkspaceStateRepository;
    use releashd::test_support::integration::workspace::{
        WorkspaceIdentity, WorkspaceTreeRepository,
    };
    // Given
    let mut fixture = archive_fixture();
    let execution_id = archive_workflow(&fixture).await;
    let target = fixture.repository.target(&execution_id).await.unwrap();
    let tree = fixture
        .trees
        .load_execution_tree(
            &WorkspaceIdentity::new(&target.workspace_identity),
            &execution_id,
        )
        .await
        .unwrap()
        .unwrap();
    let mut with_tabs = WorkspaceState::default();
    with_tabs.add_execution_tabs(&tree);
    assert_ne!(with_tabs, WorkspaceState::default());
    fixture
        .workspace_states
        .save(
            &target.workspace_identity,
            with_tabs.clone(),
            StreamVersion::zero(),
        )
        .await
        .unwrap();
    let publisher = releashd::test_support::integration::subscriptions::test_subscriptions();
    let mut changes = releashd::test_support::integration::subscriptions::changes(&publisher);
    fixture.runtime = fixture.runtime.with_state_publisher(publisher);
    let tab_failure = || {
        WorkspaceStateError::Technical(TechnicalFailure {
            nature: TechnicalFailureNature::Other,
            message: "tab save failed".into(),
        })
    };
    // When
    *fixture.workspace_states.next_save_error.lock() = Some(tab_failure());
    let archived = fixture.runtime.archive_execution_tree(&execution_id).await;
    // Then
    assert!(matches!(archived, Err(WorkflowError::Technical(_))));
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::Worktree(target.worktree_path.clone())
    );
    let (version, unchanged) = fixture
        .workspace_states
        .read(&target.workspace_identity)
        .await
        .unwrap();
    assert_eq!(unchanged, Some(with_tabs));
    // Given
    fixture
        .workspace_states
        .save(
            &target.workspace_identity,
            WorkspaceState::default(),
            version,
        )
        .await
        .unwrap();
    // When
    *fixture.workspace_states.next_save_error.lock() = Some(tab_failure());
    let restored = fixture.runtime.restore_execution_tree(&execution_id).await;
    // Then
    assert!(matches!(restored, Err(WorkflowError::Technical(_))));
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::Worktree(target.worktree_path)
    );
    let (_, unchanged) = fixture
        .workspace_states
        .read(&target.workspace_identity)
        .await
        .unwrap();
    assert_eq!(unchanged, Some(WorkspaceState::default()));
    assert!(changes.try_recv().is_err());
}
