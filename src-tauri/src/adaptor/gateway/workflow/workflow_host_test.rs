use super::test_helpers::{archive_fixture, archive_workflow};
use super::test_helpers::{TestSessions, TestWorktrees};
use super::workflow_host_tests::{AcceptingWorktreeResolver, UnusedWorkflowResolver};
use super::*;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::{
    ExecutionTreeArchiveFactRepository, WorkflowRuntimeCommandGateway,
};
use crate::adaptor::gateway::workspace_tree::{
    SqliteWorkspaceQueryService, SqliteWorkspaceTreeRepository,
};
use crate::usecase::provider_lifecycle::ProviderExecutionTreeStopCommand;
use crate::usecase::workflow::command::SubmitOutputCommand;
use crate::usecase::workflow::control_plane::WorkflowControlPlaneUsecase;

#[tokio::test]
async fn test_実行木archive_状態確認後の自然完了で再登録できなくても終了状態を保って隠す() {
    use crate::domain::workflow::{ExecutionTreeArchiveRepository, NodeFact};
    // Given
    let fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    let records = workflow_fact_log::read_tree_records(&fixture.store, &id).unwrap();
    let meta = &records[0].meta;
    let commit_lock = fixture.host.commit_lock(&id).await;
    let commit_guard = commit_lock.lock().await;
    let mut archive = Box::pin(fixture.runtime.archive_execution_tree(&id, "manual"));
    assert!(futures_util::poll!(archive.as_mut()).is_pending());
    // When
    for kind in ["submit_received", "stop_received"] {
        workflow_fact_log::append_single_fact(
            &fixture.store,
            meta,
            &super::fact_codec::decode(kind, "{}").unwrap(),
            2000,
        )
        .unwrap();
    }
    drop(commit_guard);
    archive.await.unwrap();
    // Then
    assert_eq!(
        fixture.repository.target(&id).unwrap().status,
        ExecutionStatus::Completed
    );
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(&[id.clone()])
            .unwrap()
            .records
            .len(),
        1
    );
    assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
    assert!(!workflow_fact_log::read_tree_records(&fixture.store, &id)
        .unwrap()
        .iter()
        .any(|record| matches!(record.fact, NodeFact::AbortRequested(_))));
}

#[tokio::test]
async fn test_起動時recovery_gcのabortと直列化しarchive後に実行木もプロセスも復元しない() {
    use crate::domain::workflow::{
        ExecutionTreeArchiveRepository, NodeFact, NodeFactMeta, StartedFact, TreeRootFact,
    };
    use crate::usecase::app_data_gc::ExecutionTreeGc;

    for (launched_as, kind, node) in [
        (
            ExecutionTreeLaunch::Workflow,
            NodeKindName::Command,
            "command: 'must-not-start'",
        ),
        (
            ExecutionTreeLaunch::Session,
            NodeKindName::Session,
            "session: {provider: codex}",
        ),
    ] {
        // Given
        let fixture = archive_fixture();
        let id = "00000000-0000-4000-8000-000000000997";
        let definition = serde_saphyr::from_str(&format!(
            "name: recovery\ndescription: test\nnodes:\n  main: {{{node}}}"
        ))
        .unwrap();
        let definition =
            crate::adaptor::gateway::workflow::mapper::schema_workflow_to_domain(definition)
                .unwrap();
        let fact = NodeFact::Started(StartedFact {
            worktree: None,
            parent: None,
            root: Some(Box::new(TreeRootFact {
                repository_root: Some("/repo".into()),
                workspace_identity: "/missing/worktree".into(),
                worktree_path: "/missing/worktree".into(),
                created_from: ExecutionOrigin::Cli,
                request: String::new(),
                workflow_name: definition.name.clone(),
                definition: Some(definition),
                launched_as,
            })),
        });
        workflow_fact_log::append_fact_batch_for_seed(
            &fixture.store,
            &[(
                NodeFactMeta {
                    tree_id: id.into(),
                    node_execution_id: id.into(),
                    parent_id: None,
                    node_name: "main".into(),
                    kind,
                    attempt: 1,
                },
                fact,
            )],
            1,
            id,
        )
        .unwrap();
        let activation_gate = fixture.host.runtime_activation_gate(id).await;
        let activation_guard = activation_gate.lock.lock().await;
        let mut recovery = Box::pin(test_helpers::reconcile_startup(&fixture.host, &fixture.app));
        assert!(futures_util::poll!(recovery.as_mut()).is_pending());

        // When
        let mut archive = Box::pin(fixture.runtime.archive_removed_tree(id));
        assert!(futures_util::poll!(archive.as_mut()).is_pending());
        assert!(fixture
            .repository
            .archive_snapshot_for(&[id.into()])
            .unwrap()
            .records
            .is_empty());
        drop(activation_guard);
        let (recovered, archived) =
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                tokio::join!(recovery, archive)
            })
            .await
            .unwrap();
        recovered.unwrap();
        archived.unwrap();
        test_helpers::reconcile_startup(&fixture.host, &fixture.app)
            .await
            .unwrap();

        // Then
        assert_eq!(
            fixture.repository.target(id).unwrap().status,
            ExecutionStatus::Aborted
        );
        assert_eq!(
            fixture
                .repository
                .archive_snapshot_for(&[id.into()])
                .unwrap()
                .records[0]
                .archive_reason,
            "worktree_removed"
        );
        assert!(fixture.sessions.prepared.lock().unwrap().is_empty());
        assert!(fixture.sessions.activated.lock().unwrap().is_empty());
        assert!(fixture.sessions.recovered.lock().unwrap().is_empty());
        assert!(fixture
            .host
            .node_processes
            .active_commands
            .lock()
            .unwrap()
            .is_empty());
        let facts = workflow_fact_log::read_tree_records(&fixture.store, id).unwrap();
        assert!(!facts.iter().any(|record| matches!(
            record.fact,
            NodeFact::CommandSpawned(_) | NodeFact::SessionAttached(_)
        )));
    }
}

#[tokio::test]
async fn test_起動時recovery_起動済みの実行木のプロセスを再起動しない() {
    // Given
    let fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    let before = workflow_fact_log::read_tree_records(&fixture.store, &id).unwrap();
    // When
    test_helpers::reconcile_startup(&fixture.host, &fixture.app)
        .await
        .unwrap();
    // Then
    assert_eq!(
        workflow_fact_log::read_tree_records(&fixture.store, &id).unwrap(),
        before
    );
}

#[tokio::test]
async fn test_起動時recovery_通常起動と同じsessionを一度だけ起動する() {
    use crate::domain::workflow::NodeFact;
    // Given
    let fixture = archive_fixture();
    let runtime = fixture.runtime.clone().with_startup(
        crate::adaptor::controller::wiring::wire_workflow_startup(
            fixture.app.clone(),
            fixture.host.clone(),
        ),
    );
    let mut gates = fixture.host.runtime_activation_locks.lock().await;
    let mut start = Box::pin(archive_workflow(&fixture));
    assert!(futures_util::poll!(start.as_mut()).is_pending());
    let id = workflow_fact_log::list_tree_ids(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        None,
    )
    .unwrap()
    .remove(0);
    let gate = Arc::new(RuntimeActivationGate::new());
    let guard = gate.lock.lock().await;
    gates.insert(id.clone(), Arc::downgrade(&gate));
    drop(gates);
    assert!(futures_util::poll!(start.as_mut()).is_pending());
    let mut recovery = Box::pin(runtime.recover_startup());
    assert!(futures_util::poll!(recovery.as_mut()).is_pending());

    // When
    drop(guard);
    let (started, recovered) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(start, recovery)
    })
    .await
    .unwrap();
    assert_eq!(started, id);
    recovered.unwrap();

    // Then
    assert_eq!(fixture.sessions.prepared.lock().unwrap().len(), 1);
    assert_eq!(fixture.sessions.activated.lock().unwrap().len(), 1);
    assert_eq!(fixture.sessions.live_sessions.lock().unwrap().len(), 1);
    assert!(fixture
        .host
        .load_execution(&fixture.app, &id)
        .await
        .unwrap()
        .is_active());
    let records = workflow_fact_log::read_tree_records(&fixture.store, &id).unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(record.fact, NodeFact::SessionAttached(_)))
            .count(),
        1
    );
    assert!(!records.iter().any(|record| matches!(
        record.fact,
        NodeFact::RuntimeFailureObserved(_) | NodeFact::AbortRequested(_)
    )));
}

#[tokio::test]
async fn test_起動時recovery_回復が先に起動したsessionへの古い起動要求を無視する() {
    // Given
    let fixture = test_helpers::Fixture::new(0);
    let snapshot = fixture
        .persist_started(
            "  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}",
            "/repo",
        )
        .await;
    let execution = fixture
        .host
        .load_execution(&fixture.app, &snapshot.execution_id)
        .await
        .unwrap();
    let leaf = execution
        .leaf_start_for(&execution.node_executions[0].id)
        .unwrap();
    fixture
        .sessions
        .block_preparation
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let mut recovery = Box::pin(test_helpers::reconcile_startup(&fixture.host, &fixture.app));
    assert!(futures_util::poll!(recovery.as_mut()).is_pending());
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        fixture.sessions.preparation_entered.notified(),
    )
    .await
    .unwrap();

    // When
    let mut start = Box::pin(fixture.host.start_nodes(
        &fixture.app,
        &snapshot.execution_id,
        "/repo",
        vec![NodeStart::Leaf(leaf)],
    ));
    assert!(futures_util::poll!(start.as_mut()).is_pending());
    fixture.sessions.preparation_release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(5), recovery)
        .await
        .unwrap()
        .unwrap();
    let before =
        workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), start)
        .await
        .unwrap()
        .unwrap();

    // Then
    assert_eq!(fixture.sessions.prepared.lock().unwrap().len(), 1);
    assert_eq!(fixture.sessions.activated.lock().unwrap().len(), 1);
    assert_eq!(
        workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap(),
        before
    );
}

#[tokio::test]
async fn test_workflow永続化_本番構成で起動から完了とabortまで状態ファイルを作らない() {
    for status in [ExecutionStatus::Completed, ExecutionStatus::Aborted] {
        // Given
        let directory = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into()))
                .unwrap();
        let app = test_helpers::dependencies(Some(store.clone()));
        let query = SqliteWorkspaceQueryService::with_repository(
            SqliteWorkspaceTreeRepository::new(store.clone()),
            Arc::new(ExecutionTreeArchiveFactRepository::new(
                store.clone(),
                directory.path(),
            )),
        );
        let host = Arc::new(WorkflowRuntimeHost::with_runtime_ports(
            Arc::new(UnusedWorkflowResolver),
            Arc::new(AcceptingWorktreeResolver),
            query.clone(),
            Arc::new(TestSessions::default()),
            Arc::new(TestWorktrees::default()),
        ));
        let workflow = serde_saphyr::from_str(
            "name: persistence\ndescription: test\nnodes:\n  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}",
        ).unwrap();

        // When
        let execution_id = host
            .start_resolved_workflow(
                &app,
                workflow,
                directory.path().to_string_lossy().into_owned(),
                None,
                ExecutionOrigin::Cli,
            )
            .await
            .unwrap();
        let snapshot = host
            .get_state_by_execution_id(&app, &execution_id)
            .await
            .unwrap();

        // Then
        assert_eq!(snapshot.state, RuntimeExecutionState::Running);
        assert!(!directory.path().join("workflow_executions").exists());

        // When
        if status == ExecutionStatus::Aborted {
            host.abort_workflow_execution(&app, &execution_id, None)
                .await
                .unwrap();
        } else {
            let node = &snapshot.node_executions[0];
            let control = WorkflowControlPlaneUsecase::new(Arc::new(
                WorkflowRuntimeCommandGateway::new_with_driver(app, host),
            ));
            control
                .submit_output(SubmitOutputCommand {
                    node_execution_id: node.id.clone(),
                    artifact: None,
                })
                .await
                .unwrap();
            control
                .record_provider_stop(
                    ProviderExecutionTreeStopCommand {
                        agent_session_id: node.session_id.clone().unwrap(),
                        tree_id: execution_id.clone(),
                        node_execution_id: node.id.clone(),
                        binding_id: "binding-persistence-test".into(),
                    },
                    Vec::new(),
                )
                .await
                .unwrap();
        }

        // Then
        let record = crate::usecase::workspace_tree::WorkspaceQueryService::execution_summary(
            query.as_ref(),
            &execution_id,
        )
        .unwrap()
        .unwrap();
        assert_eq!(record.status, status);
        let reloaded = crate::usecase::workspace_tree::WorkspaceQueryService::execution_summary(
            query.as_ref(),
            &execution_id,
        )
        .unwrap()
        .unwrap();
        assert_eq!(reloaded, record);
        assert!(!directory.path().join("workflow_executions").exists());
    }
}

