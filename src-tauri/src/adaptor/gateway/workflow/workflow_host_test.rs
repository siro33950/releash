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
    let mut executions = fixture.host.executions.lock().await;
    let mut archive = Box::pin(fixture.runtime.archive_execution_tree(&id, "manual"));
    assert!(futures_util::poll!(archive.as_mut()).is_pending());
    // When
    for kind in ["submit_received", "stop_received"] {
        workflow_fact_log::append_single_fact(
            &fixture.store,
            meta,
            &NodeFact::decode(kind, "{}").unwrap(),
            2000,
        )
        .unwrap();
    }
    executions.remove(&id);
    drop(executions);
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
        .any(|record| matches!(record.fact, NodeFact::AbortRequested)));
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
        let fact = NodeFact::Started(StartedFact {
            parent: None,
            root: Some(Box::new(TreeRootFact {
                repository_root: Some("/repo".into()),
                workspace_identity: "/missing/worktree".into(),
                worktree_path: "/missing/worktree".into(),
                created_from: ExecutionOrigin::Cli,
                request: String::new(),
                definition: crate::adaptor::gateway::workflow::mapper::schema_workflow_to_domain(
                    definition,
                )
                .unwrap(),
                definition_resolution: Default::default(),
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
        let mut recovery = Box::pin(fixture.host.reconcile_startup(&fixture.app));
        assert!(futures_util::poll!(recovery.as_mut()).is_pending());
        assert!(fixture.host.executions.lock().await.is_empty());
        assert!(fixture
            .host
            .execution_store
            .active_execution_snapshot(id)
            .await
            .is_none());

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
        fixture.host.reconcile_startup(&fixture.app).await.unwrap();

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
        assert!(fixture.host.executions.lock().await.is_empty());
        assert!(fixture
            .host
            .execution_store
            .active_execution_snapshot(id)
            .await
            .is_none());
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
async fn test_起動時recovery_登録済みの実行木のプロセス起動を待たない() {
    // Given
    let fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    let activation_gate = fixture.host.runtime_activation_gate(&id).await;
    let _activation_guard = activation_gate.lock.lock().await;
    let before = workflow_fact_log::read_tree_records(&fixture.store, &id).unwrap();
    // When
    let mut recovery = Box::pin(fixture.host.reconcile_startup(&fixture.app));
    assert!(matches!(
        futures_util::poll!(recovery.as_mut()),
        std::task::Poll::Ready(Ok(()))
    ));
    // Then
    assert_eq!(
        workflow_fact_log::read_tree_records(&fixture.store, &id).unwrap(),
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
        let execution_store = Arc::new(ExecutionStore::new_canonical(query.clone()));
        let host = Arc::new(WorkflowRuntimeHost::with_execution_store(
            Arc::new(UnusedWorkflowResolver),
            Arc::new(AcceptingWorktreeResolver),
            execution_store.clone(),
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
        let snapshot = host.get_state_by_execution_id(&execution_id).await.unwrap();

        // Then
        assert_eq!(snapshot.state, RuntimeExecutionState::Running);
        assert!(execution_store
            .active_execution_snapshot(&execution_id)
            .await
            .is_some());
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
        assert!(execution_store
            .active_execution_snapshot(&execution_id)
            .await
            .is_none());
        let record = execution_store
            .get_execution_record(&execution_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.status, status);
        let reloaded = ExecutionStore::new_canonical(query)
            .get_execution_record(&execution_id)
            .await
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
    assert!(fixture.host.executions.lock().await.is_empty());
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
                crate::domain::workflow::NodeFact::AbortRequested
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
            let state = fixture.host.get_state_by_execution_id(&id).await.unwrap();
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
                .filter(|record| matches!(record.fact, NodeFact::AbortRequested))
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
    root.definition.nodes[0].kind = NodeKind::Command(CommandSpec {
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
        &NodeFact::AbortRequested,
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
    corrupt.row.event_type = NodeFact::AbortRequested.event_type().into();
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
        .get_state_by_execution_id(&id)
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