#[tokio::test]
async fn test_workflow定義の起動_session木のidをworkflow専用境界で拒否する() {
    // Given
    let fixture = archive_fixture();
    let source = serde_saphyr::from_str("name: archive\ndescription: test\nnodes:\n  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}").unwrap();
    let workflow =
        crate::adaptor::gateway::workflow::mapper::schema_workflow_to_domain(source).unwrap();
    let id = "agent-session-00000000000040008000000000000098";
    // When
    let result = fixture
        .host
        .insert_workflow_execution(WorkflowExecutionInsert {
            execution_id: id.into(),
            workflow,
            worktree_path: "/missing/worktree".into(),
            request: None,
            created_from: ExecutionOrigin::Cli,
            workflow_defaults: WorkflowDefaults,
            now: 1.0,
        })
        .await;
    // Then
    assert!(matches!(
        result,
        Err(WorkflowRuntimeError::ValidationError(_))
    ));
    assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_実行木archive_gcはrepository_rootのない旧実行木も所属repo単位で判定する() {
    use crate::domain::workflow::{ExecutionTreeArchiveRepository, SessionExecutionTreeRootFacts};
    use crate::usecase::app_data_gc::{
        archive_removed_execution_trees, LiveWorktreeResolution, LiveWorktreeSet,
    };
    // Given
    let fixture = archive_fixture();
    let id = "agent-session-00000000000040008000000000000099";
    let facts = SessionExecutionTreeRootFacts::new(
        id,
        "/repos/a-worktrees/feature",
        "/repos/a-worktrees/feature",
        crate::domain::provider_lifecycle::ProviderKind::Codex,
        None,
    )
    .unwrap();
    crate::adaptor::gateway::workflow::fact_log::append_fact_batch_for_seed(
        &fixture.store,
        &facts.into_facts(),
        1,
        "legacy-root",
    )
    .unwrap();
    assert_eq!(
        fixture.repository.location(id).unwrap().repository_root,
        None
    );
    fixture
        .sessions
        .live_sessions
        .lock()
        .unwrap()
        .insert(id.into());
    let resolution = |unreadable: &str| {
        LiveWorktreeResolution::new(
            LiveWorktreeSet::default(),
            vec![unreadable.into()],
            Default::default(),
        )
        .with_repository_paths(vec!["/repos/a".into(), "/repos/b".into()])
    };
    // When / Then
    archive_removed_execution_trees(Some(&resolution("/repos/a")), &fixture.runtime)
        .await
        .unwrap();
    assert!(fixture
        .repository
        .archive_snapshot_for(&[id.into()])
        .unwrap()
        .records
        .is_empty());
    assert_eq!(
        fixture.repository.target(id).unwrap().status,
        ExecutionStatus::Running
    );
    archive_removed_execution_trees(Some(&resolution("/repos/b")), &fixture.runtime)
        .await
        .unwrap();
    assert_eq!(
        fixture.repository.target(id).unwrap().status,
        ExecutionStatus::Aborted
    );
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(&[id.into()])
            .unwrap()
            .records[0]
            .archive_reason,
        "worktree_removed"
    );
    assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_実行木archive_workflowをabortして停止完了後に隠し起動枠を解放する() {
    use crate::domain::workflow::ExecutionTreeArchiveRepository;
    use crate::usecase::workspace_tree::WorkspaceQueryService;
    // Given
    let fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    assert!(!fixture.sessions.live_sessions.lock().unwrap().is_empty());
    // When
    fixture
        .runtime
        .archive_execution_tree(&id, "manual")
        .await
        .unwrap();
    // Then
    assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
    assert_eq!(
        fixture.repository.target(&id).unwrap().status,
        ExecutionStatus::Aborted
    );
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(&[id.clone()])
            .unwrap()
            .records[0]
            .archive_reason,
        "manual"
    );
    assert!(fixture
        .query
        .workspace_tree(&crate::domain::workspace_tree::WorkspaceIdentity::new(
            "/missing/worktree"
        ))
        .unwrap()
        .nodes
        .is_empty());
    let facts = crate::adaptor::gateway::workflow::fact_log::read_tree_records(&fixture.store, &id)
        .unwrap();
    let abort = facts
        .iter()
        .position(|record| {
            matches!(
                record.fact,
                crate::domain::workflow::NodeFact::AbortRequested(_)
            )
        })
        .unwrap();
    let archive = facts
        .iter()
        .position(|record| {
            matches!(
                record.fact,
                crate::domain::workflow::NodeFact::ArchiveRequested(_)
            )
        })
        .unwrap();
    assert!(abort < archive);
    let next = archive_workflow(&fixture).await;
    assert_ne!(id, next);
}

#[tokio::test]
async fn test_実行木archive_provider_idのない単独sessionも同じ操作を使う() {
    use crate::domain::workflow::{ExecutionTreeArchiveRepository, SessionExecutionTreeRootFacts};
    use crate::usecase::workspace_tree::WorkspaceQueryService;
    // Given
    let fixture = archive_fixture();
    let id = "agent-session-00000000000040008000000000000012";
    let facts = SessionExecutionTreeRootFacts::new(
        id,
        "/missing/worktree",
        "/missing/worktree",
        crate::domain::provider_lifecycle::ProviderKind::Codex,
        None,
    )
    .unwrap();
    crate::adaptor::gateway::workflow::fact_log::append_fact_batch_for_seed(
        &fixture.store,
        &facts.into_facts(),
        1,
        "archive-standalone",
    )
    .unwrap();
    fixture
        .sessions
        .live_sessions
        .lock()
        .unwrap()
        .insert(id.into());
    let workspace = crate::domain::workspace_tree::WorkspaceIdentity::new("/missing/worktree");
    assert_eq!(
        fixture
            .query
            .workspace_tree(&workspace)
            .unwrap()
            .nodes
            .len(),
        1
    );
    // When
    fixture
        .runtime
        .archive_execution_tree(id, "manual")
        .await
        .unwrap();
    // Then
    assert!(fixture
        .query
        .workspace_tree(&workspace)
        .unwrap()
        .nodes
        .is_empty());
    assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
    assert_eq!(
        fixture.repository.target(id).unwrap().status,
        ExecutionStatus::Aborted
    );
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(&[id.into()])
            .unwrap()
            .records
            .len(),
        1
    );
}

#[tokio::test]
async fn test_実行木archive_abort書込失敗ではarchiveを記録しない() {
    use crate::domain::workflow::ExecutionTreeArchiveRepository;
    // Given
    let fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    fixture.store.close_write_queue_for_tests();
    // When
    assert!(fixture
        .runtime
        .archive_execution_tree(&id, "manual")
        .await
        .is_err());
    // Then
    assert_eq!(
        fixture.repository.target(&id).unwrap().status,
        ExecutionStatus::Running
    );
    assert!(fixture
        .repository
        .archive_snapshot_for(&[id])
        .unwrap()
        .records
        .is_empty());
}

#[tokio::test]
async fn test_実行木archive_旧記録移行はabort後に時刻と理由を保つ() {
    use crate::domain::workflow::ExecutionTreeArchiveRepository;
    // Given
    let fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    let path = fixture
        .directory
        .path()
        .join("workflow_execution_archives.json");
    std::fs::write(&path, serde_json::json!({"executions": {id.clone(): {"archivedAt": 42.123456, "archiveReason": "worktree_removed"}}}).to_string()).unwrap();
    // When
    fixture
        .runtime
        .migrate_execution_archives(fixture.repository.as_ref())
        .await
        .unwrap();
    // Then
    assert!(!path.exists());
    assert_eq!(
        fixture.repository.target(&id).unwrap().status,
        ExecutionStatus::Aborted
    );
    let records = fixture
        .repository
        .archive_snapshot_for(&[id])
        .unwrap()
        .records;
    assert_eq!(records[0].archived_at, 42.123456);
    assert_eq!(records[0].archive_reason, "worktree_removed");
    fixture
        .runtime
        .migrate_execution_archives(fixture.repository.as_ref())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_実行木archive_gcはgit登録の消失だけで判定する() {
    use crate::domain::workflow::ExecutionTreeArchiveRepository;
    use crate::usecase::app_data_gc::{
        archive_removed_execution_trees, LiveWorktree, LiveWorktreeResolution, LiveWorktreeSet,
    };
    // Given
    let fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    assert_eq!(
        fixture
            .repository
            .target(&id)
            .unwrap()
            .repository_root
            .as_deref(),
        Some("/repo")
    );
    let live = LiveWorktreeResolution::new(
        LiveWorktreeSet::from_worktrees([LiveWorktree {
            path: "/missing/worktree".into(),
            workspace_state_keys: Vec::new(),
            review_comment_keys: Vec::new(),
        }]),
        Vec::new(),
        Default::default(),
    );
    // When / Then: directory is absent but Git registration is authoritative.
    archive_removed_execution_trees(Some(&live), &fixture.runtime)
        .await
        .unwrap();
    assert_eq!(
        fixture.repository.target(&id).unwrap().status,
        ExecutionStatus::Running
    );
    let unknown = LiveWorktreeResolution::new(
        LiveWorktreeSet::default(),
        vec!["/repo".into()],
        Default::default(),
    );
    archive_removed_execution_trees(Some(&unknown), &fixture.runtime)
        .await
        .unwrap();
    assert_eq!(
        fixture.repository.target(&id).unwrap().status,
        ExecutionStatus::Running
    );
    let removed = LiveWorktreeResolution::new(
        LiveWorktreeSet::default(),
        vec!["/unrelated-unreadable-repo".into()],
        Default::default(),
    );
    archive_removed_execution_trees(Some(&removed), &fixture.runtime)
        .await
        .unwrap();
    assert_eq!(
        fixture.repository.target(&id).unwrap().status,
        ExecutionStatus::Aborted
    );
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(&[id])
            .unwrap()
            .records[0]
            .archive_reason,
        "worktree_removed"
    );
    assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_実行木archive_停止失敗では隠さず再実行で完了する() {
    use crate::domain::workflow::ExecutionTreeArchiveRepository;
    let fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    fixture
        .sessions
        .stop_fails
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(fixture
        .runtime
        .archive_execution_tree(&id, "manual")
        .await
        .is_err());
    assert!(fixture
        .repository
        .archive_snapshot_for(&[id.clone()])
        .unwrap()
        .records
        .is_empty());
    assert_eq!(
        fixture.repository.target(&id).unwrap().status,
        ExecutionStatus::Aborted
    );
    fixture
        .sessions
        .stop_fails
        .store(false, std::sync::atomic::Ordering::SeqCst);
    fixture
        .runtime
        .archive_execution_tree(&id, "manual")
        .await
        .unwrap();
    assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(&[id])
            .unwrap()
            .records
            .len(),
        1
    );
}

#[tokio::test]
async fn test_実行木archive_移行失敗は旧記録を保持する() {
    let fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    let path = fixture
        .directory
        .path()
        .join("workflow_execution_archives.json");
    let legacy =
        serde_json::json!({"executions": {id: {"archivedAt": 42.0, "archiveReason": "manual"}}})
            .to_string();
    std::fs::write(&path, &legacy).unwrap();
    fixture.store.close_write_queue_for_tests();
    assert!(fixture
        .runtime
        .migrate_execution_archives(fixture.repository.as_ref())
        .await
        .is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), legacy);
}

#[tokio::test]
async fn test_実行木archive_終了済みは状態を保持しrestoreでも再開しない() {
    use crate::domain::workflow::{ExecutionTreeArchiveRepository, NodeFact};
    for status in [ExecutionStatus::Aborted, ExecutionStatus::Completed] {
        let fixture = archive_fixture();
        let id = archive_workflow(&fixture).await;
        if status == ExecutionStatus::Aborted {
            fixture
                .host
                .abort_workflow_execution(&fixture.app, &id, None)
                .await
                .unwrap();
        } else {
            let state = fixture
                .host
                .get_state_by_execution_id(&fixture.app, &id)
                .await
                .unwrap();
            let node = &state.node_executions[0];
            fixture
                .runtime
                .submit_output(SubmitOutputCommand {
                    node_execution_id: node.id.clone(),
                    artifact: None,
                })
                .await
                .unwrap();
            fixture
                .runtime
                .record_provider_stop(
                    ProviderExecutionTreeStopCommand {
                        agent_session_id: node.session_id.clone().unwrap(),
                        tree_id: id.clone(),
                        node_execution_id: node.id.clone(),
                        binding_id: "archive-complete".into(),
                    },
                    Vec::new(),
                )
                .await
                .unwrap();
        }
        fixture
            .runtime
            .archive_execution_tree(&id, "manual")
            .await
            .unwrap();
        fixture.runtime.restore_execution_tree(&id).await.unwrap();
        assert_eq!(fixture.repository.target(&id).unwrap().status, status);
        assert!(fixture
            .repository
            .archive_snapshot_for(&[id.clone()])
            .unwrap()
            .records
            .is_empty());
        assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
        let facts =
            crate::adaptor::gateway::workflow::fact_log::read_tree_records(&fixture.store, &id)
                .unwrap();
        assert_eq!(
            facts
                .iter()
                .filter(|record| matches!(record.fact, NodeFact::AbortRequested(_)))
                .count(),
            usize::from(status == ExecutionStatus::Aborted)
        );
    }
}

#[tokio::test]
async fn test_実行木archive_command停止完了まではarchiveを記録しない() {
    use crate::domain::workflow::{
        CommandSpec, ExecutionTreeArchiveRepository, NodeFact, NodeKind, NodeKindName,
        SessionExecutionTreeRootFacts,
    };
    let fixture = archive_fixture();
    let id = "00000000-0000-4000-8000-000000000013";
    let mut facts = SessionExecutionTreeRootFacts::new(
        id,
        "/missing/worktree",
        "/missing/worktree",
        crate::domain::provider_lifecycle::ProviderKind::Codex,
        None,
    )
    .unwrap();
    facts.meta.kind = NodeKindName::Command;
    let NodeFact::Started(started) = &mut facts.started else {
        unreachable!()
    };
    let root = started.root.as_mut().unwrap();
    root.launched_as = crate::domain::workflow::ExecutionTreeLaunch::Workflow;
    root.definition.as_mut().unwrap().nodes[0].kind = NodeKind::Command(CommandSpec {
        command: "unused".into(),
        env: Default::default(),
    });
    crate::adaptor::gateway::workflow::fact_log::append_fact_batch_for_seed(
        &fixture.store,
        &[(facts.meta, facts.started)],
        1,
        "seed-command",
    )
    .unwrap();
    fixture
        .host
        .active_command_executions
        .lock()
        .await
        .insert(id.into(), id.into());
    fixture
        .host
        .node_processes
        .active_commands
        .lock()
        .unwrap()
        .insert(
            id.into(),
            crate::infrastructure::process::command_runner::ActiveCommandHandle::for_test(),
        );
    let (stopped, completion) = tokio::sync::oneshot::channel::<()>();
    fixture
        .host
        .command_completion_observers
        .lock()
        .await
        .insert(
            id.into(),
            tokio::spawn(async move {
                completion.await.unwrap();
            }),
        );
    let archive = fixture.runtime.archive_worktree("/missing/worktree/");
    tokio::pin!(archive);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut archive)
            .await
            .is_err()
    );
    assert!(fixture
        .host
        .node_processes
        .active_commands
        .lock()
        .unwrap()
        .is_empty());
    assert!(fixture
        .repository
        .archive_snapshot_for(&[id.into()])
        .unwrap()
        .records
        .is_empty());
    stopped.send(()).unwrap();
    archive.await.unwrap();
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(&[id.into()])
            .unwrap()
            .records[0]
            .archive_reason,
        "worktree_removed"
    );
}

#[tokio::test]
async fn test_実行木archive_旧sessionのarchive事実も終了状態へ移行する() {
    use crate::domain::workflow::{
        ExecutionTreeArchiveRepository, NodeFact, SessionExecutionTreeRootFacts,
    };
    let fixture = archive_fixture();
    let id = "00000000-0000-4000-8000-000000000014";
    let facts = SessionExecutionTreeRootFacts::new(
        id,
        "/workspace",
        "/missing/worktree",
        crate::domain::provider_lifecycle::ProviderKind::Codex,
        None,
    )
    .unwrap();
    let meta = facts.meta.clone();
    crate::adaptor::gateway::workflow::fact_log::append_fact_batch_for_seed(
        &fixture.store,
        &facts.into_facts(),
        1,
        "seed-old-session",
    )
    .unwrap();
    let mut pending = crate::adaptor::gateway::workflow::fact_log::pending_single_fact(
        &meta,
        &NodeFact::AbortRequested(Default::default()),
        42000,
    )
    .unwrap();
    pending.row.event_type = "archive_requested".into();
    pending.row.detail = "{}".into();
    crate::adaptor::gateway::workflow::fact_log::append_pending_rows_blocking(
        &fixture.store,
        vec![pending],
    )
    .unwrap();
    fixture
        .runtime
        .migrate_execution_archives(fixture.repository.as_ref())
        .await
        .unwrap();
    assert_eq!(
        fixture.repository.target(id).unwrap().status,
        ExecutionStatus::Aborted
    );
    assert_eq!(
        fixture.repository.target(id).unwrap().workspace_identity,
        "/workspace"
    );
    assert_eq!(
        fixture
            .repository
            .worktree_target_page("/workspace", None)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(&[id.into()])
            .unwrap()
            .records[0]
            .archived_at,
        42.0
    );
    assert!(fixture
        .repository
        .legacy_session_archive_page(None)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn test_実行木archive_gcは単独sessionの所属repoだけの読取結果で判定する() {
    use crate::adaptor::gateway::agent_session::LocalAgentSessionRepository;
    use crate::adaptor::gateway::app_data_gc::{build_startup_gc_request, StdGcFileSystem};
    use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionTreeLocation};
    use crate::domain::agent_session::repository::AgentSessionRepository;
    use crate::domain::provider_lifecycle::ProviderKind;
    use crate::domain::workflow::ExecutionTreeArchiveRepository;
    use crate::domain::workspace_tree::WorkspaceIdentity;
    use crate::usecase::app_data_gc::archive_removed_execution_trees;

    for folder_remains in [true, false] {
        // Given
        let fixture = archive_fixture();
        let (repo_dir, repo) = crate::test_support::git::create_test_repo();
        crate::test_support::git::create_initial_commit(&repo);
        let worktree_path = fixture
            .directory
            .path()
            .canonicalize()
            .unwrap()
            .join("worktree");
        let worktree = repo.worktree("feature", &worktree_path, None).unwrap();
        let path = worktree_path.to_str().unwrap();
        let id = "agent-session-00000000000040008000000000000034";
        LocalAgentSessionRepository::new(fixture.store.clone())
            .create(
                AgentSession::create(
                    id,
                    WorkspaceIdentity::new(path),
                    path,
                    ProviderKind::Codex,
                    AgentSessionTreeLocation::session_tree_root(id).unwrap(),
                )
                .unwrap(),
                "create-session-gc",
            )
            .await
            .unwrap();
        fixture
            .sessions
            .live_sessions
            .lock()
            .unwrap()
            .insert(id.into());
        let root = repo_dir.path().canonicalize().unwrap();
        assert_eq!(
            fixture
                .repository
                .target(id)
                .unwrap()
                .repository_root
                .as_deref(),
            root.to_str()
        );
        let repos = Arc::new(parking_lot::RwLock::new(vec![
            root.to_string_lossy().into_owned(),
            fixture
                .directory
                .path()
                .join("unreadable-repo")
                .to_string_lossy()
                .into_owned(),
        ]));
        if !folder_remains {
            std::fs::remove_dir_all(&worktree_path).unwrap();
        }
        let resolution = || {
            build_startup_gc_request(
                fixture.directory.path().into(),
                repos.clone(),
                &StdGcFileSystem::default(),
            )
            .live_worktrees
        };

        // When / Then: Git registration protects the tree even without its folder.
        archive_removed_execution_trees(resolution().as_ref(), &fixture.runtime)
            .await
            .unwrap();
        assert_eq!(
            fixture.repository.target(id).unwrap().status,
            ExecutionStatus::Running
        );
        let mut options = git2::WorktreePruneOptions::new();
        options.valid(true).working_tree(false);
        worktree.prune(Some(&mut options)).unwrap();
        let git_dir = repo.path().to_path_buf();
        let hidden_git = root.join("hidden-git");
        std::fs::rename(&git_dir, &hidden_git).unwrap();
        archive_removed_execution_trees(resolution().as_ref(), &fixture.runtime)
            .await
            .unwrap();
        assert_eq!(
            fixture.repository.target(id).unwrap().status,
            ExecutionStatus::Running
        );
        assert!(fixture
            .repository
            .archive_snapshot_for(&[id.into()])
            .unwrap()
            .records
            .is_empty());
        std::fs::rename(&hidden_git, &git_dir).unwrap();

        // When
        archive_removed_execution_trees(resolution().as_ref(), &fixture.runtime)
            .await
            .unwrap();

        // Then
        assert_eq!(
            fixture.repository.target(id).unwrap().status,
            ExecutionStatus::Aborted
        );
        assert_eq!(
            fixture
                .repository
                .archive_snapshot_for(&[id.into()])
                .unwrap()
                .records[0]
                .archive_reason,
            "worktree_removed"
        );
        assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
        assert_eq!(worktree_path.exists(), folder_remains);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_実行木restore_同じarchive期間への並行要求は一度だけ記録する() {
    // Given
    let fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    for period in 1..=2 {
        fixture
            .runtime
            .archive_execution_tree(&id, "manual")
            .await
            .unwrap();
        let barrier = Arc::new(tokio::sync::Barrier::new(8));
        let mut tasks = Vec::new();
        // When
        for _ in 0..8 {
            let runtime = fixture.runtime.clone();
            let id = id.clone();
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                runtime.restore_execution_tree(&id).await
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        // Then
        let records =
            crate::adaptor::gateway::workflow::fact_log::read_tree_records(&fixture.store, &id)
                .unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(
                    record.fact,
                    crate::domain::workflow::NodeFact::RestoreRequested
                ))
                .count(),
            period
        );
        assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn test_archive移行_旧ファイルも対象も無い起動では無関係な破損履歴をfoldしない() {
    use crate::domain::workflow::{NodeFact, SessionExecutionTreeRootFacts};
    // Given
    let fixture = archive_fixture();
    let facts = SessionExecutionTreeRootFacts::new(
        "unrelated",
        "/repo",
        "/repo",
        crate::domain::provider_lifecycle::ProviderKind::Codex,
        None,
    )
    .unwrap();
    let mut rows = facts
        .into_facts()
        .iter()
        .map(|(meta, fact)| {
            crate::adaptor::gateway::workflow::fact_log::pending_single_fact(meta, fact, 1).unwrap()
        })
        .collect::<Vec<_>>();
    let mut corrupt = rows[1].clone();
    corrupt.row.event_type =
        super::fact_codec::event_type(&NodeFact::AbortRequested(Default::default())).into();
    corrupt.row.detail = "broken history".into();
    rows.push(corrupt);
    crate::adaptor::gateway::workflow::fact_log::append_pending_rows_blocking(&fixture.store, rows)
        .unwrap();
    // When / Then
    fixture
        .runtime
        .migrate_execution_archives(fixture.repository.as_ref())
        .await
        .unwrap();
    assert!(crate::adaptor::gateway::workflow::fact_log::fold_tree_from(
        &crate::adaptor::gateway::workflow::fact_log::FactLogReadBackend::Live(fixture.store),
        "unrelated"
    )
    .is_err());
}

#[tokio::test]
async fn test_worktree削除中_外部変更を拒否して読み取りと内部archiveを許可する() {
    use crate::domain::workflow::{ExecutionTreeArchiveRepository, NodeFact};
    use crate::usecase::repository_usecase::WorktreeExecutionArchiver;
    use crate::usecase::workflow::command::{
        AbortExecutionCommand, ApprovalCommand, ResumeSessionNodeCommand, RetryNodeCommand,
        StartExecutionCommand,
    };
    use crate::usecase::workspace_tree::WorkspaceQueryService;
    // Given
    let fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    let node = fixture
        .host
        .get_state_by_execution_id(&fixture.app, &id)
        .await
        .unwrap()
        .node_executions[0]
        .id
        .clone();
    let _deletion = fixture
        .runtime
        .begin_worktree_deletion("/missing/worktree/")
        .await
        .unwrap();
    let before =
        crate::adaptor::gateway::workflow::fact_log::read_tree_records(&fixture.store, &id)
            .unwrap();
    // When
    let errors = [
        fixture
            .runtime
            .archive_execution_tree(&id, "manual")
            .await
            .unwrap_err(),
        fixture
            .runtime
            .restore_execution_tree(&id)
            .await
            .unwrap_err(),
        fixture
            .runtime
            .abort_execution(AbortExecutionCommand {
                execution_id: id.clone(),
                expected_node_name: None,
            })
            .await
            .unwrap_err(),
        fixture
            .runtime
            .retry_node(RetryNodeCommand {
                execution_id: id.clone(),
                node_execution_id: node.clone(),
            })
            .await
            .unwrap_err(),
        fixture
            .runtime
            .resume_session_node(ResumeSessionNodeCommand {
                execution_id: id.clone(),
                node_execution_id: node.clone(),
            })
            .await
            .unwrap_err(),
        fixture
            .runtime
            .resolve_approval(ApprovalCommand {
                execution_id: id.clone(),
                node_name: "main".into(),
                node_execution_id: Some(node.clone()),
                comment: None,
            })
            .await
            .unwrap_err(),
        fixture
            .runtime
            .submit_output(SubmitOutputCommand {
                node_execution_id: node,
                artifact: None,
            })
            .await
            .unwrap_err(),
        fixture
            .runtime
            .start_execution(StartExecutionCommand {
                workflow_name: "archive".into(),
                worktree_path: "/missing/worktree".into(),
                request: None,
                created_from: ExecutionOrigin::Cli,
            })
            .await
            .unwrap_err(),
    ];
    // Then
    assert!(
        errors
            .iter()
            .all(|error| error.to_string().contains("deletion is in progress")),
        "{errors:?}"
    );
    assert_eq!(
        crate::adaptor::gateway::workflow::fact_log::read_tree_records(&fixture.store, &id)
            .unwrap(),
        before
    );
    assert!(!fixture
        .query
        .workspace_tree(&crate::domain::workspace_tree::WorkspaceIdentity::new(
            "/missing/worktree"
        ))
        .unwrap()
        .nodes
        .is_empty());
    fixture
        .runtime
        .archive_worktree("/missing/worktree")
        .await
        .unwrap();
    assert_eq!(
        fixture.repository.target(&id).unwrap().status,
        ExecutionStatus::Aborted
    );
    assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
    assert!(
        crate::adaptor::gateway::workflow::fact_log::read_tree_records(&fixture.store, &id)
            .unwrap()
            .iter()
            .any(|fact| matches!(fact.fact, NodeFact::ArchiveRequested(_)))
    );
}

#[tokio::test]
async fn test_旧実行木gc_任意位置のlinked_worktreeを対象repoの読取結果だけで判定する() {
    assert_legacy_linked_worktree_gc(false, false).await;
}

#[tokio::test]
async fn test_旧実行木gc_任意位置のlinked_worktreeの所属をフォルダ消失後も保持する() {
    assert_legacy_linked_worktree_gc(true, false).await;
}

#[tokio::test]
async fn test_旧実行木gc_初回gc前にフォルダが消えていてもgit登録から所属を保持する() {
    assert_legacy_linked_worktree_gc(true, true).await;
}

async fn assert_legacy_linked_worktree_gc(remove_directory: bool, remove_before_gc: bool) {
    use crate::adaptor::gateway::app_data_gc::{build_startup_gc_request, StdGcFileSystem};
    use crate::adaptor::gateway::workflow::fact_log;
    use crate::domain::workflow::{ExecutionTreeArchiveRepository, SessionExecutionTreeRootFacts};
    use crate::usecase::app_data_gc::archive_removed_execution_trees;
    // Given
    let fixture = archive_fixture();
    let (repo_dir, repo) = crate::test_support::git::create_test_repo();
    crate::test_support::git::create_initial_commit(&repo);
    let path = fixture
        .directory
        .path()
        .canonicalize()
        .unwrap()
        .join("arbitrary-linked-worktree");
    let worktree = repo.worktree("feature", &path, None).unwrap();
    let id = "00000000-0000-4000-8000-000000000996";
    let facts = SessionExecutionTreeRootFacts::new(
        id,
        path.to_str().unwrap(),
        path.to_str().unwrap(),
        crate::domain::provider_lifecycle::ProviderKind::Codex,
        None,
    )
    .unwrap();
    fact_log::append_fact_batch_for_seed(&fixture.store, &facts.into_facts(), 1, id).unwrap();
    assert!(fixture
        .repository
        .target(id)
        .unwrap()
        .repository_root
        .is_none());
    let root = repo_dir.path().canonicalize().unwrap();
    let repos = Arc::new(parking_lot::RwLock::new(vec![
        root.to_string_lossy().into_owned(),
        "/unreadable-other-repository".into(),
    ]));
    let resolution = || {
        build_startup_gc_request(
            fixture.directory.path().into(),
            repos.clone(),
            &StdGcFileSystem::default(),
        )
        .live_worktrees
    };
    // When / Then
    if remove_before_gc {
        std::fs::remove_dir_all(&path).unwrap();
    }
    archive_removed_execution_trees(resolution().as_ref(), &fixture.runtime)
        .await
        .unwrap();
    assert!(fixture
        .repository
        .archive_snapshot_for(&[id.into()])
        .unwrap()
        .records
        .is_empty());
    let mut options = git2::WorktreePruneOptions::new();
    options.valid(true).working_tree(false);
    worktree.prune(Some(&mut options)).unwrap();
    if remove_directory && !remove_before_gc {
        std::fs::remove_dir_all(&path).unwrap();
    }
    let reopened =
        crate::adaptor::gateway::workflow::ExecutionTreeArchiveFactRepository::from_backend(
            fact_log::FactLogReadBackend::ReadOnly(
                crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore::open(
                    fixture.directory.path(),
                )
                .unwrap(),
            ),
        );
    assert_eq!(
        reopened.candidate_page(None).unwrap()[0]
            .repository_root
            .as_deref(),
        root.to_str(),
    );
    let git_dir = root.join(".git");
    let hidden = root.join("hidden-git");
    std::fs::rename(&git_dir, &hidden).unwrap();
    archive_removed_execution_trees(resolution().as_ref(), &fixture.runtime)
        .await
        .unwrap();
    assert_eq!(
        fixture.repository.target(id).unwrap().status,
        ExecutionStatus::Running
    );
    std::fs::rename(&hidden, &git_dir).unwrap();
    assert_eq!(
        archive_removed_execution_trees(resolution().as_ref(), &fixture.runtime)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        fixture.repository.target(id).unwrap().status,
        ExecutionStatus::Aborted
    );
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(&[id.into()])
            .unwrap()
            .records[0]
            .archive_reason,
        "worktree_removed"
    );
    assert_eq!(path.exists(), !remove_directory);
}

#[tokio::test]
async fn test_記録からの操作_agent_sessionが再開を保存した後にsubmitとstopを記録する() {
    use crate::adaptor::gateway::agent_session::LocalAgentSessionRepository;
    use crate::domain::agent_session::aggregates::{
        AgentSession, AgentSessionRecoveryResult, AgentSessionTreeLocation,
    };
    use crate::domain::agent_session::repository::AgentSessionRepository;
    use crate::domain::provider_lifecycle::ProviderKind;
    use crate::domain::workspace_tree::WorkspaceIdentity;
    // Given
    let fixture = test_helpers::Fixture::new(0);
    let id = "resumed-session-without-runtime-registration";
    let repository = LocalAgentSessionRepository::new(fixture.store.clone());
    let mut saved = repository
        .create(
            AgentSession::create(
                id,
                WorkspaceIdentity::new("/repo"),
                "/repo",
                ProviderKind::Codex,
                AgentSessionTreeLocation::session_tree_root(id).unwrap(),
            )
            .unwrap(),
            "create-record-only-session",
        )
        .await
        .unwrap();
    fixture
        .host
        .load_control_plane_execution(&fixture.app, id)
        .await
        .unwrap()
        .unwrap();
    saved
        .session_mut()
        .associate_provider_session("provider-session", None)
        .unwrap();
    saved.session_mut().observe_provider_process_exit(Some(1));
    let mut saved = repository.save(saved, "record-only-exit").await.unwrap();
    saved
        .session_mut()
        .complete_resume(AgentSessionRecoveryResult::Succeeded)
        .unwrap();
    repository.save(saved, "record-only-resume").await.unwrap();
    let control =
        WorkflowControlPlaneUsecase::new(Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
            fixture.app.clone(),
            Arc::new(fixture.restarted_host()),
        )));
    // When
    control
        .submit_output(SubmitOutputCommand {
            node_execution_id: id.into(),
            artifact: None,
        })
        .await
        .unwrap();
    control
        .record_provider_stop(
            ProviderExecutionTreeStopCommand {
                agent_session_id: id.into(),
                tree_id: id.into(),
                node_execution_id: id.into(),
                binding_id: "binding-resumed".into(),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    // Then
    let records = workflow_fact_log::read_tree_records(&fixture.store, id).unwrap();
    assert!(records.iter().any(|record| matches!(
        record.fact,
        crate::domain::workflow::NodeFact::ResumeRequested
    )));
    assert!(records.iter().any(|record| matches!(
        record.fact,
        crate::domain::workflow::NodeFact::SubmitReceived(_)
    )));
    assert!(records.iter().any(|record| matches!(
        record.fact,
        crate::domain::workflow::NodeFact::StopReceived(_)
    )));
}

#[tokio::test]
async fn test_起動時前進_失敗を理由付きabortにし他の木を進め再試行しない() {
    // Given
    let fixture = test_helpers::Fixture::new(0);
    let failed = fixture.persist_started("  main: {session: {provider: codex, facets: {instruction: missing-startup-facet-1840}}}\n", "/failed").await;
    let healthy = fixture
        .persist_started(
            "  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n",
            "/healthy",
        )
        .await;
    let host = Arc::new(fixture.restarted_host());
    let startup = crate::adaptor::controller::wiring::wire_workflow_startup(
        fixture.app.clone(),
        host.clone(),
    )
    .unwrap();
    // When
    assert!(startup.execute().await.is_err());
    let failed_records =
        workflow_fact_log::read_tree_records(&fixture.store, &failed.execution_id).unwrap();
    let healthy_records =
        workflow_fact_log::read_tree_records(&fixture.store, &healthy.execution_id).unwrap();
    startup.execute().await.unwrap();
    // Then
    assert!(failed_records.iter().any(|record| matches!(&record.fact,
        crate::domain::workflow::NodeFact::AbortRequested(fact) if fact.reason.as_ref().is_some_and(|reason| reason.contains("missing-startup-facet-1840")))));
    assert_eq!(
        host.load_execution(&fixture.app, &failed.execution_id)
            .await
            .unwrap()
            .state(),
        &RuntimeExecutionState::Aborted
    );
    assert!(healthy_records.iter().any(|record| matches!(
        record.fact,
        crate::domain::workflow::NodeFact::SessionAttached(_)
    )));
    assert_eq!(fixture.sessions.activated.lock().unwrap().len(), 1);
    assert_eq!(
        workflow_fact_log::read_tree_records(&fixture.store, &failed.execution_id).unwrap(),
        failed_records
    );
    assert_eq!(
        workflow_fact_log::read_tree_records(&fixture.store, &healthy.execution_id).unwrap(),
        healthy_records
    );
    assert!(host.startup_retries.lock().await.is_empty());
}

#[tokio::test]
async fn test_起動時前進_reply喪失後は保存済みなら続行し未保存なら理由付きabortする() {
    use crate::adaptor::gateway::local_event_store::layout::StoreLayout;
    use crate::domain::workflow::NodeFact;

    for persisted in [true, false] {
        // Given
        let fixture = test_helpers::Fixture::new(0);
        let interrupted = fixture
            .persist_started(
                "  main: {sequence: {children: [first, nested]}}\n  nested: {sequence: {children: [next]}}\n  first: {session: {provider: codex}}\n  next: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n",
                "/interrupted",
            )
            .await;
        let records =
            workflow_fact_log::read_tree_records(&fixture.store, &interrupted.execution_id)
                .unwrap();
        let first = &records
            .iter()
            .find(|record| record.meta.node_name == "first")
            .unwrap()
            .meta;
        for fact in [
            NodeFact::SubmitReceived(crate::domain::workflow::SubmitReceivedFact {
                request_id: None,
            }),
            NodeFact::StopReceived(crate::domain::workflow::StopReceivedFact {
                result_summary: None,
                token_usage: None,
            }),
        ] {
            workflow_fact_log::append_single_fact(&fixture.store, first, &fact, 2000).unwrap();
        }
        let healthy = fixture
            .persist_started(
                "  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n",
                "/healthy",
            )
            .await;
        if !persisted {
            let connection = rusqlite::Connection::open(
                StoreLayout::new(fixture._directory.path()).database_path(),
            )
            .unwrap();
            connection.execute_batch("CREATE TRIGGER fail_startup_advance BEFORE INSERT ON node_events WHEN NEW.event_type = 'started' AND NEW.node_name = 'next' BEGIN SELECT RAISE(ABORT, 'injected advancement failure'); END;").unwrap();
        }
        let host = Arc::new(fixture.restarted_host());
        let startup = crate::adaptor::controller::wiring::wire_workflow_startup(
            fixture.app.clone(),
            host.clone(),
        )
        .unwrap();
        fixture.store.fault_injector().arm_drop_reply();

        // When
        let result = startup.execute().await;
        let records =
            workflow_fact_log::read_tree_records(&fixture.store, &interrupted.execution_id)
                .unwrap();
        let healthy_records =
            workflow_fact_log::read_tree_records(&fixture.store, &healthy.execution_id).unwrap();
        startup.execute().await.unwrap();

        // Then
        assert_eq!(result.is_ok(), persisted, "{result:?}");
        for name in ["nested", "next"] {
            assert_eq!(
                records
                    .iter()
                    .filter(|record| record.meta.node_name == name
                        && matches!(record.fact, NodeFact::Started(_)))
                    .count(),
                usize::from(persisted)
            );
        }
        assert_eq!(
            records
                .iter()
                .filter(|record| record.meta.node_name == "next"
                    && matches!(record.fact, NodeFact::SessionAttached(_)))
                .count(),
            usize::from(persisted)
        );
        assert_eq!(
            records.iter().filter(|record| matches!(&record.fact, NodeFact::AbortRequested(fact) if fact.reason.as_ref().is_some_and(|reason| reason.contains("startup advancement commit failed")))).count(),
            usize::from(!persisted)
        );
        assert_eq!(
            host.load_execution(&fixture.app, &interrupted.execution_id)
                .await
                .unwrap()
                .state(),
            if persisted {
                &RuntimeExecutionState::Running
            } else {
                &RuntimeExecutionState::Aborted
            }
        );
        assert!(healthy_records
            .iter()
            .any(|record| matches!(record.fact, NodeFact::SessionAttached(_))));
        assert_eq!(
            fixture.sessions.activated.lock().unwrap().len(),
            1 + usize::from(persisted)
        );
        assert_eq!(
            workflow_fact_log::read_tree_records(&fixture.store, &interrupted.execution_id)
                .unwrap(),
            records
        );
        assert_eq!(
            workflow_fact_log::read_tree_records(&fixture.store, &healthy.execution_id).unwrap(),
            healthy_records
        );
        assert!(host.startup_retries.lock().await.is_empty());
    }
}

#[tokio::test]
async fn test_起動時session紐付け_reply喪失後は保存済みなら起動し未保存ならabortする() {
    use crate::adaptor::gateway::local_event_store::layout::StoreLayout;
    use crate::domain::workflow::NodeFact;

    for persisted in [true, false] {
        // Given
        let fixture = test_helpers::Fixture::new(0);
        let interrupted = fixture
            .persist_started(
                "  main: {fanout: {children: [one, two]}}\n  one: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n  two: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n",
                "/interrupted",
            )
            .await;
        let healthy = fixture
            .persist_started(
                "  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n",
                "/healthy",
            )
            .await;
        if !persisted {
            let connection = rusqlite::Connection::open(
                StoreLayout::new(fixture._directory.path()).database_path(),
            )
            .unwrap();
            connection.execute_batch("CREATE TRIGGER fail_session_attachment BEFORE INSERT ON node_events WHEN NEW.event_type = 'session_attached' AND NEW.node_name = 'two' BEGIN SELECT RAISE(ABORT, 'injected attachment failure'); END;").unwrap();
        }
        let host = Arc::new(fixture.restarted_host());
        let startup = crate::adaptor::controller::wiring::wire_workflow_startup(
            fixture.app.clone(),
            host.clone(),
        )
        .unwrap();
        fixture.store.fault_injector().arm_drop_reply();

        // When
        let result = startup.execute().await;
        let records =
            workflow_fact_log::read_tree_records(&fixture.store, &interrupted.execution_id)
                .unwrap();
        let healthy_records =
            workflow_fact_log::read_tree_records(&fixture.store, &healthy.execution_id).unwrap();
        startup.execute().await.unwrap();

        // Then
        assert_eq!(result.is_ok(), persisted, "{result:?}");
        for name in ["one", "two"] {
            assert_eq!(
                records
                    .iter()
                    .filter(|record| record.meta.node_name == name
                        && matches!(record.fact, NodeFact::SessionAttached(_)))
                    .count(),
                usize::from(persisted)
            );
        }
        let aborts = records
            .iter()
            .filter_map(|record| match &record.fact {
                NodeFact::AbortRequested(fact) => Some(fact),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(aborts.len(), usize::from(!persisted));
        if let Some(abort) = aborts.first() {
            assert!(abort
                .reason
                .as_ref()
                .unwrap()
                .contains("session attachment event append failed"));
        }
        assert_eq!(
            host.load_execution(&fixture.app, &interrupted.execution_id)
                .await
                .unwrap()
                .is_active(),
            persisted
        );
        assert!(healthy_records
            .iter()
            .any(|record| matches!(record.fact, NodeFact::SessionAttached(_))));
        assert_eq!(fixture.sessions.prepared.lock().unwrap().len(), 3);
        assert_eq!(
            fixture.sessions.activated.lock().unwrap().len(),
            1 + 2 * usize::from(persisted)
        );
        assert_eq!(
            fixture.sessions.live_sessions.lock().unwrap().len(),
            1 + 2 * usize::from(persisted)
        );
        assert_eq!(
            workflow_fact_log::read_tree_records(&fixture.store, &interrupted.execution_id)
                .unwrap(),
            records
        );
        assert_eq!(
            workflow_fact_log::read_tree_records(&fixture.store, &healthy.execution_id).unwrap(),
            healthy_records
        );
        assert!(host.startup_retries.lock().await.is_empty());
    }
}

#[tokio::test]
async fn test_worktree排他_再起動直後の記録を使い占有実行をエラーに含む() {
    // Given
    let fixture = test_helpers::Fixture::new(0);
    let active = fixture
        .persist_started("  main: {session: {provider: codex}}\n", "/repo")
        .await;
    let host = fixture.restarted_host();
    let attempted: WorkflowDefinition = serde_saphyr::from_str("name: attempted\ndescription: test\nnodes:\n  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n").unwrap();
    // When
    let error = host
        .start_resolved_workflow(
            &fixture.app,
            attempted,
            "/repo".into(),
            None,
            ExecutionOrigin::Cli,
        )
        .await
        .unwrap_err();
    // Then
    let message = error.to_string();
    assert!(message.contains("Worktree '/repo'"));
    assert!(message.contains(&active.execution_id));
    assert!(message.contains("isolated"));
    assert!(!message.contains("attempted"));
    assert!(fixture.sessions.prepared.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_worktree排他_同時起動は一件だけ成功し外部abort後は起動できる() {
    // Given
    let fixture = test_helpers::Fixture::new(0);
    let workflow: WorkflowDefinition = serde_saphyr::from_str("name: concurrent\ndescription: test\nnodes:\n  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n").unwrap();
    let facet_guard = fixture.host.execution_facet_contents.lock().await;
    // When
    let mut first = Box::pin(fixture.host.start_resolved_workflow(
        &fixture.app,
        workflow.clone(),
        "/repo".into(),
        None,
        ExecutionOrigin::Cli,
    ));
    assert!(futures_util::poll!(first.as_mut()).is_pending());
    let mut second = Box::pin(fixture.host.start_resolved_workflow(
        &fixture.app,
        workflow.clone(),
        "/repo".into(),
        None,
        ExecutionOrigin::Cli,
    ));
    assert!(futures_util::poll!(second.as_mut()).is_pending());
    drop(facet_guard);
    let (first, second) = tokio::join!(first, second);
    // Then
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let (id, error) = match (first, second) {
        (Ok(id), Err(error)) | (Err(error), Ok(id)) => (id, error),
        _ => unreachable!(),
    };
    assert!(error.to_string().contains(&id));
    workflow_fact_log::append_facts_for_events(
        &fixture.store,
        &[WorkflowEvent::ExecutionAborted {
            execution_id: id,
            aborted_node: None,
            timestamp: current_timestamp(),
        }],
    )
    .unwrap();
    assert!(fixture
        .host
        .start_resolved_workflow(
            &fixture.app,
            workflow,
            "/repo".into(),
            None,
            ExecutionOrigin::Cli
        )
        .await
        .is_ok());
}

#[tokio::test]
async fn test_worktree排他_abort待機中も別worktreeは起動し同一worktreeだけ待つ() {
    for (abort_succeeds, before_commit) in [(true, true), (false, true), (true, false)] {
        // Given
        let fixture = test_helpers::Fixture::new(0);
        let active = fixture
            .persist_started("  main: {session: {provider: codex}}\n", "/repo")
            .await;
        let workflow: WorkflowDefinition = serde_saphyr::from_str("name: concurrent\ndescription: test\nnodes:\n  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n").unwrap();
        let gate = fixture
            .host
            .runtime_activation_gate(&active.execution_id)
            .await;
        let activation_guard = if before_commit {
            Some(gate.lock.lock().await)
        } else {
            None
        };
        let shutdown_guard = if before_commit {
            None
        } else {
            Some(fixture.host.active_command_executions.lock().await)
        };
        let mut abort = Box::pin(fixture.host.abort_workflow_execution(
            &fixture.app,
            &active.execution_id,
            if abort_succeeds {
                None
            } else {
                Some("other-node")
            },
        ));
        assert!(futures_util::poll!(abort.as_mut()).is_pending());

        // When
        let mut same = Box::pin(fixture.host.start_resolved_workflow(
            &fixture.app,
            workflow.clone(),
            "/repo/".into(),
            None,
            ExecutionOrigin::Cli,
        ));
        assert!(futures_util::poll!(same.as_mut()).is_pending());
        let other_id = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            fixture.host.start_resolved_workflow(
                &fixture.app,
                workflow,
                "/other".into(),
                None,
                ExecutionOrigin::Cli,
            ),
        )
        .await
        .expect("別worktreeの起動はAbortの終了を待たない")
        .unwrap();

        // Then
        assert_eq!(
            fixture
                .host
                .load_execution(&fixture.app, &active.execution_id)
                .await
                .unwrap()
                .is_active(),
            before_commit
        );
        assert!(
            !workflow_fact_log::read_tree_records(&fixture.store, &other_id)
                .unwrap()
                .is_empty()
        );
        assert!(futures_util::poll!(same.as_mut()).is_pending());
        drop(activation_guard);
        drop(shutdown_guard);
        assert_eq!(abort.await.is_ok(), abort_succeeds);
        let result = same.await;
        if abort_succeeds {
            assert_ne!(result.unwrap(), active.execution_id);
        } else {
            assert!(result
                .unwrap_err()
                .to_string()
                .contains(&active.execution_id));
        }
    }
}

#[tokio::test]
async fn test_commit排他_同一実行木で共有し不要なlockを保持し続けない() {
    // Given
    let fixture = test_helpers::Fixture::new(0);
    let lock = fixture.host.commit_lock("tree-a").await;
    let guard = lock.lock().await;
    let waiter = fixture.host.clone().commit_lock("tree-a").await;

    // When / Then
    assert!(Arc::ptr_eq(&lock, &waiter));
    assert!(waiter.try_lock().is_err());
    let other = fixture.host.commit_lock("tree-b").await;
    assert!(other.try_lock().is_ok());
    let weak = Arc::downgrade(&lock);
    drop(guard);
    drop(lock);
    assert!(weak.upgrade().is_some());
    drop(waiter);
    assert!(weak.upgrade().is_none());
    let _next = fixture.host.commit_lock("tree-b").await;
    let locks = fixture.host.commit_locks.lock().await;
    assert_eq!(locks.len(), 1);
    assert!(locks.contains_key("tree-b"));
}

#[tokio::test]
async fn test_commit排他_別実行木のcommitとcommand起動判定を妨げない() {
    // Given
    let fixture = test_helpers::Fixture::new(0);
    let first = fixture
        .persist_started("  main: {command: true}\n", "/repo")
        .await;
    let second = fixture
        .persist_started("  main: {command: true}\n", "/other")
        .await;
    let lock = fixture.host.commit_lock(&first.execution_id).await;
    let guard = lock.lock().await;
    let first_input = command_input(&first);
    let mut same = Box::pin(fixture.host.commit_command_spawned(
        &fixture.app,
        &first_input,
        "true".into(),
    ));
    assert!(futures_util::poll!(same.as_mut()).is_pending());

    // When / Then
    let second_input = command_input(&second);
    let mut other = Box::pin(fixture.host.commit_command_spawned(
        &fixture.app,
        &second_input,
        "true".into(),
    ));
    assert!(matches!(
        futures_util::poll!(other.as_mut()),
        std::task::Poll::Ready(Ok(true))
    ));
    drop(other);
    fixture
        .host
        .abort_workflow_execution(&fixture.app, &second.execution_id, None)
        .await
        .unwrap();
    let mut spawn = Box::pin(
        fixture
            .host
            .spawn_command_execution(&fixture.app, second_input),
    );
    assert!(matches!(
        futures_util::poll!(spawn.as_mut()),
        std::task::Poll::Ready(Ok(()))
    ));
    assert!(fixture
        .host
        .node_processes
        .active_commands
        .lock()
        .unwrap()
        .is_empty());
    assert!(futures_util::poll!(same.as_mut()).is_pending());
    drop(guard);
    assert!(same.await.unwrap());
}

#[tokio::test]
async fn test_worktree排他_待機者と共有し不要なlockを保持し続けない() {
    // Given
    let fixture = test_helpers::Fixture::new(0);
    let lock = fixture.host.workflow_start_lock("/repo/").await;
    let guard = lock.lock().await;
    let waiter = fixture.host.workflow_start_lock("/repo").await;

    // When / Then
    assert!(Arc::ptr_eq(&lock, &waiter));
    assert!(waiter.try_lock().is_err());
    let weak = Arc::downgrade(&lock);
    drop(guard);
    drop(lock);
    assert!(weak.upgrade().is_some());
    drop(waiter);
    assert!(weak.upgrade().is_none());
    let _next = fixture.host.workflow_start_lock("/other").await;
    let locks = fixture.host.workflow_start_locks.lock().await;
    assert_eq!(locks.len(), 1);
    assert!(locks.contains_key("/other"));
}

#[tokio::test]
async fn test_worktree排他_abort対象の所在地を読めなければ記録を変更せずエラーにする() {
    // Given
    let fixture = test_helpers::Fixture::new(0);
    let active = fixture
        .persist_started("  main: {session: {provider: codex}}\n", "/repo")
        .await;
    let records =
        workflow_fact_log::read_tree_records(&fixture.store, &active.execution_id).unwrap();
    let mut broken =
        workflow_fact_log::pending_single_fact(&records[0].meta, &records[0].fact, 1_000).unwrap();
    broken.row.tree_id = "broken".into();
    broken.row.detail = "{".into();
    fixture
        .store
        .append_node_event_blocking(broken.row, Some(1_000))
        .unwrap();
    let mut unavailable = fixture.app.clone();
    unavailable.store = None;

    // When / Then
    assert!(matches!(
        fixture
            .host
            .abort_workflow_execution(&unavailable, &active.execution_id, None)
            .await,
        Err(WorkflowRuntimeError::SessionStore(_))
    ));
    assert!(
        matches!(fixture.host.abort_workflow_execution(&fixture.app, "missing", None).await,
        Err(WorkflowRuntimeError::ExecutionNotFound(id)) if id == "missing")
    );
    assert!(matches!(
        fixture
            .host
            .abort_workflow_execution(&fixture.app, "broken", None)
            .await,
        Err(WorkflowRuntimeError::SessionStore(_))
    ));
    assert_eq!(
        workflow_fact_log::read_tree_records(&fixture.store, &active.execution_id).unwrap(),
        records
    );
    assert!(fixture.host.workflow_start_locks.lock().await.is_empty());
}

fn command_input(snapshot: &RuntimeCommitSnapshot) -> CommandExecutionInput {
    let node = &snapshot.node_executions[0];
    CommandExecutionInput {
        execution_id: snapshot.execution_id.clone(),
        node_execution_id: node.id.clone(),
        node_name: node.node_name.clone(),
        attempt: node.attempt,
        worktree_path: snapshot.worktree_path.clone(),
        raw_command: Some("true".into()),
        definition_env: Vec::new(),
        contract: None,
        schemas: Default::default(),
        session_id: None,
    }
}

fn command_output() -> CommandRunOutput {
    CommandRunOutput {
        exit_code: 0,
        stdout: "done".into(),
        stderr: String::new(),
        duration_ms: 1,
    }
}

#[tokio::test]
async fn test_command反映_登録なしの起動と結果を保存し確定後は理由をログに残す() {
    // Given
    crate::test_support::install_capturing_logger();
    let fixture = test_helpers::Fixture::new(0);
    let snapshot = fixture
        .persist_started("  main: {command: true}\n", "/repo")
        .await;
    let host = fixture.restarted_host();
    let input = command_input(&snapshot);
    // When
    assert!(host
        .commit_command_spawned(&fixture.app, &input, "true".into())
        .await
        .unwrap());
    host.commit_command_output(&fixture.app, input.clone(), command_output())
        .await
        .unwrap();
    let completed =
        workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap();
    host.commit_command_output(&fixture.app, input.clone(), command_output())
        .await
        .unwrap();
    assert!(!host
        .commit_command_spawned(&fixture.app, &input, "late".into())
        .await
        .unwrap());
    host.fail_current_command_node(&fixture.app, &input, "late failure".into())
        .await
        .unwrap();
    // Then
    assert!(completed.iter().any(|record| matches!(
        record.fact,
        crate::domain::workflow::NodeFact::CommandSpawned(_)
    )));
    assert!(completed.iter().any(|record| matches!(
        record.fact,
        crate::domain::workflow::NodeFact::ExecutionCompleted
    )));
    assert_eq!(
        workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap(),
        completed
    );
    let warnings = crate::test_support::captured_warning_messages();
    assert_eq!(
        warnings
            .iter()
            .filter(|message| message.contains(&input.node_execution_id)
                && message.contains("was not applied: execution tree is terminal"))
            .count(),
        3
    );
}

#[tokio::test]
async fn test_command失敗_登録なしの最新attemptに保存し別attemptは反映しない() {
    // Given
    let fixture = test_helpers::Fixture::new(0);
    let snapshot = fixture
        .persist_started("  main: {command: true}\n", "/repo")
        .await;
    let host = fixture.restarted_host();
    let input = command_input(&snapshot);
    // When
    host.fail_current_command_node(&fixture.app, &input, "process wait failed".into())
        .await
        .unwrap();
    let before =
        workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap();
    let mut stale = input.clone();
    stale.attempt += 1;
    host.fail_current_command_node(&fixture.app, &stale, "wrong attempt".into())
        .await
        .unwrap();
    // Then
    assert!(before.iter().any(|record| matches!(
        record.fact,
        crate::domain::workflow::NodeFact::RuntimeFailureObserved(_)
    )));
    assert_eq!(
        workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap(),
        before
    );
}

#[tokio::test]
async fn test_commit結果不明_一部や別内容は競合とし保存済みbatchは再追記しない() {
    for change in ["partial", "session", "timestamp", "complete"] {
        // Given
        let fixture = test_helpers::Fixture::new(0);
        let snapshot = fixture
            .persist_started("  main: {session: {provider: codex}}\n", "/repo")
            .await;
        let records =
            workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap();
        let head = records.last().unwrap().seq;
        let events = [
            WorkflowEvent::SessionAttached {
                execution_id: snapshot.execution_id.clone(),
                node_execution_id: records[0].meta.node_execution_id.clone(),
                session_id: "expected-session".into(),
                timestamp: 2.0,
            },
            WorkflowEvent::NodeStopReceived {
                execution_id: snapshot.execution_id.clone(),
                node_execution_id: records[0].meta.node_execution_id.clone(),
                timestamp: 3.0,
            },
        ];
        let mut concurrent = events.to_vec();
        match change {
            "partial" => concurrent.truncate(1),
            "session" | "timestamp" => {
                let WorkflowEvent::SessionAttached {
                    session_id,
                    timestamp,
                    ..
                } = &mut concurrent[0]
                else {
                    unreachable!()
                };
                if change == "session" {
                    *session_id = "other-session".into();
                } else {
                    *timestamp += 1.0;
                }
            }
            "complete" => concurrent.push(WorkflowEvent::ExecutionAborted {
                execution_id: snapshot.execution_id.clone(),
                aborted_node: None,
                timestamp: 4.0,
            }),
            _ => unreachable!(),
        }
        workflow_fact_log::append_facts_for_events(&fixture.store, &concurrent).unwrap();
        let before =
            workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap();
        fixture.store.fault_injector().arm_drop_reply();

        // When
        let result = WorkflowRuntimeHost::append_events_at_head(
            &fixture.app,
            &snapshot.execution_id,
            head,
            &events,
        );

        // Then
        if change == "complete" {
            result.unwrap();
        } else {
            assert!(
                matches!(result, Err(WorkflowRuntimeError::Conflict(_))),
                "{result:?}"
            );
        }
        assert_eq!(
            workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap(),
            before
        );
    }
}

#[tokio::test]
async fn test_commit結果不明_記録を読み直せなければ保存成功にしない() {
    use crate::adaptor::gateway::local_event_store::layout::StoreLayout;

    // Given
    let fixture = test_helpers::Fixture::new(0);
    let snapshot = fixture
        .persist_started("  main: {session: {provider: codex}}\n", "/repo")
        .await;
    let records =
        workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap();
    let head = records.last().unwrap().seq;
    let event = WorkflowEvent::SessionAttached {
        execution_id: snapshot.execution_id.clone(),
        node_execution_id: records[0].meta.node_execution_id.clone(),
        session_id: "expected-session".into(),
        timestamp: 2.0,
    };
    let stall = fixture.store.fault_injector().arm_node_event_append_stall();
    fixture.store.fault_injector().arm_drop_reply();
    let app = fixture.app.clone();
    let commit = std::thread::spawn(move || {
        WorkflowRuntimeHost::append_events_at_head(&app, &snapshot.execution_id, head, &[event])
    });
    stall.wait_until_arrived();

    // When
    let connection =
        rusqlite::Connection::open(StoreLayout::new(fixture._directory.path()).database_path())
            .unwrap();
    connection
        .execute_batch("ALTER TABLE node_events RENAME TO unavailable_node_events;")
        .unwrap();
    stall.release();
    let error = commit.join().unwrap().unwrap_err();

    // Then
    assert!(
        matches!(error, WorkflowRuntimeError::SessionStore(reason) if reason.contains("control-plane commit readback failed"))
    );
}

#[tokio::test]
async fn test_commit競合_候補作成後に外部が保存した事実を上書きしない() {
    // Given
    let fixture = test_helpers::Fixture::new(0);
    let snapshot = fixture
        .persist_started("  main: {session: {provider: codex}}\n", "/repo")
        .await;
    let before = fixture
        .host
        .load_execution(&fixture.app, &snapshot.execution_id)
        .await
        .unwrap();
    let mut candidate = before.clone();
    candidate.transition_aborted();
    let records =
        workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap();
    workflow_fact_log::append_facts_for_events(
        &fixture.store,
        &[WorkflowEvent::NodeStopReceived {
            execution_id: snapshot.execution_id.clone(),
            node_execution_id: records[0].meta.node_execution_id.clone(),
            timestamp: current_timestamp() + 0.001,
        }],
    )
    .unwrap();
    let advanced =
        workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap();
    // When
    let error = fixture
        .host
        .commit_control_plane_candidate(
            &fixture.app,
            ControlPlaneCommitCandidate {
                execution_id: &snapshot.execution_id,
                snapshot_before: before,
                candidate,
                transition_outcome: TransitionOutcome::Applied,
                events: &[WorkflowEvent::ExecutionAborted {
                    execution_id: snapshot.execution_id.clone(),
                    aborted_node: None,
                    timestamp: current_timestamp(),
                }],
                provider_events: Vec::new(),
            },
        )
        .await
        .unwrap_err();
    // Then
    assert!(matches!(error, WorkflowRuntimeError::Conflict(_)));
    assert_eq!(
        workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap(),
        advanced
    );
}

#[tokio::test]
async fn test_command反映_実行木がない場合は起動と結果と失敗の不反映理由を残す() {
    // Given
    crate::test_support::install_capturing_logger();
    let fixture = test_helpers::Fixture::new(0);
    let snapshot = fixture
        .persist_started("  main: {command: true}\n", "/repo")
        .await;
    let mut input = command_input(&snapshot);
    input.execution_id = format!("missing-{}", snapshot.execution_id);
    // When
    assert!(!fixture
        .host
        .commit_command_spawned(&fixture.app, &input, "true".into())
        .await
        .unwrap());
    fixture
        .host
        .commit_command_output(&fixture.app, input.clone(), command_output())
        .await
        .unwrap();
    fixture
        .host
        .fail_current_command_node(&fixture.app, &input, "missing".into())
        .await
        .unwrap();
    // Then
    let warnings = crate::test_support::captured_warning_messages();
    assert_eq!(
        warnings
            .iter()
            .filter(|message| message.contains(&input.execution_id)
                && message.contains("was not applied: execution tree was not found"))
            .count(),
        3
    );
    assert!(
        workflow_fact_log::read_tree_records(&fixture.store, &input.execution_id)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn test_記録からの承認_外部writerの完了信号で承認待ちになったnodeを承認する() {
    use crate::domain::workflow::NodeFact;
    use crate::usecase::workflow::command::ApprovalCommand;
    // Given
    let fixture = test_helpers::Fixture::new(0);
    let snapshot = fixture
        .persist_started(
            "  main:\n    session: {provider: codex}\n    completion: {require: approval}\n",
            "/repo",
        )
        .await;
    let before = fixture
        .host
        .load_execution(&fixture.app, &snapshot.execution_id)
        .await
        .unwrap();
    let node = &before.node_executions[0];
    workflow_fact_log::append_facts_for_events(
        &fixture.store,
        &[
            WorkflowEvent::NodeSubmitReceived {
                execution_id: before.id.clone(),
                node_execution_id: node.id.clone(),
                timestamp: current_timestamp(),
            },
            WorkflowEvent::NodeStopReceived {
                execution_id: before.id.clone(),
                node_execution_id: node.id.clone(),
                timestamp: current_timestamp(),
            },
        ],
    )
    .unwrap();
    let control =
        WorkflowControlPlaneUsecase::new(Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
            fixture.app.clone(),
            Arc::new(fixture.host.clone()),
        )));
    // When
    control
        .resolve_approval(ApprovalCommand {
            execution_id: before.id.clone(),
            node_name: node.node_name.clone(),
            node_execution_id: Some(node.id.clone()),
            comment: None,
        })
        .await
        .unwrap();
    // Then
    let records = workflow_fact_log::read_tree_records(&fixture.store, &before.id).unwrap();
    assert!(records
        .iter()
        .any(|record| matches!(record.fact, NodeFact::ApprovalGranted(_))));
    assert!(records
        .iter()
        .any(|record| matches!(record.fact, NodeFact::ExecutionCompleted)));
}

#[tokio::test(start_paused = true)]
async fn test_記録からのretry_外部writerが作った最新attemptを再試行する() {
    use crate::domain::workflow::NodeFact;
    use crate::usecase::workflow::command::RetryNodeCommand;
    // Given
    let fixture = test_helpers::Fixture::new(0);
    let snapshot = fixture
        .persist_started(
            "  main:\n    command: true\n    input: [document]\n    env: {DOC: document}\n",
            fixture._directory.path().to_str().unwrap(),
        )
        .await;
    let before = fixture
        .host
        .load_execution(&fixture.app, &snapshot.execution_id)
        .await
        .unwrap();
    let node = &before.node_executions[0];
    let next_id = uuid::Uuid::new_v4().to_string();
    workflow_fact_log::append_facts_for_events(
        &fixture.store,
        &[
            WorkflowEvent::NodeRetryRequested {
                execution_id: before.id.clone(),
                node_execution_id: node.id.clone(),
                timestamp: current_timestamp(),
            },
            WorkflowEvent::NodeStarted {
                execution_id: before.id.clone(),
                node_execution_id: next_id.clone(),
                node_name: node.node_name.clone(),
                kind: node.kind,
                attempt: 2,
                parent: node.parent.clone(),
                worktree: None,
                timestamp: current_timestamp(),
            },
        ],
    )
    .unwrap();
    let control =
        WorkflowControlPlaneUsecase::new(Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
            fixture.app.clone(),
            Arc::new(fixture.host.clone()),
        )));
    // When
    control
        .retry_node(RetryNodeCommand {
            execution_id: before.id.clone(),
            node_execution_id: next_id.clone(),
        })
        .await
        .unwrap();
    fixture.wait_startup_retries().await;
    // Then
    let records = workflow_fact_log::read_tree_records(&fixture.store, &before.id).unwrap();
    assert!(records
        .iter()
        .any(|record| record.meta.node_execution_id == next_id
            && matches!(record.fact, NodeFact::RetryRequested)));
    assert!(records
        .iter()
        .any(|record| record.meta.attempt == 3 && matches!(record.fact, NodeFact::Started(_))));
    assert!(fixture
        .host
        .node_processes
        .active_commands
        .lock()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn test_abort競合_外部writerの追記後も最新記録を中止する() {
    use crate::domain::workflow::NodeFact;
    // Given
    let fixture = test_helpers::Fixture::new(0);
    let snapshot = fixture
        .persist_started("  main: {command: true}\n", "/repo")
        .await;
    let meta = workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id)
        .unwrap()[0]
        .meta
        .clone();
    let commit_lock = fixture.host.commit_lock(&snapshot.execution_id).await;
    let guard = commit_lock.lock().await;
    let mut abort = Box::pin(fixture.host.abort_workflow_execution(
        &fixture.app,
        &snapshot.execution_id,
        None,
    ));
    assert!(futures_util::poll!(abort.as_mut()).is_pending());
    // When
    workflow_fact_log::append_single_fact(
        &fixture.store,
        &meta,
        &NodeFact::CommandSpawned(crate::domain::workflow::CommandSpawnedFact {
            display_command: "external".into(),
        }),
        2000,
    )
    .unwrap();
    drop(guard);
    abort.await.unwrap();
    // Then
    let records =
        workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(record.fact, NodeFact::AbortRequested(_)))
            .count(),
        1
    );
    assert!(records
        .iter()
        .any(|record| matches!(record.fact, NodeFact::CommandSpawned(_))));
}

#[tokio::test]
async fn test_command結果競合_最新記録で成功を保存し終端なら理由付きで反映しない() {
    use crate::domain::workflow::NodeFact;
    for abort in [false, true] {
        // Given
        crate::test_support::install_capturing_logger();
        let fixture = test_helpers::Fixture::new(0);
        let snapshot = fixture
            .persist_started("  main: {command: true}\n", "/repo")
            .await;
        let input = command_input(&snapshot);
        let meta = workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id)
            .unwrap()[0]
            .meta
            .clone();
        let commit_lock = fixture.host.commit_lock(&snapshot.execution_id).await;
        let guard = commit_lock.lock().await;
        let mut completion = Box::pin(fixture.host.finish_command_execution(
            &fixture.app,
            input.clone(),
            Ok(command_output()),
        ));
        assert!(futures_util::poll!(completion.as_mut()).is_pending());
        // When
        workflow_fact_log::append_single_fact(
            &fixture.store,
            &meta,
            &if abort {
                NodeFact::AbortRequested(Default::default())
            } else {
                NodeFact::CommandSpawned(crate::domain::workflow::CommandSpawnedFact {
                    display_command: "external".into(),
                })
            },
            2000,
        )
        .unwrap();
        let before =
            workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap();
        drop(guard);
        completion.await;
        // Then
        let records =
            workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap();
        assert!(!records
            .iter()
            .any(|record| matches!(record.fact, NodeFact::RuntimeFailureObserved(_))));
        if abort {
            assert_eq!(records, before);
            assert!(crate::test_support::captured_warning_messages()
                .iter()
                .any(|message| message.contains(&input.node_execution_id)
                    && message.contains("was not applied: execution tree is terminal")));
        } else {
            assert_eq!(
                records
                    .iter()
                    .filter(|record| matches!(record.fact, NodeFact::ArtifactProduced(_)))
                    .count(),
                1
            );
            assert_eq!(
                records
                    .iter()
                    .filter(|record| matches!(record.fact, NodeFact::ExecutionCompleted))
                    .count(),
                1
            );
            let execution = fixture
                .host
                .load_execution(&fixture.app, &snapshot.execution_id)
                .await
                .unwrap();
            assert_eq!(
                execution.node_executions[0].artifact.as_ref().unwrap()["ok"],
                true
            );
        }
    }
}

#[tokio::test]
async fn test_操作競合_abortとcommand結果は上限で止まり障害事実を追加しない() {
    use crate::domain::workflow::NodeFact;
    use crate::usecase::workflow::command::CONTROL_PLANE_MAX_ATTEMPTS;
    for command in [false, true] {
        // Given
        crate::test_support::install_capturing_logger();
        let fixture = test_helpers::Fixture::new(0);
        let snapshot = fixture
            .persist_started("  main: {command: true}\n", "/repo")
            .await;
        let input = command_input(&snapshot);
        let meta = workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id)
            .unwrap()[0]
            .meta
            .clone();
        let commit_lock = fixture.host.commit_lock(&snapshot.execution_id).await;
        let mut guard = commit_lock.lock().await;
        let mut operation = Box::pin(async {
            if command {
                fixture
                    .host
                    .finish_command_execution(&fixture.app, input.clone(), Ok(command_output()))
                    .await;
                Ok(())
            } else {
                fixture
                    .host
                    .abort_workflow_execution(&fixture.app, &snapshot.execution_id, None)
                    .await
            }
        });
        // When
        for attempt in 0..CONTROL_PLANE_MAX_ATTEMPTS {
            assert!(futures_util::poll!(operation.as_mut()).is_pending());
            workflow_fact_log::append_single_fact(
                &fixture.store,
                &meta,
                &NodeFact::CommandSpawned(crate::domain::workflow::CommandSpawnedFact {
                    display_command: format!("external-{attempt}"),
                }),
                2000 + attempt as i64 * 1000,
            )
            .unwrap();
            if attempt + 1 == CONTROL_PLANE_MAX_ATTEMPTS {
                drop(guard);
                break;
            }
            let mut next_guard = Box::pin(commit_lock.lock());
            assert!(futures_util::poll!(next_guard.as_mut()).is_pending());
            drop(guard);
            assert!(futures_util::poll!(operation.as_mut()).is_pending());
            guard = next_guard.await;
        }
        let result = operation.await;
        // Then
        if command {
            result.unwrap();
            assert!(crate::test_support::captured_warning_messages()
                .iter()
                .any(|message| message.contains(&input.node_execution_id)
                    && message.contains("result was not applied")
                    && message.contains("conflict")));
        } else {
            assert!(matches!(result, Err(WorkflowRuntimeError::Conflict(_))));
        }
        let records =
            workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap();
        assert_eq!(records.len(), CONTROL_PLANE_MAX_ATTEMPTS + 1);
        assert!(records.iter().all(|record| matches!(
            record.fact,
            NodeFact::Started(_) | NodeFact::CommandSpawned(_)
        )));
        fixture
            .host
            .abort_workflow_execution(&fixture.app, &snapshot.execution_id, None)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn test_command起動失敗競合_最新attemptへ保存し終端や更新済みattemptは理由付きで反映しない() {
    use crate::domain::workflow::NodeFact;
    for spawned in [true, false] {
        for change in ["current", "abort", "retry"] {
            // Given
            crate::test_support::install_capturing_logger();
            let fixture = test_helpers::Fixture::new(0);
            let snapshot = fixture
                .persist_started("  main: {command: true}\n", "/repo")
                .await;
            let input = command_input(&snapshot);
            let commit_lock = fixture.host.commit_lock(&snapshot.execution_id).await;
            let guard = commit_lock.lock().await;
            let mut operation = Box::pin(async {
                if spawned {
                    fixture
                        .host
                        .commit_command_spawned(&fixture.app, &input, "true".into())
                        .await
                } else {
                    fixture
                        .host
                        .fail_current_command_node(
                            &fixture.app,
                            &input,
                            "process wait failed".into(),
                        )
                        .await
                        .map(|()| true)
                }
            });
            assert!(futures_util::poll!(operation.as_mut()).is_pending());
            // When
            let timestamp = current_timestamp();
            let events = match change {
                "abort" => vec![WorkflowEvent::ExecutionAborted {
                    execution_id: input.execution_id.clone(),
                    aborted_node: None,
                    timestamp,
                }],
                "retry" => vec![
                    WorkflowEvent::NodeRetryRequested {
                        execution_id: input.execution_id.clone(),
                        node_execution_id: input.node_execution_id.clone(),
                        timestamp,
                    },
                    WorkflowEvent::NodeStarted {
                        execution_id: input.execution_id.clone(),
                        node_execution_id: uuid::Uuid::new_v4().to_string(),
                        node_name: input.node_name.clone(),
                        kind: NodeKindName::Command,
                        attempt: input.attempt + 1,
                        parent: None,
                        worktree: None,
                        timestamp,
                    },
                ],
                _ => vec![WorkflowEvent::CommandSpawned {
                    execution_id: input.execution_id.clone(),
                    node_execution_id: input.node_execution_id.clone(),
                    display_command: "external".into(),
                    timestamp,
                }],
            };
            workflow_fact_log::append_facts_for_events(&fixture.store, &events).unwrap();
            let before =
                workflow_fact_log::read_tree_records(&fixture.store, &input.execution_id).unwrap();
            drop(guard);
            let applied = operation.await.unwrap();
            // Then
            let records =
                workflow_fact_log::read_tree_records(&fixture.store, &input.execution_id).unwrap();
            if change == "current" {
                assert!(applied);
                assert_eq!(records.len(), before.len() + 1);
                assert_eq!(&records[..before.len()], before);
                let record = records.last().unwrap();
                assert_eq!(record.meta.node_execution_id, input.node_execution_id);
                assert_eq!(record.meta.attempt, input.attempt);
                if spawned {
                    assert!(
                        matches!(&record.fact, NodeFact::CommandSpawned(fact) if fact.display_command == "true")
                    );
                } else {
                    assert!(
                        matches!(&record.fact, NodeFact::RuntimeFailureObserved(fact) if fact.reason == "process wait failed" && fact.failure_kind == NodeExecutionFailureKind::InfrastructureCrash)
                    );
                }
            } else {
                if spawned {
                    assert!(!applied);
                }
                assert_eq!(records, before);
                let reason = if change == "abort" {
                    "execution tree is terminal"
                } else {
                    "command NodeExecution is no longer running"
                };
                assert!(crate::test_support::captured_warning_messages()
                    .iter()
                    .any(|message| message.contains(&input.node_execution_id)
                        && message.contains("was not applied")
                        && message.contains(reason)));
            }
        }
    }
}

#[tokio::test]
async fn test_command起動失敗競合_上限で不反映理由を残し競合を障害事実にしない() {
    use crate::domain::workflow::NodeFact;
    use crate::usecase::workflow::command::CONTROL_PLANE_MAX_ATTEMPTS;
    for spawned in [true, false] {
        // Given
        crate::test_support::install_capturing_logger();
        let fixture = test_helpers::Fixture::new(0);
        let snapshot = fixture
            .persist_started("  main: {command: true}\n", "/repo")
            .await;
        let input = command_input(&snapshot);
        let commit_lock = fixture.host.commit_lock(&snapshot.execution_id).await;
        let mut guard = commit_lock.lock().await;
        let mut operation = Box::pin(async {
            if spawned {
                assert!(!fixture
                    .host
                    .commit_command_spawned(&fixture.app, &input, "true".into())
                    .await
                    .unwrap());
            } else {
                fixture
                    .host
                    .finish_command_execution(
                        &fixture.app,
                        input.clone(),
                        Err(CommandRunnerError::Wait(std::io::Error::other(
                            "process wait failed",
                        ))),
                    )
                    .await;
            }
        });
        // When
        for attempt in 0..CONTROL_PLANE_MAX_ATTEMPTS {
            assert!(futures_util::poll!(operation.as_mut()).is_pending());
            workflow_fact_log::append_facts_for_events(
                &fixture.store,
                &[WorkflowEvent::CommandSpawned {
                    execution_id: input.execution_id.clone(),
                    node_execution_id: input.node_execution_id.clone(),
                    display_command: format!("external-{attempt}"),
                    timestamp: current_timestamp(),
                }],
            )
            .unwrap();
            if attempt + 1 == CONTROL_PLANE_MAX_ATTEMPTS {
                drop(guard);
                break;
            }
            let mut next_guard = Box::pin(commit_lock.lock());
            assert!(futures_util::poll!(next_guard.as_mut()).is_pending());
            drop(guard);
            assert!(futures_util::poll!(operation.as_mut()).is_pending());
            guard = next_guard.await;
        }
        operation.await;
        // Then
        let records =
            workflow_fact_log::read_tree_records(&fixture.store, &input.execution_id).unwrap();
        assert_eq!(records.len(), CONTROL_PLANE_MAX_ATTEMPTS + 1);
        assert!(records.iter().all(|record| matches!(
            record.fact,
            NodeFact::Started(_) | NodeFact::CommandSpawned(_)
        )));
        let expected = if spawned {
            "start was not applied"
        } else {
            "failure was not applied"
        };
        assert!(crate::test_support::captured_warning_messages()
            .iter()
            .any(|message| message.contains(&input.node_execution_id)
                && message.contains(expected)
                && message.contains("conflict")));
    }
}

#[tokio::test]
async fn test_session準備競合_最新記録で紐付けを再評価し準備を繰り返さない() {
    use crate::domain::workflow::NodeFact;
    use crate::usecase::workflow::command::CONTROL_PLANE_MAX_ATTEMPTS;
    for change in ["sibling", "abort", "attached", "exhausted"] {
        // Given
        let fixture = test_helpers::Fixture::new(0);
        let snapshot = fixture.persist_started(
            "  main: {fanout: {children: [work, other]}}\n  work: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n  other: {command: 'must-not-start'}",
            "/repo",
        ).await;
        let execution = fixture
            .host
            .load_execution(&fixture.app, &snapshot.execution_id)
            .await
            .unwrap();
        let target = execution
            .node_executions
            .iter()
            .find(|node| node.node_name == "work")
            .unwrap();
        let sibling = execution
            .node_executions
            .iter()
            .find(|node| node.node_name == "other")
            .unwrap();
        let starts = vec![NodeStart::Leaf(
            execution.leaf_start_for(&target.id).unwrap(),
        )];
        let commit_lock = fixture.host.commit_lock(&snapshot.execution_id).await;
        let mut guard = commit_lock.lock().await;
        let mut operation = Box::pin(fixture.host.start_nodes(
            &fixture.app,
            &snapshot.execution_id,
            "/repo",
            starts,
        ));
        let attempts = if change == "exhausted" {
            CONTROL_PLANE_MAX_ATTEMPTS
        } else {
            1
        };

        // When
        for attempt in 0..attempts {
            assert!(futures_util::poll!(operation.as_mut()).is_pending());
            workflow_fact_log::append_facts_for_events(
                &fixture.store,
                &[match change {
                    "abort" => WorkflowEvent::ExecutionAborted {
                        execution_id: snapshot.execution_id.clone(),
                        aborted_node: None,
                        timestamp: current_timestamp(),
                    },
                    "attached" => WorkflowEvent::SessionAttached {
                        execution_id: snapshot.execution_id.clone(),
                        node_execution_id: target.id.clone(),
                        session_id: "external-session".into(),
                        timestamp: current_timestamp(),
                    },
                    _ => WorkflowEvent::CommandSpawned {
                        execution_id: snapshot.execution_id.clone(),
                        node_execution_id: sibling.id.clone(),
                        display_command: format!("external-{attempt}"),
                        timestamp: current_timestamp(),
                    },
                }],
            )
            .unwrap();
            if attempt + 1 == attempts {
                drop(guard);
                break;
            }
            let mut next_guard = Box::pin(commit_lock.lock());
            assert!(futures_util::poll!(next_guard.as_mut()).is_pending());
            drop(guard);
            assert!(futures_util::poll!(operation.as_mut()).is_pending());
            guard = next_guard.await;
        }
        let result = operation.await;

        // Then
        if change == "sibling" {
            result.unwrap();
            assert!(fixture.sessions.rolled_back.lock().unwrap().is_empty());
            assert_eq!(
                *fixture.sessions.activated.lock().unwrap(),
                [target.id.clone()]
            );
        } else {
            let error = result.unwrap_err();
            if change == "exhausted" {
                assert!(matches!(error, WorkflowRuntimeError::Conflict(_)));
                assert!(matches!(
                    fixture
                        .host
                        .settle_runtime_failure(&fixture.app, &snapshot.execution_id, &error,)
                        .await,
                    Err(WorkflowRuntimeError::Conflict(_))
                ));
            } else {
                assert!(matches!(error, WorkflowRuntimeError::InvalidState(_)));
            }
            assert_eq!(
                *fixture.sessions.rolled_back.lock().unwrap(),
                [target.id.clone()]
            );
            assert!(fixture.sessions.activated.lock().unwrap().is_empty());
        }
        assert_eq!(fixture.sessions.prepared.lock().unwrap().len(), 1);
        let records =
            workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record.fact, NodeFact::SessionAttached(_)))
                .count(),
            usize::from(matches!(change, "sibling" | "attached"))
        );
        assert!(!records
            .iter()
            .any(|record| matches!(record.fact, NodeFact::RuntimeFailureObserved(_))));
        let latest = fixture
            .host
            .load_execution(&fixture.app, &snapshot.execution_id)
            .await
            .unwrap();
        let expected = match change {
            "sibling" => Some(format!("agent-{}", target.id)),
            "attached" => Some("external-session".into()),
            _ => None,
        };
        assert_eq!(
            latest.node_execution(&target.id).unwrap().session_id,
            expected
        );
    }
}

#[tokio::test]
async fn test_runtime再試行_競合だけを上限まで再実行する() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    for (conflicts, storage_error) in [(0, false), (1, false), (4, false), (0, true)] {
        // Given
        let calls = AtomicUsize::new(0);
        // When
        let result = retry_runtime_conflicts(|| async {
            let attempt = calls.fetch_add(1, Ordering::SeqCst);
            if storage_error {
                Err(WorkflowRuntimeError::SessionStore("unavailable".into()))
            } else if attempt < conflicts {
                Err(WorkflowRuntimeError::Conflict("advanced".into()))
            } else {
                Ok(())
            }
        })
        .await;
        // Then
        assert_eq!(
            calls.load(Ordering::SeqCst),
            (conflicts + 1).min(crate::usecase::workflow::command::CONTROL_PLANE_MAX_ATTEMPTS)
        );
        assert_eq!(result.is_ok(), !storage_error && conflicts < 4);
    }
}

#[tokio::test]
async fn test_自動再起動競合_最新記録で再評価し上限まで再試行する() {
    use crate::domain::workflow::NodeFact;
    use crate::usecase::workflow::command::CONTROL_PLANE_MAX_ATTEMPTS;
    for change in ["sibling", "abort", "exhausted"] {
        // Given
        let fixture = test_helpers::Fixture::new(0);
        let snapshot = fixture.persist_started(
            "  main: {fanout: {children: [work, other]}}\n  work: {command: 'must-not-start'}\n  other: {command: 'must-not-start'}",
            "/repo",
        ).await;
        let execution = fixture
            .host
            .load_execution(&fixture.app, &snapshot.execution_id)
            .await
            .unwrap();
        let target = execution
            .node_executions
            .iter()
            .find(|node| node.node_name == "work")
            .unwrap();
        let sibling = execution
            .node_executions
            .iter()
            .find(|node| node.node_name == "other")
            .unwrap();
        let commit_lock = fixture.host.commit_lock(&snapshot.execution_id).await;
        let mut guard = commit_lock.lock().await;
        let mut operation = Box::pin(fixture.host.restart_node_attempt(
            &fixture.app,
            &snapshot.execution_id,
            &target.id,
        ));
        let attempts = if change == "exhausted" {
            CONTROL_PLANE_MAX_ATTEMPTS
        } else {
            1
        };

        // When
        for attempt in 0..attempts {
            assert!(futures_util::poll!(operation.as_mut()).is_pending());
            workflow_fact_log::append_facts_for_events(
                &fixture.store,
                &[if change == "abort" {
                    WorkflowEvent::ExecutionAborted {
                        execution_id: snapshot.execution_id.clone(),
                        aborted_node: None,
                        timestamp: current_timestamp(),
                    }
                } else {
                    WorkflowEvent::CommandSpawned {
                        execution_id: snapshot.execution_id.clone(),
                        node_execution_id: sibling.id.clone(),
                        display_command: format!("external-{attempt}"),
                        timestamp: current_timestamp(),
                    }
                }],
            )
            .unwrap();
            if attempt + 1 == attempts {
                drop(guard);
                break;
            }
            let mut next_guard = Box::pin(commit_lock.lock());
            assert!(futures_util::poll!(next_guard.as_mut()).is_pending());
            drop(guard);
            assert!(futures_util::poll!(operation.as_mut()).is_pending());
            guard = next_guard.await;
        }
        let result = operation.await;

        // Then
        match change {
            "sibling" => {
                let start = result.unwrap().unwrap();
                assert_ne!(start.node_execution_id(), target.id);
                let latest = fixture
                    .host
                    .load_execution(&fixture.app, &snapshot.execution_id)
                    .await
                    .unwrap();
                assert_eq!(
                    latest
                        .node_execution(start.node_execution_id())
                        .unwrap()
                        .attempt,
                    target.attempt + 1
                );
                assert!(fixture
                    .host
                    .restart_node_attempt(&fixture.app, &snapshot.execution_id, &target.id)
                    .await
                    .unwrap()
                    .is_none());
            }
            "abort" => assert!(result.unwrap().is_none()),
            _ => assert!(matches!(result, Err(WorkflowRuntimeError::Conflict(_)))),
        }
        let records =
            workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record.fact, NodeFact::RetryRequested))
                .count(),
            usize::from(change == "sibling")
        );
        assert!(!records
            .iter()
            .any(|record| matches!(record.fact, NodeFact::RuntimeFailureObserved(_))));
    }
}

#[tokio::test]
async fn test_自動再起動_先行nodeのエラーを後続の起動成功nodeへ記録しない() {
    use crate::domain::workflow::NodeFact;
    // Given
    let fixture = test_helpers::Fixture::new(0);
    let snapshot = fixture.persist_started(
        "  main: {fanout: {children: [work, other]}}\n  work: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n  other: {session: {provider: codex, facets: {instruction: policy-confirmation}}}",
        "/repo",
    ).await;
    let execution = fixture
        .host
        .load_execution(&fixture.app, &snapshot.execution_id)
        .await
        .unwrap();
    let target = execution
        .node_executions
        .iter()
        .find(|node| node.node_name == "work")
        .unwrap();
    let sibling = execution
        .node_executions
        .iter()
        .find(|node| node.node_name == "other")
        .unwrap();
    workflow_fact_log::append_facts_for_events(
        &fixture.store,
        &[WorkflowEvent::SessionAttached {
            execution_id: snapshot.execution_id.clone(),
            node_execution_id: target.id.clone(),
            session_id: "unavailable-session".into(),
            timestamp: current_timestamp(),
        }],
    )
    .unwrap();
    *fixture.sessions.presence_error_session.lock().unwrap() = Some("unavailable-session".into());

    // When
    fixture
        .host
        .schedule_startup_retries(
            &fixture.app,
            &snapshot.execution_id,
            "/repo",
            vec![target.id.clone(), sibling.id.clone()],
        )
        .await;
    fixture.wait_startup_retries().await;

    // Then
    let records =
        workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id).unwrap();
    let failures: Vec<_> = records
        .iter()
        .filter(|record| matches!(record.fact, NodeFact::RuntimeFailureObserved(_)))
        .collect();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].meta.node_execution_id, target.id);
    let latest = fixture
        .host
        .load_execution(&fixture.app, &snapshot.execution_id)
        .await
        .unwrap();
    let restarted = latest
        .node_executions
        .iter()
        .find(|node| node.node_name == "other" && node.attempt == sibling.attempt + 1)
        .unwrap();
    assert_eq!(
        *fixture.sessions.activated.lock().unwrap(),
        [restarted.id.clone()]
    );
    assert!(restarted.status.is_active());
    assert!(records
        .iter()
        .any(|record| record.meta.node_execution_id == restarted.id
            && matches!(record.fact, NodeFact::SessionAttached(_))));
    assert!(!records
        .iter()
        .any(|record| record.meta.node_execution_id == restarted.id
            && matches!(record.fact, NodeFact::RuntimeFailureObserved(_))));
}
