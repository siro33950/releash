use releash_lib::test_support::integration::fixtures::adaptor_gateway_workflow_workflow_host_EFFECT_AGENT_SESSION_ID as EFFECT_AGENT_SESSION_ID;
use releash_lib::test_support::integration::fixtures::adaptor_gateway_workflow_workflow_host_RecordingWorkflowAgentSessions as RecordingWorkflowAgentSessions;
use releash_lib::test_support::integration::sessions::AgentSessionRepository;
use releash_lib::test_support::integration::workflow::ApprovalCommand;
use releash_lib::test_support::integration::workflow::ExecutionTreeArchiveRepository;
use releash_lib::test_support::integration::workflow::RetryNodeCommand;

use crate::adaptor_gateway_workflow_workflow_host::workflow_host_tests::AcceptingWorktreeResolver;
use crate::adaptor_gateway_workflow_workflow_host::workflow_host_tests::UnusedWorkflowResolver;
use crate::adaptor_gateway_workflow_workflow_host_test_helpers::archive_fixture;
use crate::adaptor_gateway_workflow_workflow_host_test_helpers::archive_workflow;
use crate::adaptor_gateway_workflow_workflow_host_test_helpers::TestSessions;
use crate::adaptor_gateway_workflow_workflow_host_test_helpers::TestWorktrees;
use releash_lib::test_support::integration::persistence::archive_removed_execution_trees;
use releash_lib::test_support::integration::persistence::build_startup_gc_request;
use releash_lib::test_support::integration::persistence::LiveWorktreeResolution;
use releash_lib::test_support::integration::persistence::LiveWorktreeSet;
use releash_lib::test_support::integration::persistence::LocalEventStore;
use releash_lib::test_support::integration::persistence::LocalEventStoreConfig;
use releash_lib::test_support::integration::persistence::StdGcFileSystem;
use releash_lib::test_support::integration::persistence::StoreLayout;
use releash_lib::test_support::integration::process::CommandRunOutput;
use releash_lib::test_support::integration::process::CommandRunnerError;
use releash_lib::test_support::integration::providers::ProviderExecutionTreeStopCommand;
use releash_lib::test_support::integration::providers::ProviderKind;
use releash_lib::test_support::integration::sessions::AgentSession;
use releash_lib::test_support::integration::sessions::AgentSessionTreeLocation;
use releash_lib::test_support::integration::sessions::LocalAgentSessionRepository;
use releash_lib::test_support::integration::workflow::current_timestamp;
use releash_lib::test_support::integration::workflow::CommandExecutionInput;
use releash_lib::test_support::integration::workflow::ControlPlaneCommitCandidate;
use releash_lib::test_support::integration::workflow::ExecutionOrigin;
use releash_lib::test_support::integration::workflow::ExecutionStatus;
use releash_lib::test_support::integration::workflow::ExecutionTreeLaunch;
use releash_lib::test_support::integration::workflow::NodeExecutionFailureKind;
use releash_lib::test_support::integration::workflow::NodeExecutionStatus;
use releash_lib::test_support::integration::workflow::NodeFact;
use releash_lib::test_support::integration::workflow::NodeKindName;
use releash_lib::test_support::integration::workflow::NodeStart;
use releash_lib::test_support::integration::workflow::RuntimeActivationGate;
use releash_lib::test_support::integration::workflow::RuntimeCommitSnapshot;
use releash_lib::test_support::integration::workflow::RuntimeExecutionState;
use releash_lib::test_support::integration::workflow::SessionExecutionTreeRootFacts;
use releash_lib::test_support::integration::workflow::SubmitOutputCommand;
use releash_lib::test_support::integration::workflow::TransitionOutcome;
use releash_lib::test_support::integration::workflow::WorkflowControlPlaneUsecase;
use releash_lib::test_support::integration::workflow::WorkflowDefaults;
use releash_lib::test_support::integration::workflow::WorkflowDefinition;
use releash_lib::test_support::integration::workflow::WorkflowEvent;
use releash_lib::test_support::integration::workflow::WorkflowExecutionInsert;
use releash_lib::test_support::integration::workflow::WorkflowRuntimeCommandGateway;
use releash_lib::test_support::integration::workflow::WorkflowRuntimeError;
use releash_lib::test_support::integration::workflow::WorkflowRuntimeHost;
use releash_lib::test_support::integration::workspace::SqliteWorkspaceQueryService;
use releash_lib::test_support::integration::workspace::SqliteWorkspaceTreeRepository;
use releash_lib::test_support::integration::workspace::WorkspaceIdentity;
use std::sync::Arc;

#[tokio::test]
pub async fn test_実行木読取_load_execution_revisionで失敗分類を保持する() {
    use releash_lib::test_support::integration::persistence::ReadFailure;
    use releash_lib::test_support::integration::transport::classified_error;
    // Given
    let fixture = archive_fixture();
    for (failure, expected) in ReadFailure::cases() {
        fixture.store.fail_next_read(failure);
        // When
        let error = WorkflowRuntimeHost::load_execution_revision(&fixture.app, "execution")
            .await
            .unwrap_err();
        // Then
        assert!(matches!(&error, WorkflowRuntimeError::Store(_)));
        assert_eq!(classified_error(error).code, expected);
    }
}

#[tokio::test]
pub async fn test_実行木archive_状態確認後の自然完了で再登録できなくても終了状態を保って隠す() {
    use releash_lib::test_support::integration::workflow::ExecutionTreeArchiveRepository;
    use releash_lib::test_support::integration::workflow::NodeFact;
    // Given
    let fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    let records =
        releash_lib::test_support::integration::workflow::read_tree_records(&fixture.store, &id)
            .await
            .unwrap();
    let meta = &records[0].meta;
    let commit_lock = fixture.host.commit_lock(&id).await;
    let commit_guard = commit_lock.lock().await;
    let mut archive = Box::pin(fixture.runtime.archive_execution_tree(&id, "manual"));
    assert!(futures_util::poll!(archive.as_mut()).is_pending());
    // When
    for kind in ["submit_received", "stop_received"] {
        releash_lib::test_support::integration::workflow::append_single_fact(
            &fixture.store,
            meta,
            &releash_lib::test_support::integration::workflow::decode(kind, "{}").unwrap(),
            2000,
        )
        .await
        .unwrap();
    }
    drop(commit_guard);
    archive.await.unwrap();
    // Then
    assert_eq!(
        fixture.repository.target(&id).await.unwrap().status,
        ExecutionStatus::Completed
    );
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(std::slice::from_ref(&id))
            .await
            .unwrap()
            .records
            .len(),
        1
    );
    assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
    assert!(
        !releash_lib::test_support::integration::workflow::read_tree_records(&fixture.store, &id)
            .await
            .unwrap()
            .iter()
            .any(|record| matches!(record.fact, NodeFact::AbortRequested(_)))
    );
}

#[tokio::test]
pub async fn test_起動時recovery_gcのabortと直列化しarchive後に実行木もプロセスも復元しない() {
    use releash_lib::test_support::integration::persistence::ExecutionTreeGc;
    use releash_lib::test_support::integration::workflow::NodeFactMeta;
    use releash_lib::test_support::integration::workflow::StartedFact;
    use releash_lib::test_support::integration::workflow::TreeRootFact;

    for (launched_as, kind, node) in [
        (
            ExecutionTreeLaunch::Workflow,
            NodeKindName::Command,
            "command: 'must-not-start'",
        ),
        (
            ExecutionTreeLaunch::Session,
            NodeKindName::Session,
            "session: {provider: codex, facets: {instruction: policy-confirmation}}",
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
            releash_lib::test_support::integration::workflow::schema_workflow_to_domain(definition)
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
        releash_lib::test_support::integration::workflow::append_fact_batch_for_seed(
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
        let activation_guard = activation_gate.test_lock().lock().await;
        let mut recovery = Box::pin(
            crate::adaptor_gateway_workflow_workflow_host_test_helpers::reconcile_startup(
                &fixture.host,
                &fixture.app,
            ),
        );
        assert!(futures_util::poll!(recovery.as_mut()).is_pending());

        // When
        let mut archive = Box::pin(fixture.runtime.archive_removed_tree(id));
        assert!(futures_util::poll!(archive.as_mut()).is_pending());
        assert!(fixture
            .repository
            .archive_snapshot_for(&[id.into()])
            .await
            .unwrap()
            .records
            .is_empty());
        let mut operations = Box::pin(async { tokio::join!(recovery, archive) });
        crate::adaptor_gateway_workflow_workflow_host_test_helpers::poll_until_pending(
            operations.as_mut(),
            || Arc::strong_count(&activation_gate) >= 3,
        )
        .await;
        drop(activation_guard);
        let (recovered, archived) =
            tokio::time::timeout(std::time::Duration::from_secs(5), operations)
                .await
                .unwrap();
        recovered.unwrap();
        archived.unwrap();
        crate::adaptor_gateway_workflow_workflow_host_test_helpers::reconcile_startup(
            &fixture.host,
            &fixture.app,
        )
        .await
        .unwrap();

        // Then
        assert_eq!(
            fixture.repository.target(id).await.unwrap().status,
            ExecutionStatus::Aborted
        );
        assert_eq!(
            fixture
                .repository
                .archive_snapshot_for(&[id.into()])
                .await
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
            .test_active_commands()
            .lock()
            .unwrap()
            .is_empty());
        let facts =
            releash_lib::test_support::integration::workflow::read_tree_records(&fixture.store, id)
                .await
                .unwrap();
        assert!(!facts.iter().any(|record| matches!(
            record.fact,
            NodeFact::CommandSpawned(_) | NodeFact::SessionAttached(_)
        )));
    }
}

#[tokio::test]
pub async fn test_起動時recovery_起動済みの実行木のプロセスを再起動しない() {
    // Given
    let fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    let before =
        releash_lib::test_support::integration::workflow::read_tree_records(&fixture.store, &id)
            .await
            .unwrap();
    // When
    crate::adaptor_gateway_workflow_workflow_host_test_helpers::reconcile_startup(
        &fixture.host,
        &fixture.app,
    )
    .await
    .unwrap();
    // Then
    assert_eq!(
        releash_lib::test_support::integration::workflow::read_tree_records(&fixture.store, &id)
            .await
            .unwrap(),
        before
    );
}

#[tokio::test]
pub async fn test_起動時recovery_通常起動と同じsessionを一度だけ起動する() {
    // Given
    let fixture = archive_fixture();
    let startup = releash_lib::test_support::integration::platform::wire_workflow_startup(
        fixture.app.clone(),
        fixture.host.clone(),
    )
    .unwrap();
    let retrying = releash_lib::test_support::integration::platform::test_retrying();
    let mut gates = fixture.host.test_runtime_activation_locks().lock().await;
    let mut start = Box::pin(archive_workflow(&fixture));
    assert!(futures_util::poll!(start.as_mut()).is_pending());
    let id = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::select! {
            _ = start.as_mut() => panic!("start must wait for activation gate"),
            id = async {
                loop {
                    let ids = releash_lib::test_support::integration::workflow::list_tree_ids(
                        &releash_lib::test_support::integration::workflow::FactLogReadBackend::Live(fixture.store.clone()), None,
                    ).await.unwrap();
                    if let Some(id) = ids.into_iter().next() { break id; }
                    tokio::task::yield_now().await;
                }
            } => id,
        }
    })
    .await
    .unwrap();
    let gate = Arc::new(RuntimeActivationGate::new());
    let guard = gate.test_lock().lock().await;
    gates.insert(id.clone(), Arc::downgrade(&gate));
    drop(gates);
    crate::adaptor_gateway_workflow_workflow_host_test_helpers::poll_until_pending(
        start.as_mut(),
        || Arc::strong_count(&gate) > 1,
    )
    .await;
    let mut recovery = Box::pin(releash_lib::test_support::integration::platform::recover(
        &retrying, &startup,
    ));
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
    let records =
        releash_lib::test_support::integration::workflow::read_tree_records(&fixture.store, &id)
            .await
            .unwrap();
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
pub async fn test_起動時recovery_回復が先に起動したsessionへの古い起動要求を無視する() {
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
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
    let mut recovery = Box::pin(
        crate::adaptor_gateway_workflow_workflow_host_test_helpers::reconcile_startup(
            &fixture.host,
            &fixture.app,
        ),
    );
    assert!(futures_util::poll!(recovery.as_mut()).is_pending());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::select! {
            _ = fixture.sessions.preparation_entered.notified() => {},
            _ = recovery.as_mut() => panic!("recovery must wait for preparation"),
        }
    })
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
    let before = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        &snapshot.execution_id,
    )
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), start)
        .await
        .unwrap()
        .unwrap();

    // Then
    assert_eq!(fixture.sessions.prepared.lock().unwrap().len(), 1);
    assert_eq!(fixture.sessions.activated.lock().unwrap().len(), 1);
    assert_eq!(
        releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &snapshot.execution_id
        )
        .await
        .unwrap(),
        before
    );
}

#[tokio::test]
pub async fn test_workflow永続化_本番構成で起動から完了とabortまで状態ファイルを作らない() {
    for status in [ExecutionStatus::Completed, ExecutionStatus::Aborted] {
        // Given
        let directory = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().into(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ))
        .unwrap();
        let app = crate::adaptor_gateway_workflow_workflow_host_test_helpers::dependencies(Some(
            store.clone(),
        ));
        let query = SqliteWorkspaceQueryService::with_repository(
            SqliteWorkspaceTreeRepository::new(store.clone()),
        );
        let host = Arc::new(WorkflowRuntimeHost::with_runtime_ports(
            releash_lib::test_support::integration::platform::shared().clone(),
            Arc::new(UnusedWorkflowResolver),
            Arc::new(AcceptingWorktreeResolver),
            query.clone(),
            Arc::new(TestSessions::default()),
            Arc::new(TestWorktrees::default()),
            releash_lib::test_support::integration::daemon::serving(),
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
            let control = WorkflowControlPlaneUsecase::new(
                releash_lib::test_support::integration::platform::shared().clone(),
                Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(app, host)),
            );
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
        let record = releash_lib::test_support::integration::workspace::WorkspaceQueryService::execution_summary(
            query.as_ref(),
            &execution_id,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(record.status, status);
        let reloaded = releash_lib::test_support::integration::workspace::WorkspaceQueryService::execution_summary(
            query.as_ref(),
            &execution_id,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(reloaded, record);
        assert!(!directory.path().join("workflow_executions").exists());
    }
}

#[tokio::test]
pub async fn test_workflow定義の起動_session木のidをworkflow専用境界で拒否する() {
    // Given
    let fixture = archive_fixture();
    let source = serde_saphyr::from_str("name: archive\ndescription: test\nnodes:\n  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}").unwrap();
    let workflow =
        releash_lib::test_support::integration::workflow::schema_workflow_to_domain(source)
            .unwrap();
    let id = "agent-session-00000000000040008000000000000098";
    // When
    let result = fixture
        .host
        .insert_workflow_execution(WorkflowExecutionInsert::test_new(
            id.into(),
            workflow,
            "/missing/worktree".into(),
            None,
            ExecutionOrigin::Cli,
            WorkflowDefaults,
            1.0,
        ))
        .await;
    // Then
    assert!(matches!(
        result,
        Err(WorkflowRuntimeError::ValidationError(_))
    ));
    assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
}

#[tokio::test]
pub async fn test_実行木archive_gcはrepository_rootのない旧実行木も所属repo単位で判定する() {
    use releash_lib::test_support::integration::persistence::archive_removed_execution_trees;
    use releash_lib::test_support::integration::persistence::LiveWorktreeResolution;
    use releash_lib::test_support::integration::persistence::LiveWorktreeSet;
    use releash_lib::test_support::integration::workflow::SessionExecutionTreeRootFacts;
    // Given
    let fixture = archive_fixture();
    let id = "agent-session-00000000000040008000000000000099";
    let facts = SessionExecutionTreeRootFacts::new(
        id,
        "/repos/a-worktrees/feature",
        "/repos/a-worktrees/feature",
        releash_lib::test_support::integration::providers::ProviderKind::Codex,
        None,
    )
    .unwrap();
    releash_lib::test_support::integration::workflow::append_fact_batch_for_seed(
        &fixture.store,
        &facts.into_facts(),
        1,
        "legacy-root",
    )
    .unwrap();
    assert_eq!(
        fixture
            .repository
            .location(id)
            .await
            .unwrap()
            .repository_root,
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
        .await
        .unwrap()
        .records
        .is_empty());
    assert_eq!(
        fixture.repository.target(id).await.unwrap().status,
        ExecutionStatus::Running
    );
    archive_removed_execution_trees(Some(&resolution("/repos/b")), &fixture.runtime)
        .await
        .unwrap();
    assert_eq!(
        fixture.repository.target(id).await.unwrap().status,
        ExecutionStatus::Aborted
    );
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(&[id.into()])
            .await
            .unwrap()
            .records[0]
            .archive_reason,
        "worktree_removed"
    );
    assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
}

#[tokio::test]
pub async fn test_実行木archive_workflowをabortして停止完了後に隠し起動枠を解放する() {
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
        fixture.repository.target(&id).await.unwrap().status,
        ExecutionStatus::Aborted
    );
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(std::slice::from_ref(&id))
            .await
            .unwrap()
            .records[0]
            .archive_reason,
        "manual"
    );
    assert_eq!(fixture.visible_root_count("/missing/worktree").await, 0);
    let facts =
        releash_lib::test_support::integration::workflow::read_tree_records(&fixture.store, &id)
            .await
            .unwrap();
    let abort = facts
        .iter()
        .position(|record| {
            matches!(
                record.fact,
                releash_lib::test_support::integration::workflow::NodeFact::AbortRequested(_)
            )
        })
        .unwrap();
    let archive = facts
        .iter()
        .position(|record| {
            matches!(
                record.fact,
                releash_lib::test_support::integration::workflow::NodeFact::ArchiveRequested(_)
            )
        })
        .unwrap();
    assert!(abort < archive);
    let next = archive_workflow(&fixture).await;
    assert_ne!(id, next);
}

#[tokio::test]
pub async fn test_実行木archive_provider_idのない単独sessionも同じ操作を使う() {
    // Given
    let fixture = archive_fixture();
    let id = "agent-session-00000000000040008000000000000012";
    let facts = SessionExecutionTreeRootFacts::new(
        id,
        "/missing/worktree",
        "/missing/worktree",
        releash_lib::test_support::integration::providers::ProviderKind::Codex,
        None,
    )
    .unwrap();
    releash_lib::test_support::integration::workflow::append_fact_batch_for_seed(
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
    assert_eq!(fixture.visible_root_count("/missing/worktree").await, 1);
    // When
    fixture
        .runtime
        .archive_execution_tree(id, "manual")
        .await
        .unwrap();
    // Then
    assert_eq!(fixture.visible_root_count("/missing/worktree").await, 0);
    assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
    assert_eq!(
        fixture.repository.target(id).await.unwrap().status,
        ExecutionStatus::Aborted
    );
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(&[id.into()])
            .await
            .unwrap()
            .records
            .len(),
        1
    );
}

#[tokio::test]
pub async fn test_実行木archive_abort書込失敗ではarchiveを記録しない() {
    // Given
    let fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    fixture.store.close_write_queue_for_tests();
    // When
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(100),
        fixture.runtime.archive_execution_tree(&id, "manual")
    )
    .await
    .is_err());
    // Then
    assert_eq!(
        fixture.repository.target(&id).await.unwrap().status,
        ExecutionStatus::Running
    );
    assert!(fixture
        .repository
        .archive_snapshot_for(&[id])
        .await
        .unwrap()
        .records
        .is_empty());
}

#[tokio::test]
pub async fn test_実行木archive_旧記録移行はabort後に時刻と理由を保つ() {
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
        fixture.repository.target(&id).await.unwrap().status,
        ExecutionStatus::Aborted
    );
    let records = fixture
        .repository
        .archive_snapshot_for(&[id])
        .await
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
pub async fn test_実行木archive_gcはgit登録の消失だけで判定する() {
    use releash_lib::test_support::integration::persistence::LiveWorktree;

    // Given
    let fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    assert_eq!(
        fixture
            .repository
            .target(&id)
            .await
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
        fixture.repository.target(&id).await.unwrap().status,
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
        fixture.repository.target(&id).await.unwrap().status,
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
        fixture.repository.target(&id).await.unwrap().status,
        ExecutionStatus::Aborted
    );
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(&[id])
            .await
            .unwrap()
            .records[0]
            .archive_reason,
        "worktree_removed"
    );
    assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
}

#[tokio::test]
pub async fn test_実行木archive_停止失敗では隠さず再実行で完了する() {
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
        .archive_snapshot_for(std::slice::from_ref(&id))
        .await
        .unwrap()
        .records
        .is_empty());
    assert_eq!(
        fixture.repository.target(&id).await.unwrap().status,
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
            .await
            .unwrap()
            .records
            .len(),
        1
    );
}

#[tokio::test]
pub async fn test_実行木archive_移行失敗は旧記録を保持する() {
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
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(100),
        fixture
            .runtime
            .migrate_execution_archives(fixture.repository.as_ref())
    )
    .await
    .is_err());

    assert_eq!(std::fs::read_to_string(path).unwrap(), legacy);
}

#[tokio::test]
pub async fn test_実行木archive_終了済みは状態を保持しrestoreでも再開しない() {
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
        assert_eq!(fixture.repository.target(&id).await.unwrap().status, status);
        assert!(fixture
            .repository
            .archive_snapshot_for(std::slice::from_ref(&id))
            .await
            .unwrap()
            .records
            .is_empty());
        assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
        let facts = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &id,
        )
        .await
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
pub async fn test_実行木archive_command停止完了まではarchiveを記録しない() {
    use releash_lib::test_support::integration::workflow::CommandSpec;

    use releash_lib::test_support::integration::workflow::NodeKind;

    let fixture = archive_fixture();
    let id = "00000000-0000-4000-8000-000000000013";
    let mut facts = SessionExecutionTreeRootFacts::new(
        id,
        "/missing/worktree",
        "/missing/worktree",
        releash_lib::test_support::integration::providers::ProviderKind::Codex,
        None,
    )
    .unwrap();
    facts.meta.kind = NodeKindName::Command;
    let NodeFact::Started(started) = &mut facts.started else {
        unreachable!()
    };
    let root = started.root.as_mut().unwrap();
    root.launched_as =
        releash_lib::test_support::integration::workflow::ExecutionTreeLaunch::Workflow;
    root.definition.as_mut().unwrap().nodes[0].kind = NodeKind::Command(CommandSpec {
        command: "unused".into(),
        env: Default::default(),
    });
    releash_lib::test_support::integration::workflow::append_fact_batch_for_seed(
        &fixture.store,
        &[(facts.meta, facts.started)],
        1,
        "seed-command",
    )
    .unwrap();
    fixture
        .host
        .test_active_command_executions()
        .lock()
        .await
        .insert(id.into(), id.into());
    fixture
        .host
        .node_processes
        .test_active_commands()
        .lock()
        .unwrap()
        .insert(
            id.into(),
            releash_lib::test_support::integration::process::ActiveCommandHandle::for_test(),
        );
    let (stopped, completion) = tokio::sync::oneshot::channel::<()>();
    fixture
        .host
        .test_command_completion_observers()
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
        .test_active_commands()
        .lock()
        .unwrap()
        .is_empty());
    assert!(fixture
        .repository
        .archive_snapshot_for(&[id.into()])
        .await
        .unwrap()
        .records
        .is_empty());
    stopped.send(()).unwrap();
    archive.await.unwrap();
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(&[id.into()])
            .await
            .unwrap()
            .records[0]
            .archive_reason,
        "worktree_removed"
    );
}

#[tokio::test]
pub async fn test_実行木archive_旧sessionのarchive事実も終了状態へ移行する() {
    let fixture = archive_fixture();
    let id = "00000000-0000-4000-8000-000000000014";
    let facts = SessionExecutionTreeRootFacts::new(
        id,
        "/workspace",
        "/missing/worktree",
        releash_lib::test_support::integration::providers::ProviderKind::Codex,
        None,
    )
    .unwrap();
    let meta = facts.meta.clone();
    releash_lib::test_support::integration::workflow::append_fact_batch_for_seed(
        &fixture.store,
        &facts.into_facts(),
        1,
        "seed-old-session",
    )
    .unwrap();
    let mut pending = releash_lib::test_support::integration::workflow::pending_single_fact(
        &meta,
        &NodeFact::AbortRequested(Default::default()),
        42000,
    )
    .unwrap();
    pending.row.event_type = "archive_requested".into();
    pending.row.detail = "{}".into();
    releash_lib::test_support::integration::workflow::append_pending_rows(
        &fixture.store,
        vec![pending],
    )
    .await
    .unwrap();
    fixture
        .runtime
        .migrate_execution_archives(fixture.repository.as_ref())
        .await
        .unwrap();
    assert_eq!(
        fixture.repository.target(id).await.unwrap().status,
        ExecutionStatus::Aborted
    );
    assert_eq!(
        fixture
            .repository
            .target(id)
            .await
            .unwrap()
            .workspace_identity,
        "/workspace"
    );
    assert_eq!(
        fixture
            .repository
            .worktree_target_page("/workspace", None)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(&[id.into()])
            .await
            .unwrap()
            .records[0]
            .archived_at,
        42.0
    );
    assert!(fixture
        .repository
        .legacy_session_archive_page(None)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
pub async fn test_実行木archive_gcは単独sessionの所属repoだけの読取結果で判定する() {
    use releash_lib::test_support::integration::persistence::build_startup_gc_request;
    use releash_lib::test_support::integration::persistence::StdGcFileSystem;
    use releash_lib::test_support::integration::providers::ProviderKind;
    use releash_lib::test_support::integration::sessions::AgentSession;
    use releash_lib::test_support::integration::sessions::AgentSessionRepository;
    use releash_lib::test_support::integration::sessions::AgentSessionTreeLocation;
    use releash_lib::test_support::integration::sessions::LocalAgentSessionRepository;

    use releash_lib::test_support::integration::workspace::WorkspaceIdentity;

    for folder_remains in [true, false] {
        // Given
        let fixture = archive_fixture();
        let (repo_dir, repo) = crate::test_support_git::create_test_repo();
        crate::test_support_git::create_initial_commit(&repo);
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
                .await
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
            fixture.repository.target(id).await.unwrap().status,
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
            fixture.repository.target(id).await.unwrap().status,
            ExecutionStatus::Running
        );
        assert!(fixture
            .repository
            .archive_snapshot_for(&[id.into()])
            .await
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
            fixture.repository.target(id).await.unwrap().status,
            ExecutionStatus::Aborted
        );
        assert_eq!(
            fixture
                .repository
                .archive_snapshot_for(&[id.into()])
                .await
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
pub async fn test_実行木restore_同じarchive期間への並行要求は一度だけ記録する() {
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
        let records = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &id,
        )
        .await
        .unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(
                    record.fact,
                    releash_lib::test_support::integration::workflow::NodeFact::RestoreRequested
                ))
                .count(),
            period
        );
        assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
    }
}

#[tokio::test]
pub async fn test_archive移行_旧ファイルも対象も無い起動では無関係な破損履歴をfoldしない() {
    // Given
    let fixture = archive_fixture();
    let facts = SessionExecutionTreeRootFacts::new(
        "unrelated",
        "/repo",
        "/repo",
        releash_lib::test_support::integration::providers::ProviderKind::Codex,
        None,
    )
    .unwrap();
    let mut rows = facts
        .into_facts()
        .iter()
        .map(|(meta, fact)| {
            releash_lib::test_support::integration::workflow::pending_single_fact(meta, fact, 1)
                .unwrap()
        })
        .collect::<Vec<_>>();
    let mut corrupt = rows[1].clone();
    corrupt.row.event_type = releash_lib::test_support::integration::workflow::event_type(
        &NodeFact::AbortRequested(Default::default()),
    )
    .into();
    corrupt.row.detail = "broken history".into();
    rows.push(corrupt);
    releash_lib::test_support::integration::workflow::append_pending_rows(&fixture.store, rows)
        .await
        .unwrap();
    // When / Then
    fixture
        .runtime
        .migrate_execution_archives(fixture.repository.as_ref())
        .await
        .unwrap();
    assert!(
        releash_lib::test_support::integration::workflow::fold_tree_from(
            &releash_lib::test_support::integration::workflow::FactLogReadBackend::Live(
                fixture.store
            ),
            "unrelated"
        )
        .await
        .is_err()
    );
}

#[tokio::test]
pub async fn test_worktree削除中_外部変更を拒否して読み取りと内部archiveを許可する() {
    use releash_lib::test_support::integration::platform::WorktreeExecutionArchiver;
    use releash_lib::test_support::integration::workflow::AbortExecutionCommand;
    use releash_lib::test_support::integration::workflow::ApprovalCommand;
    use releash_lib::test_support::integration::workflow::ResumeSessionNodeCommand;
    use releash_lib::test_support::integration::workflow::RetryNodeCommand;
    use releash_lib::test_support::integration::workflow::StartExecutionCommand;
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
        releash_lib::test_support::integration::workflow::read_tree_records(&fixture.store, &id)
            .await
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
        releash_lib::test_support::integration::workflow::read_tree_records(&fixture.store, &id)
            .await
            .unwrap(),
        before
    );
    assert_ne!(fixture.visible_root_count("/missing/worktree").await, 0);
    fixture
        .runtime
        .archive_worktree("/missing/worktree")
        .await
        .unwrap();
    assert_eq!(
        fixture.repository.target(&id).await.unwrap().status,
        ExecutionStatus::Aborted
    );
    assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
    assert!(
        releash_lib::test_support::integration::workflow::read_tree_records(&fixture.store, &id)
            .await
            .unwrap()
            .iter()
            .any(|fact| matches!(fact.fact, NodeFact::ArchiveRequested(_)))
    );
}

#[tokio::test]
pub async fn test_旧実行木gc_任意位置のlinked_worktreeを対象repoの読取結果だけで判定する() {
    assert_legacy_linked_worktree_gc(false, false).await;
}

#[tokio::test]
pub async fn test_旧実行木gc_任意位置のlinked_worktreeの所属をフォルダ消失後も保持する() {
    assert_legacy_linked_worktree_gc(true, false).await;
}

#[tokio::test]
pub async fn test_旧実行木gc_初回gc前にフォルダが消えていてもgit登録から所属を保持する() {
    assert_legacy_linked_worktree_gc(true, true).await;
}

async fn assert_legacy_linked_worktree_gc(remove_directory: bool, remove_before_gc: bool) {
    // Given
    let fixture = archive_fixture();
    let (repo_dir, repo) = crate::test_support_git::create_test_repo();
    crate::test_support_git::create_initial_commit(&repo);
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
        releash_lib::test_support::integration::providers::ProviderKind::Codex,
        None,
    )
    .unwrap();
    releash_lib::test_support::integration::workflow::append_fact_batch_for_seed(
        &fixture.store,
        &facts.into_facts(),
        1,
        id,
    )
    .unwrap();
    assert!(fixture
        .repository
        .target(id)
        .await
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
        .await
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
        releash_lib::test_support::integration::workflow::ExecutionTreeArchiveFactRepository::from_backend(
            releash_lib::test_support::integration::workflow::FactLogReadBackend::ReadOnly(
                releash_lib::test_support::integration::persistence::LocalEventReadStore::open(
                    fixture.directory.path(),
                    std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
                )
                .unwrap(),
            ),
        );
    assert_eq!(
        reopened.candidate_page(None).await.unwrap()[0]
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
        fixture.repository.target(id).await.unwrap().status,
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
        fixture.repository.target(id).await.unwrap().status,
        ExecutionStatus::Aborted
    );
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(&[id.into()])
            .await
            .unwrap()
            .records[0]
            .archive_reason,
        "worktree_removed"
    );
    assert_eq!(path.exists(), !remove_directory);
}

#[tokio::test]
pub async fn test_記録からの操作_完了済み単独sessionは再開後もsubmitを拒否しstopを記録する() {
    use releash_lib::test_support::integration::sessions::AgentSessionRecoveryResult;

    use releash_lib::test_support::integration::workflow::WorkflowError;

    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
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
    let control = WorkflowControlPlaneUsecase::new(
        releash_lib::test_support::integration::platform::shared().clone(),
        Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
            fixture.app.clone(),
            Arc::new(fixture.restarted_host()),
        )),
    );
    // When
    let submit_error = control
        .submit_output(SubmitOutputCommand {
            node_execution_id: id.into(),
            artifact: None,
        })
        .await
        .unwrap_err();
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
    assert!(matches!(submit_error, WorkflowError::InvalidState(_)));
    let records =
        releash_lib::test_support::integration::workflow::read_tree_records(&fixture.store, id)
            .await
            .unwrap();
    assert!(records.iter().any(|record| matches!(
        record.fact,
        releash_lib::test_support::integration::workflow::NodeFact::ResumeRequested
    )));
    assert!(!records.iter().any(|record| matches!(
        record.fact,
        releash_lib::test_support::integration::workflow::NodeFact::SubmitReceived(_)
    )));
    assert!(records.iter().any(|record| matches!(
        record.fact,
        releash_lib::test_support::integration::workflow::NodeFact::StopReceived(_)
    )));
    let folded = releash_lib::test_support::integration::workflow::fold_tree_from(
        &releash_lib::test_support::integration::workflow::FactLogReadBackend::Live(
            fixture.store.clone(),
        ),
        id,
    )
    .await
    .unwrap()
    .unwrap();
    let node = folded
        .aggregate
        .node_executions
        .iter()
        .find(|node| node.id == id)
        .unwrap();
    assert_eq!(node.status, NodeExecutionStatus::Succeeded);
    assert_eq!(
        node.completion_signals,
        releash_lib::test_support::integration::workflow::NodeCompletionSignalState::Pending
    );
}

#[tokio::test]
pub async fn test_起動時前進_恒久失敗でもabortせず他の木を進める() {
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
    let failed = fixture.persist_started("  main: {session: {provider: codex, facets: {instruction: missing-startup-facet-1840}}}\n", "/failed").await;
    let healthy = fixture
        .persist_started(
            "  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n",
            "/healthy",
        )
        .await;
    let host = Arc::new(fixture.restarted_host());
    let startup = releash_lib::test_support::integration::platform::wire_workflow_startup(
        fixture.app.clone(),
        host.clone(),
    )
    .unwrap();
    // When
    assert!(releash_lib::test_support::integration::platform::recover(
        &releash_lib::test_support::integration::platform::test_retrying(),
        &startup,
    )
    .await
    .is_err());
    let failed_records = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        &failed.execution_id,
    )
    .await
    .unwrap();
    let healthy_records = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        &healthy.execution_id,
    )
    .await
    .unwrap();
    // Then
    assert!(!failed_records.iter().any(|record| matches!(&record.fact,
        releash_lib::test_support::integration::workflow::NodeFact::AbortRequested(fact) if fact.reason.as_ref().is_some_and(|reason| reason.contains("missing-startup-facet-1840")))));
    assert_eq!(
        host.load_execution(&fixture.app, &failed.execution_id)
            .await
            .unwrap()
            .state(),
        &RuntimeExecutionState::Running
    );
    assert!(healthy_records.iter().any(|record| matches!(
        record.fact,
        releash_lib::test_support::integration::workflow::NodeFact::SessionAttached(_)
    )));
    assert_eq!(fixture.sessions.activated.lock().unwrap().len(), 1);
    assert_eq!(
        releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &failed.execution_id
        )
        .await
        .unwrap(),
        failed_records
    );
    assert_eq!(
        releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &healthy.execution_id
        )
        .await
        .unwrap(),
        healthy_records
    );
    assert!(host.test_startup_retries().lock().await.is_empty());
}

#[tokio::test]
pub async fn test_起動時前進_reply喪失後は保存済みなら続行し未保存でもabortしない() {
    use releash_lib::test_support::integration::persistence::StoreLayout;

    for persisted in [true, false] {
        // Given
        let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
        let interrupted = fixture
            .persist_started(
                "  main: {sequence: {children: [first, nested]}}\n  nested: {sequence: {children: [next]}}\n  first: {session: {provider: codex}}\n  next: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n",
                "/interrupted",
            )
            .await;
        let records = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &interrupted.execution_id,
        )
        .await
        .unwrap();
        let first = &records
            .iter()
            .find(|record| record.meta.node_name == "first")
            .unwrap()
            .meta;
        for fact in [
            NodeFact::SubmitReceived(
                releash_lib::test_support::integration::workflow::SubmitReceivedFact {
                    request_id: None,
                },
            ),
            NodeFact::StopReceived(
                releash_lib::test_support::integration::workflow::StopReceivedFact {
                    result_summary: None,
                    token_usage: None,
                },
            ),
        ] {
            releash_lib::test_support::integration::workflow::append_single_fact(
                &fixture.store,
                first,
                &fact,
                2000,
            )
            .await
            .unwrap();
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
        let startup = releash_lib::test_support::integration::platform::wire_workflow_startup(
            fixture.app.clone(),
            host.clone(),
        )
        .unwrap();
        fixture.store.fault_injector().arm_drop_reply();

        // When
        let result = releash_lib::test_support::integration::platform::recover(
            &releash_lib::test_support::integration::platform::test_retrying(),
            &startup,
        )
        .await;
        let records = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &interrupted.execution_id,
        )
        .await
        .unwrap();
        let healthy_records = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &healthy.execution_id,
        )
        .await
        .unwrap();

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
            0
        );
        assert_eq!(
            host.load_execution(&fixture.app, &interrupted.execution_id)
                .await
                .unwrap()
                .state(),
            &RuntimeExecutionState::Running
        );
        assert!(healthy_records
            .iter()
            .any(|record| matches!(record.fact, NodeFact::SessionAttached(_))));
        assert_eq!(
            fixture.sessions.activated.lock().unwrap().len(),
            1 + usize::from(persisted)
        );
        assert_eq!(
            releash_lib::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                &interrupted.execution_id
            )
            .await
            .unwrap(),
            records
        );
        assert_eq!(
            releash_lib::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                &healthy.execution_id
            )
            .await
            .unwrap(),
            healthy_records
        );
        assert!(host.test_startup_retries().lock().await.is_empty());
    }
}

#[tokio::test]
pub async fn test_起動時session紐付け_reply喪失後は保存済みなら起動し未保存でもabortしない() {
    for persisted in [true, false] {
        // Given
        let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
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
        let startup = releash_lib::test_support::integration::platform::wire_workflow_startup(
            fixture.app.clone(),
            host.clone(),
        )
        .unwrap();
        fixture.store.fault_injector().arm_drop_reply();

        // When
        let result = releash_lib::test_support::integration::platform::recover(
            &releash_lib::test_support::integration::platform::test_retrying(),
            &startup,
        )
        .await;
        let records = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &interrupted.execution_id,
        )
        .await
        .unwrap();
        let healthy_records = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &healthy.execution_id,
        )
        .await
        .unwrap();

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
        assert!(aborts.is_empty());
        assert!(host
            .load_execution(&fixture.app, &interrupted.execution_id)
            .await
            .unwrap()
            .is_active());
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
            releash_lib::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                &interrupted.execution_id
            )
            .await
            .unwrap(),
            records
        );
        assert_eq!(
            releash_lib::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                &healthy.execution_id
            )
            .await
            .unwrap(),
            healthy_records
        );
        assert!(host.test_startup_retries().lock().await.is_empty());
    }
}

#[tokio::test]
pub async fn test_worktree排他_再起動直後の記録を使い占有実行をエラーに含む() {
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
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
pub async fn test_worktree排他_同時起動は一件だけ成功し外部abort後は起動できる() {
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
    let workflow: WorkflowDefinition = serde_saphyr::from_str("name: concurrent\ndescription: test\nnodes:\n  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n").unwrap();
    let facet_guard = fixture.host.test_execution_facet_contents().lock().await;
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
    releash_lib::test_support::integration::workflow::append_facts_for_events(
        &fixture.store,
        &[WorkflowEvent::ExecutionAborted {
            execution_id: id,
            aborted_node: None,
            timestamp: current_timestamp(),
        }],
    )
    .await
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
pub async fn test_worktree排他_abort待機中も別worktreeは起動し同一worktreeだけ待つ() {
    for (abort_succeeds, before_commit) in [(true, true), (false, true), (true, false)] {
        // Given
        let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
        let active = fixture
            .persist_started("  main: {session: {provider: codex}}\n", "/repo")
            .await;
        let workflow: WorkflowDefinition = serde_saphyr::from_str("name: concurrent\ndescription: test\nnodes:\n  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n").unwrap();
        let gate = fixture
            .host
            .runtime_activation_gate(&active.execution_id)
            .await;
        let activation_guard = if before_commit {
            Some(gate.test_lock().lock().await)
        } else {
            None
        };
        let shutdown_guard = if before_commit {
            None
        } else {
            Some(fixture.host.test_active_command_executions().lock().await)
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
        if before_commit {
            crate::adaptor_gateway_workflow_workflow_host_test_helpers::poll_until_pending(
                abort.as_mut(),
                || Arc::strong_count(&gate) > 1,
            )
            .await;
        } else {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                tokio::select! {
                    _ = abort.as_mut() => panic!("abort must wait for shutdown"),
                    _ = async {
                        while fixture.host.load_execution(&fixture.app, &active.execution_id).await.unwrap().is_active() {
                            tokio::task::yield_now().await;
                        }
                    } => {},
                }
            }).await.unwrap();
        }

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
            !releash_lib::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                &other_id
            )
            .await
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
pub async fn test_commit排他_同一実行木で共有し不要なlockを保持し続けない() {
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
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
    let locks = fixture.host.test_commit_locks().lock().await;
    assert_eq!(locks.len(), 1);
    assert!(locks.contains_key("tree-b"));
}

#[tokio::test]
pub async fn test_commit排他_別実行木のcommitとcommand起動判定を妨げない() {
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
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
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), other.as_mut())
            .await
            .unwrap()
            .unwrap()
    );
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
    tokio::time::timeout(std::time::Duration::from_secs(5), spawn.as_mut())
        .await
        .unwrap()
        .unwrap();
    assert!(fixture
        .host
        .node_processes
        .test_active_commands()
        .lock()
        .unwrap()
        .is_empty());
    assert!(futures_util::poll!(same.as_mut()).is_pending());
    drop(guard);
    assert!(same.await.unwrap());
}

#[tokio::test]
pub async fn test_worktree排他_待機者と共有し不要なlockを保持し続けない() {
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
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
    let locks = fixture.host.test_workflow_start_locks().lock().await;
    assert_eq!(locks.len(), 1);
    assert!(locks.contains_key("/other"));
}

#[tokio::test]
pub async fn test_worktree排他_abort対象の所在地を読めなければ記録を変更せずエラーにする() {
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
    let active = fixture
        .persist_started("  main: {session: {provider: codex}}\n", "/repo")
        .await;
    let records = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        &active.execution_id,
    )
    .await
    .unwrap();
    let mut broken = releash_lib::test_support::integration::workflow::pending_single_fact(
        &records[0].meta,
        &records[0].fact,
        1_000,
    )
    .unwrap();
    broken.row.tree_id = "broken".into();
    broken.row.detail = "{".into();
    fixture
        .store
        .append_node_event(broken.row, Some(1_000))
        .await
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
        releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &active.execution_id
        )
        .await
        .unwrap(),
        records
    );
    assert!(fixture
        .host
        .test_workflow_start_locks()
        .lock()
        .await
        .is_empty());
}

fn command_input(snapshot: &RuntimeCommitSnapshot) -> CommandExecutionInput {
    let node = &snapshot.node_executions[0];
    CommandExecutionInput::test_new(
        (
            snapshot.execution_id.clone(),
            node.id.clone(),
            node.node_name.clone(),
            node.attempt,
        ),
        snapshot.worktree_path.clone(),
        Some("true".into()),
        Vec::new(),
        None,
        Default::default(),
        None,
    )
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
pub async fn test_command反映_登録なしの起動と結果を保存し確定後は理由をログに残す() {
    // Given
    releash_lib::test_support::integration::platform::install_capturing_logger();
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
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
    let completed = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        &snapshot.execution_id,
    )
    .await
    .unwrap();
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
        releash_lib::test_support::integration::workflow::NodeFact::CommandSpawned(_)
    )));
    assert!(completed.iter().any(|record| matches!(
        record.fact,
        releash_lib::test_support::integration::workflow::NodeFact::ExecutionCompleted
    )));
    assert_eq!(
        releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &snapshot.execution_id
        )
        .await
        .unwrap(),
        completed
    );
    let warnings = releash_lib::test_support::integration::platform::captured_warning_messages();
    assert_eq!(
        warnings
            .iter()
            .filter(|message| message.contains(&input.test_node_execution_id())
                && message.contains("was not applied: execution tree is terminal"))
            .count(),
        3
    );
}

#[tokio::test]
pub async fn test_command失敗_登録なしの最新attemptに保存し別attemptは反映しない() {
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
    let snapshot = fixture
        .persist_started("  main: {command: true}\n", "/repo")
        .await;
    let host = fixture.restarted_host();
    let input = command_input(&snapshot);
    // When
    host.fail_current_command_node(&fixture.app, &input, "process wait failed".into())
        .await
        .unwrap();
    let before = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        &snapshot.execution_id,
    )
    .await
    .unwrap();
    let mut stale = input.clone();
    *stale.test_attempt_mut() += 1;
    host.fail_current_command_node(&fixture.app, &stale, "wrong attempt".into())
        .await
        .unwrap();
    // Then
    assert!(before.iter().any(|record| matches!(
        record.fact,
        releash_lib::test_support::integration::workflow::NodeFact::RuntimeFailureObserved(_)
    )));
    assert_eq!(
        releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &snapshot.execution_id
        )
        .await
        .unwrap(),
        before
    );
}

#[tokio::test]
pub async fn test_commit結果不明_一部や別内容は競合とし保存済みbatchは再追記しない() {
    for change in ["partial", "session", "timestamp", "complete"] {
        // Given
        let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
        let snapshot = fixture
            .persist_started("  main: {session: {provider: codex}}\n", "/repo")
            .await;
        let records = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &snapshot.execution_id,
        )
        .await
        .unwrap();
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
        releash_lib::test_support::integration::workflow::append_facts_for_events(
            &fixture.store,
            &concurrent,
        )
        .await
        .unwrap();
        let before = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &snapshot.execution_id,
        )
        .await
        .unwrap();
        fixture.store.fault_injector().arm_drop_reply();

        // When
        let result = WorkflowRuntimeHost::append_events_at_head(
            &fixture.app,
            &snapshot.execution_id,
            head,
            &events,
        )
        .await;

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
            releash_lib::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                &snapshot.execution_id
            )
            .await
            .unwrap(),
            before
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
pub async fn test_commit結果不明_記録を読み直せなければ保存成功にしない() {
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
    let snapshot = fixture
        .persist_started("  main: {session: {provider: codex}}\n", "/repo")
        .await;
    let records = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        &snapshot.execution_id,
    )
    .await
    .unwrap();
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
    let commit = tokio::spawn(async move {
        WorkflowRuntimeHost::append_events_at_head(&app, &snapshot.execution_id, head, &[event])
            .await
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
    let error = commit.await.unwrap().unwrap_err();

    // Then
    assert!(matches!(
        error,
        WorkflowRuntimeError::Store(failure) if failure.to_string().contains("control-plane commit readback failed") && releash_lib::test_support::integration::transport::ConnectFailure::connect_code(&failure) == connectrpc::ErrorCode::Internal
    ));
}

#[tokio::test]
pub async fn test_commit競合_候補作成後に外部が保存した事実を上書きしない() {
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
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
    let records = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        &snapshot.execution_id,
    )
    .await
    .unwrap();
    releash_lib::test_support::integration::workflow::append_facts_for_events(
        &fixture.store,
        &[WorkflowEvent::NodeStopReceived {
            execution_id: snapshot.execution_id.clone(),
            node_execution_id: records[0].meta.node_execution_id.clone(),
            timestamp: current_timestamp() + 0.001,
        }],
    )
    .await
    .unwrap();
    let advanced = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        &snapshot.execution_id,
    )
    .await
    .unwrap();
    // When
    let error = fixture
        .host
        .commit_control_plane_candidate(
            &fixture.app,
            ControlPlaneCommitCandidate::test_new(
                &snapshot.execution_id,
                before,
                candidate,
                TransitionOutcome::Applied,
                &[WorkflowEvent::ExecutionAborted {
                    execution_id: snapshot.execution_id.clone(),
                    aborted_node: None,
                    timestamp: current_timestamp(),
                }],
                Vec::new(),
            ),
        )
        .await
        .unwrap_err();
    // Then
    assert!(matches!(error, WorkflowRuntimeError::Conflict(_)));
    assert_eq!(
        releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &snapshot.execution_id
        )
        .await
        .unwrap(),
        advanced
    );
}

#[tokio::test]
pub async fn test_command反映_実行木がない場合は起動と結果と失敗の不反映理由を残す() {
    // Given
    releash_lib::test_support::integration::platform::install_capturing_logger();
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
    let snapshot = fixture
        .persist_started("  main: {command: true}\n", "/repo")
        .await;
    let mut input = command_input(&snapshot);
    *input.test_execution_id_mut() = format!("missing-{}", snapshot.execution_id);
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
    let warnings = releash_lib::test_support::integration::platform::captured_warning_messages();
    assert_eq!(
        warnings
            .iter()
            .filter(|message| message.contains(&input.test_execution_id())
                && message.contains("was not applied: execution tree was not found"))
            .count(),
        3
    );
    assert!(
        releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &input.test_execution_id()
        )
        .await
        .unwrap()
        .is_empty()
    );
}

#[tokio::test]
pub async fn test_記録からの承認_外部writerの完了信号で承認待ちになったnodeを承認する() {
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
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
    releash_lib::test_support::integration::workflow::append_facts_for_events(
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
    .await
    .unwrap();
    let control = WorkflowControlPlaneUsecase::new(
        releash_lib::test_support::integration::platform::shared().clone(),
        Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
            fixture.app.clone(),
            Arc::new(fixture.host.clone()),
        )),
    );
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
    let records = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        &before.id,
    )
    .await
    .unwrap();
    assert!(records
        .iter()
        .any(|record| matches!(record.fact, NodeFact::ApprovalGranted(_))));
    assert!(records
        .iter()
        .any(|record| matches!(record.fact, NodeFact::ExecutionCompleted)));
}

#[tokio::test]
pub async fn test_記録からのretry_外部writerが作った最新attemptを再試行する() {
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
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
    releash_lib::test_support::integration::workflow::append_facts_for_events(
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
    .await
    .unwrap();
    let control = WorkflowControlPlaneUsecase::new(
        releash_lib::test_support::integration::platform::shared().clone(),
        Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
            fixture.app.clone(),
            Arc::new(fixture.host.clone()),
        )),
    );
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
    let records = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        &before.id,
    )
    .await
    .unwrap();
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
        .test_active_commands()
        .lock()
        .unwrap()
        .is_empty());
}

#[tokio::test]
pub async fn test_abort競合_外部writerの追記後も最新記録を中止する() {
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
    let snapshot = fixture
        .persist_started("  main: {command: true}\n", "/repo")
        .await;
    let meta = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        &snapshot.execution_id,
    )
    .await
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
    crate::adaptor_gateway_workflow_workflow_host_test_helpers::poll_until_pending(
        abort.as_mut(),
        || Arc::strong_count(&commit_lock) > 1,
    )
    .await;
    // When
    releash_lib::test_support::integration::workflow::append_single_fact(
        &fixture.store,
        &meta,
        &NodeFact::CommandSpawned(
            releash_lib::test_support::integration::workflow::CommandSpawnedFact {
                display_command: "external".into(),
            },
        ),
        2000,
    )
    .await
    .unwrap();
    drop(guard);
    abort.await.unwrap();
    // Then
    let records = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        &snapshot.execution_id,
    )
    .await
    .unwrap();
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
pub async fn test_command結果競合_最新記録で成功を保存し終端なら理由付きで反映しない() {
    for abort in [false, true] {
        // Given
        releash_lib::test_support::integration::platform::install_capturing_logger();
        let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
        let snapshot = fixture
            .persist_started("  main: {command: true}\n", "/repo")
            .await;
        let input = command_input(&snapshot);
        let meta = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &snapshot.execution_id,
        )
        .await
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
        crate::adaptor_gateway_workflow_workflow_host_test_helpers::poll_until_pending(
            completion.as_mut(),
            || Arc::strong_count(&commit_lock) > 1,
        )
        .await;
        // When
        releash_lib::test_support::integration::workflow::append_single_fact(
            &fixture.store,
            &meta,
            &if abort {
                NodeFact::AbortRequested(Default::default())
            } else {
                NodeFact::CommandSpawned(
                    releash_lib::test_support::integration::workflow::CommandSpawnedFact {
                        display_command: "external".into(),
                    },
                )
            },
            2000,
        )
        .await
        .unwrap();
        let before = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &snapshot.execution_id,
        )
        .await
        .unwrap();
        drop(guard);
        completion.await;
        // Then
        let records = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &snapshot.execution_id,
        )
        .await
        .unwrap();
        assert!(!records
            .iter()
            .any(|record| matches!(record.fact, NodeFact::RuntimeFailureObserved(_))));
        if abort {
            assert_eq!(records, before);
            assert!(
                releash_lib::test_support::integration::platform::captured_warning_messages()
                    .iter()
                    .any(|message| message.contains(&input.test_node_execution_id())
                        && message.contains("was not applied: execution tree is terminal"))
            );
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
pub async fn test_操作競合_abortとcommand結果は4回を超えて収束し障害事実を追加しない() {
    const CONFLICT_COUNT: usize = 5;
    for command in [false, true] {
        // Given
        releash_lib::test_support::integration::platform::install_capturing_logger();
        let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
        let snapshot = fixture
            .persist_started("  main: {command: true}\n", "/repo")
            .await;
        let input = command_input(&snapshot);
        let meta = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &snapshot.execution_id,
        )
        .await
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
        for attempt in 0..CONFLICT_COUNT {
            crate::adaptor_gateway_workflow_workflow_host_test_helpers::poll_until_pending(
                operation.as_mut(),
                || Arc::strong_count(&commit_lock) > 1,
            )
            .await;
            releash_lib::test_support::integration::workflow::append_single_fact(
                &fixture.store,
                &meta,
                &NodeFact::CommandSpawned(
                    releash_lib::test_support::integration::workflow::CommandSpawnedFact {
                        display_command: format!("external-{attempt}"),
                    },
                ),
                2000 + attempt as i64 * 1000,
            )
            .await
            .unwrap();
            if attempt + 1 == CONFLICT_COUNT {
                drop(guard);
                break;
            }
            let mut next_guard = Box::pin(commit_lock.lock());
            assert!(futures_util::poll!(next_guard.as_mut()).is_pending());
            drop(guard);
            guard = tokio::select! {
                guard = next_guard => guard,
                _ = operation.as_mut() => panic!("operation must retry after conflict"),
            };
        }
        let result = operation.await;
        // Then
        result.unwrap();
        let records = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &snapshot.execution_id,
        )
        .await
        .unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record.fact, NodeFact::CommandSpawned(_)))
                .count(),
            CONFLICT_COUNT
        );
        assert!(!records
            .iter()
            .any(|record| matches!(record.fact, NodeFact::RuntimeFailureObserved(_))));
        assert!(!fixture
            .host
            .load_execution(&fixture.app, &snapshot.execution_id)
            .await
            .unwrap()
            .is_active());
    }
}

#[tokio::test]
pub async fn test_command起動失敗競合_最新attemptへ保存し終端や更新済みattemptは理由付きで反映しない(
) {
    for spawned in [true, false] {
        for change in ["current", "abort", "retry"] {
            // Given
            releash_lib::test_support::integration::platform::install_capturing_logger();
            let fixture =
                crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
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
            crate::adaptor_gateway_workflow_workflow_host_test_helpers::poll_until_pending(
                operation.as_mut(),
                || Arc::strong_count(&commit_lock) > 1,
            )
            .await;
            // When
            let timestamp = current_timestamp();
            let events = match change {
                "abort" => vec![WorkflowEvent::ExecutionAborted {
                    execution_id: input.test_execution_id().clone(),
                    aborted_node: None,
                    timestamp,
                }],
                "retry" => vec![
                    WorkflowEvent::NodeRetryRequested {
                        execution_id: input.test_execution_id().clone(),
                        node_execution_id: input.test_node_execution_id().clone(),
                        timestamp,
                    },
                    WorkflowEvent::NodeStarted {
                        execution_id: input.test_execution_id().clone(),
                        node_execution_id: uuid::Uuid::new_v4().to_string(),
                        node_name: input.test_node_name().clone(),
                        kind: NodeKindName::Command,
                        attempt: input.test_attempt() + 1,
                        parent: None,
                        worktree: None,
                        timestamp,
                    },
                ],
                _ => vec![WorkflowEvent::CommandSpawned {
                    execution_id: input.test_execution_id().clone(),
                    node_execution_id: input.test_node_execution_id().clone(),
                    display_command: "external".into(),
                    timestamp,
                }],
            };
            releash_lib::test_support::integration::workflow::append_facts_for_events(
                &fixture.store,
                &events,
            )
            .await
            .unwrap();
            let before = releash_lib::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                &input.test_execution_id(),
            )
            .await
            .unwrap();
            drop(guard);
            let applied = operation.await.unwrap();
            // Then
            let records = releash_lib::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                &input.test_execution_id(),
            )
            .await
            .unwrap();
            if change == "current" {
                assert!(applied);
                assert_eq!(records.len(), before.len() + 1);
                assert_eq!(&records[..before.len()], before);
                let record = records.last().unwrap();
                assert_eq!(
                    record.meta.node_execution_id,
                    input.test_node_execution_id()
                );
                assert_eq!(record.meta.attempt, input.test_attempt());
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
                assert!(
                    releash_lib::test_support::integration::platform::captured_warning_messages()
                        .iter()
                        .any(|message| message.contains(&input.test_node_execution_id())
                            && message.contains("was not applied")
                            && message.contains(reason))
                );
            }
        }
    }
}

#[tokio::test]
pub async fn test_command起動失敗競合_上限で不反映理由を残し競合を障害事実にしない() {
    const CONFLICT_COUNT: usize = 5;
    for spawned in [true, false] {
        // Given
        releash_lib::test_support::integration::platform::install_capturing_logger();
        let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
        let snapshot = fixture
            .persist_started("  main: {command: true}\n", "/repo")
            .await;
        let input = command_input(&snapshot);
        let commit_lock = fixture.host.commit_lock(&snapshot.execution_id).await;
        let mut guard = commit_lock.lock().await;
        let mut operation = Box::pin(async {
            if spawned {
                assert!(fixture
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
        for attempt in 0..CONFLICT_COUNT {
            crate::adaptor_gateway_workflow_workflow_host_test_helpers::poll_until_pending(
                operation.as_mut(),
                || Arc::strong_count(&commit_lock) > 1,
            )
            .await;
            releash_lib::test_support::integration::workflow::append_facts_for_events(
                &fixture.store,
                &[WorkflowEvent::CommandSpawned {
                    execution_id: input.test_execution_id().clone(),
                    node_execution_id: input.test_node_execution_id().clone(),
                    display_command: format!("external-{attempt}"),
                    timestamp: current_timestamp(),
                }],
            )
            .await
            .unwrap();
            if attempt + 1 == CONFLICT_COUNT {
                drop(guard);
                break;
            }
            let mut next_guard = Box::pin(commit_lock.lock());
            assert!(futures_util::poll!(next_guard.as_mut()).is_pending());
            drop(guard);
            guard = tokio::select! {
                guard = next_guard => guard,
                _ = operation.as_mut() => panic!("operation must retry after conflict"),
            };
        }
        operation.await;
        // Then
        let records = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &input.test_execution_id(),
        )
        .await
        .unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record.fact, NodeFact::CommandSpawned(_)))
                .count(),
            CONFLICT_COUNT + usize::from(spawned)
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record.fact, NodeFact::RuntimeFailureObserved(_)))
                .count(),
            usize::from(!spawned)
        );
    }
}

#[tokio::test]
pub async fn test_session準備競合_最新記録で紐付けを再評価し準備を繰り返さない() {
    const CONFLICT_COUNT: usize = 5;
    for change in ["sibling", "abort", "attached", "exhausted"] {
        // Given
        let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
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
            CONFLICT_COUNT
        } else {
            1
        };

        // When
        for attempt in 0..attempts {
            crate::adaptor_gateway_workflow_workflow_host_test_helpers::poll_until_pending(
                operation.as_mut(),
                || Arc::strong_count(&commit_lock) > 1,
            )
            .await;
            releash_lib::test_support::integration::workflow::append_facts_for_events(
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
            .await
            .unwrap();
            if attempt + 1 == attempts {
                drop(guard);
                break;
            }
            let mut next_guard = Box::pin(commit_lock.lock());
            assert!(futures_util::poll!(next_guard.as_mut()).is_pending());
            drop(guard);
            guard = tokio::select! {
                guard = next_guard => guard,
                _ = operation.as_mut() => panic!("operation must retry after conflict"),
            };
        }
        let result = operation.await;

        // Then
        if matches!(change, "sibling" | "exhausted") {
            result.unwrap();
            assert!(fixture.sessions.rolled_back.lock().unwrap().is_empty());
            assert_eq!(
                fixture.sessions.activated.lock().unwrap().as_slice(),
                std::slice::from_ref(&target.id)
            );
        } else {
            let error = result.unwrap_err();
            assert!(matches!(error, WorkflowRuntimeError::InvalidState(_)));
            assert_eq!(
                fixture.sessions.rolled_back.lock().unwrap().as_slice(),
                std::slice::from_ref(&target.id)
            );
            assert!(fixture.sessions.activated.lock().unwrap().is_empty());
        }
        assert_eq!(fixture.sessions.prepared.lock().unwrap().len(), 1);
        let records = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &snapshot.execution_id,
        )
        .await
        .unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record.fact, NodeFact::SessionAttached(_)))
                .count(),
            usize::from(matches!(change, "sibling" | "attached" | "exhausted"))
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
            "sibling" | "exhausted" => Some(format!("agent-{}", target.id)),
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
pub async fn test_自動再起動競合_最新記録で再評価し上限まで再試行する() {
    const CONFLICT_COUNT: usize = 5;
    for change in ["sibling", "abort", "exhausted"] {
        // Given
        let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
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
            CONFLICT_COUNT
        } else {
            1
        };

        // When
        for attempt in 0..attempts {
            crate::adaptor_gateway_workflow_workflow_host_test_helpers::poll_until_pending(
                operation.as_mut(),
                || Arc::strong_count(&commit_lock) > 1,
            )
            .await;
            releash_lib::test_support::integration::workflow::append_facts_for_events(
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
            .await
            .unwrap();
            if attempt + 1 == attempts {
                drop(guard);
                break;
            }
            let mut next_guard = Box::pin(commit_lock.lock());
            assert!(futures_util::poll!(next_guard.as_mut()).is_pending());
            drop(guard);
            guard = tokio::select! {
                guard = next_guard => guard,
                _ = operation.as_mut() => panic!("operation must retry after conflict"),
            };
        }
        let result = operation.await;

        // Then
        match change {
            "sibling" | "exhausted" => {
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
            _ => unreachable!(),
        }
        let records = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &snapshot.execution_id,
        )
        .await
        .unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record.fact, NodeFact::RetryRequested))
                .count(),
            usize::from(matches!(change, "sibling" | "exhausted"))
        );
        assert!(!records
            .iter()
            .any(|record| matches!(record.fact, NodeFact::RuntimeFailureObserved(_))));
    }
}

#[tokio::test]
pub async fn test_自動再起動_先行nodeのエラーを後続の起動成功nodeへ記録しない() {
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
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
    releash_lib::test_support::integration::workflow::append_facts_for_events(
        &fixture.store,
        &[WorkflowEvent::SessionAttached {
            execution_id: snapshot.execution_id.clone(),
            node_execution_id: target.id.clone(),
            session_id: "unavailable-session".into(),
            timestamp: current_timestamp(),
        }],
    )
    .await
    .unwrap();
    *fixture.sessions.presence_error_session.lock().unwrap() = Some("unavailable-session".into());

    // When
    fixture
        .host
        .schedule_startup_retries(
            &fixture.app,
            &snapshot.execution_id,
            "/repo",
            vec![target.id.as_str().into(), sibling.id.as_str().into()],
        )
        .await;
    fixture.wait_startup_retries().await;

    // Then
    let records = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        &snapshot.execution_id,
    )
    .await
    .unwrap();
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
        fixture.sessions.activated.lock().unwrap().as_slice(),
        std::slice::from_ref(&restarted.id)
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

#[tokio::test]
pub async fn test_node事実追記_sqlite混雑をruntimeとconnectまで保持する() {
    use connectrpc::ErrorCode;
    use releash_lib::test_support::integration::transport::ConnectFailure;
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
    let snapshot = fixture
        .persist_started("  main: {session: {provider: codex}}\n", "/repo")
        .await;
    let records = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        &snapshot.execution_id,
    )
    .await
    .unwrap();
    let head = records.last().unwrap().seq;
    let connection =
        rusqlite::Connection::open(StoreLayout::new(fixture._directory.path()).database_path())
            .unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    let event = WorkflowEvent::NodeStopReceived {
        execution_id: snapshot.execution_id.clone(),
        node_execution_id: records[0].meta.node_execution_id.clone(),
        timestamp: 3.0,
    };
    // When
    let error = WorkflowRuntimeHost::append_events_at_head(
        &fixture.app,
        &snapshot.execution_id,
        head,
        std::slice::from_ref(&event),
    )
    .await
    .unwrap_err();
    let batch_error = fixture
        .host
        .write_log_required_batch(&fixture.app, &[event])
        .await
        .unwrap_err();
    connection.execute_batch("ROLLBACK").unwrap();
    // Then
    assert_eq!(error.connect_code(), ErrorCode::Unavailable);
    assert_eq!(batch_error.connect_code(), ErrorCode::Unavailable);
    assert_eq!(
        releash_lib::test_support::integration::transport::classified_error(error).code,
        connectrpc::ErrorCode::Unavailable
    );
    assert_eq!(
        releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &snapshot.execution_id
        )
        .await
        .unwrap(),
        records
    );
}

#[tokio::test]
pub async fn test_provider停止_完了済みworkflowと単独sessionはプロセスliveでも行が緑になる() {
    use crate::adaptor_gateway_workflow_workflow_host::workflow_host_tests::runtime_effect_tests::sequential_runtime_effect_fixture;
    use crate::adaptor_gateway_workflow_workflow_host::workflow_host_tests::EFFECT_WORKTREE_PATH;

    use releash_lib::test_support::integration::workspace::WorkspaceNodeStatus;
    use releash_lib::test_support::integration::workspace::WorkspaceNodeStatusClassification;
    use releash_lib::test_support::integration::workspace::WorkspaceTreeRepository;

    async fn node_signals(
        store: &Arc<LocalEventStore>,
        tree_id: &str,
        node_id: &str,
    ) -> (
        NodeExecutionStatus,
        releash_lib::test_support::integration::workflow::NodeCompletionSignalState,
    ) {
        let backend = releash_lib::test_support::integration::workflow::FactLogReadBackend::Live(
            store.clone(),
        );
        let folded =
            releash_lib::test_support::integration::workflow::fold_tree_from(&backend, tree_id)
                .await
                .unwrap()
                .unwrap();
        let node = folded
            .aggregate
            .node_executions
            .iter()
            .find(|node| node.id == node_id)
            .unwrap();
        (node.status, node.completion_signals)
    }

    struct LiveSessionProcess;

    impl releash_lib::test_support::integration::workflow::NodeProcessReader for LiveSessionProcess {
        fn presence(
            &self,
            _workspace: &str,
            _node_execution_id: &str,
            _kind: NodeKindName,
            _session_id: Option<&str>,
        ) -> Result<
            releash_lib::test_support::integration::workflow::NodeProcessPresence,
            releash_lib::test_support::integration::workflow::WorkflowError,
        > {
            Ok(releash_lib::test_support::integration::workflow::NodeProcessPresence::Live)
        }
    }

    // Given
    let fixture = sequential_runtime_effect_fixture().await;
    let workflow_stop = ProviderExecutionTreeStopCommand {
        agent_session_id: fixture.first_agent_session_id.clone(),
        tree_id: fixture.execution_id.clone(),
        node_execution_id: fixture.first_node_execution_id.clone(),
        binding_id: "binding-completed-workflow".to_string(),
    };
    fixture
        .control_plane
        .record_provider_stop(workflow_stop.clone(), Vec::new())
        .await
        .unwrap();
    fixture
        .control_plane
        .submit_output(SubmitOutputCommand {
            node_execution_id: fixture.first_node_execution_id.clone(),
            artifact: None,
        })
        .await
        .unwrap();
    let records = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        &fixture.execution_id,
    )
    .await
    .unwrap();
    let workflow_meta = &records
        .iter()
        .find(|record| record.meta.node_execution_id == fixture.first_node_execution_id)
        .unwrap()
        .meta;
    releash_lib::test_support::integration::workflow::append_single_fact(
        &fixture.store,
        workflow_meta,
        &NodeFact::AgentActivityObserved(
            releash_lib::test_support::integration::workflow::AgentActivityObservedFact {
                activity:
                    releash_lib::test_support::integration::workflow::AgentSessionActivity::Working,
            },
        ),
        (current_timestamp() * 1000.0) as i64,
    )
    .await
    .unwrap();

    let standalone_id = "agent-session-completed-stop";
    LocalAgentSessionRepository::new(fixture.store.clone())
        .create(
            AgentSession::create(
                standalone_id,
                WorkspaceIdentity::new(EFFECT_WORKTREE_PATH),
                EFFECT_WORKTREE_PATH,
                ProviderKind::Codex,
                AgentSessionTreeLocation::session_tree_root(standalone_id).unwrap(),
            )
            .unwrap(),
            "create-completed-stop",
        )
        .await
        .unwrap();
    fixture
        .host
        .register_started_execution_tree(&fixture._app, standalone_id)
        .await
        .unwrap();
    let standalone_records = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        standalone_id,
    )
    .await
    .unwrap();
    let standalone_meta = &standalone_records
        .iter()
        .find(|record| record.meta.node_execution_id == standalone_id)
        .unwrap()
        .meta;
    releash_lib::test_support::integration::workflow::append_single_fact(
        &fixture.store,
        standalone_meta,
        &NodeFact::AgentActivityObserved(
            releash_lib::test_support::integration::workflow::AgentActivityObservedFact {
                activity:
                    releash_lib::test_support::integration::workflow::AgentSessionActivity::Working,
            },
        ),
        (current_timestamp() * 1000.0) as i64,
    )
    .await
    .unwrap();
    let mut repository = SqliteWorkspaceTreeRepository::new(fixture.store.clone());
    Arc::get_mut(&mut repository).unwrap().processes = Some(Arc::new(LiveSessionProcess));
    let standalone_node_id = standalone_id.to_string();
    for node_id in [&fixture.first_node_execution_id, &standalone_node_id] {
        let node = repository
            .load_node_by_node_execution_id(node_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            node.status_classification,
            WorkspaceNodeStatusClassification::Active
        );
        assert_eq!(
            node.process_presence,
            releash_lib::test_support::integration::workflow::NodeProcessPresence::Live
        );
        assert_eq!(node.status, WorkspaceNodeStatus::Completed);
    }

    let workflow_before = node_signals(
        &fixture.store,
        &fixture.execution_id,
        &fixture.first_node_execution_id,
    )
    .await;
    let standalone_before = node_signals(&fixture.store, standalone_id, standalone_id).await;

    // When
    fixture
        .control_plane
        .record_provider_stop(workflow_stop, Vec::new())
        .await
        .unwrap();
    fixture
        .control_plane
        .record_provider_stop(
            ProviderExecutionTreeStopCommand {
                agent_session_id: standalone_id.to_string(),
                tree_id: standalone_id.to_string(),
                node_execution_id: standalone_id.to_string(),
                binding_id: "binding-completed-standalone".to_string(),
            },
            Vec::new(),
        )
        .await
        .unwrap();

    // Then
    for node_id in [&fixture.first_node_execution_id, &standalone_node_id] {
        let node = repository
            .load_node_by_node_execution_id(node_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            node.status_classification,
            WorkspaceNodeStatusClassification::Idle
        );
        assert_eq!(
            node.process_presence,
            releash_lib::test_support::integration::workflow::NodeProcessPresence::Live
        );
        assert_eq!(node.status, WorkspaceNodeStatus::Completed);
    }
    assert_eq!(
        node_signals(
            &fixture.store,
            &fixture.execution_id,
            &fixture.first_node_execution_id,
        )
        .await,
        workflow_before
    );
    assert_eq!(
        node_signals(&fixture.store, standalone_id, standalone_id).await,
        standalone_before
    );
}

#[tokio::test]
pub async fn test_command完了監視_起動元の期限後も完了を反映して監視を外す() {
    use releash_lib::test_support::integration::platform::OperationStopped;
    use std::time::Duration;
    use std::time::Instant;
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
    let cwd = fixture._directory.path().to_str().unwrap();
    let snapshot = fixture
        .persist_started("  main: {command: true}", cwd)
        .await;
    let mut input = command_input(&snapshot);
    *input.test_raw_command_mut() =
        Some("while [ ! -f release ]; do sleep 0.01; done; printf done".into());
    let id = input.test_node_execution_id().clone();
    let deadline = Instant::now() + Duration::from_millis(300);
    let context =
        releash_lib::test_support::integration::platform::ingress(Some(deadline), async {
            fixture
                .host
                .spawn_command_execution(&fixture.app, input)
                .await
                .unwrap();
            releash_lib::test_support::integration::platform::current()
        })
        .await
        .unwrap();
    assert!(fixture
        .host
        .test_command_completion_observers()
        .lock()
        .await
        .contains_key(&id));
    // When
    tokio::time::sleep(deadline.saturating_duration_since(Instant::now())).await;
    assert_eq!(
        context.check(Instant::now()),
        Err(OperationStopped::Expired)
    );
    std::fs::write(fixture._directory.path().join("release"), "").unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if !fixture
                .host
                .test_command_completion_observers()
                .lock()
                .await
                .contains_key(&id)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Then
    let records = releash_lib::test_support::integration::workflow::read_tree_records(
        &fixture.store,
        &snapshot.execution_id,
    )
    .await
    .unwrap();
    assert!(records
        .iter()
        .any(|record| record.meta.node_execution_id == id
            && matches!(
                record.fact,
                releash_lib::test_support::integration::workflow::NodeFact::ProcessExited(_)
            )));
    let folded = releash_lib::test_support::integration::workflow::fold_tree_from(
        &releash_lib::test_support::integration::workflow::FactLogReadBackend::Live(
            fixture.store.clone(),
        ),
        &snapshot.execution_id,
    )
    .await
    .unwrap()
    .unwrap();
    let node = folded
        .aggregate
        .node_executions
        .iter()
        .find(|node| node.id == id)
        .unwrap();
    assert_eq!(node.status, NodeExecutionStatus::Succeeded);
    assert!(fixture
        .host
        .test_command_completion_observers()
        .lock()
        .await
        .is_empty());
}
pub(crate) mod command_env_tests {

    use releash_lib::test_support::integration::workflow::command_env;
    use releash_lib::test_support::integration::workflow::CommandExecutionInput;
    use releash_lib::test_support::integration::workflow::WorkflowDefinition;
    use std::collections::BTreeMap;

    #[tokio::test]
    pub async fn test_command_env_yaml定義と束縛から子processへstringとjsonを渡す() {
        let workflow = serde_saphyr::from_str::<WorkflowDefinition>(
            r#"name: env-runtime
description: env runtime
nodes:
  main:
    command: 'printf "%s\n" "$DOC" "$META" "$COUNT"'
    input:
      - document
      - metadata
    env:
      DOC: document
      META: metadata
      COUNT: metadata.count
"#,
        )
        .unwrap();
        let command = workflow.entry_node().unwrap().command_spec().unwrap();
        let bindings = vec![
            (
                "document".to_string(),
                serde_json::Value::String("plain document".to_string()),
            ),
            (
                "metadata".to_string(),
                serde_json::json!({"count": 2, "ready": true}),
            ),
        ];
        let definition_env =
            releash_lib::test_support::integration::workflow::resolve_command_environment(
                &command.env,
                &bindings,
            )
            .unwrap();
        let cwd = tempfile::TempDir::new().unwrap();
        let input = CommandExecutionInput::test_new(
            (
                "execution-1".to_string(),
                "node-execution-1".to_string(),
                "main".to_string(),
                1,
            ),
            cwd.path().to_string_lossy().into_owned(),
            Some(command.command.clone()),
            Vec::new(),
            None,
            BTreeMap::new(),
            None,
        );

        let output = releash_lib::test_support::integration::process::spawn_shell_command(
            cwd.path(),
            &command.command,
            command_env(&input, definition_env),
            "workflow command",
            releash_lib::test_support::integration::process::OutputLimit {
                max_bytes: releash_lib::test_support::integration::workflow::MAX_OUTPUT_SIZE,
                truncation_marker:
                    releash_lib::test_support::integration::workflow::TRUNCATION_MARKER,
            },
        )
        .unwrap()
        .wait()
        .await
        .unwrap();

        assert_eq!(output.exit_code, 0);
        assert_eq!(
            output.stdout,
            "plain document\n{\"count\":2,\"ready\":true}\n2\n"
        );
    }
}
pub(crate) mod workflow_host_tests {
    use super::{RecordingWorkflowAgentSessions, EFFECT_AGENT_SESSION_ID};
    use crate::adaptor_gateway_workflow_workflow_host_test_helpers::record_workflow_execution_broadcasts;
    use crate::adaptor_gateway_workflow_workflow_host_test_helpers::take_workflow_execution_broadcasts;
    use releash_lib::test_support::integration::persistence::FaultInjector;
    use releash_lib::test_support::integration::persistence::LocalEventStore;
    use releash_lib::test_support::integration::persistence::LocalEventStoreConfig;
    use releash_lib::test_support::integration::persistence::NewNodeEventRow;
    use releash_lib::test_support::integration::platform::LoadStreamRequest;
    use releash_lib::test_support::integration::platform::StreamId;
    use releash_lib::test_support::integration::process::CommandRunOutput;
    use releash_lib::test_support::integration::providers::ProviderKind;
    use releash_lib::test_support::integration::providers::ProviderLifecycleEvent;
    use releash_lib::test_support::integration::providers::ProviderLifecycleScope;
    use releash_lib::test_support::integration::providers::ScopedProviderLifecycleEvent;
    use releash_lib::test_support::integration::repository::LocalEventTransactionRepository;
    use releash_lib::test_support::integration::sessions::AgentSession;
    use releash_lib::test_support::integration::sessions::AgentSessionRepository;
    use releash_lib::test_support::integration::sessions::AgentSessionTreeLocation;
    use releash_lib::test_support::integration::sessions::LocalAgentSessionRepository;
    use releash_lib::test_support::integration::workflow::current_timestamp;
    use releash_lib::test_support::integration::workflow::ChildEntry;
    use releash_lib::test_support::integration::workflow::CommandExecutionInput;
    use releash_lib::test_support::integration::workflow::ExecutionOrigin;
    use releash_lib::test_support::integration::workflow::ExecutionParentRef;
    use releash_lib::test_support::integration::workflow::ExecutionTreeLaunch;
    use releash_lib::test_support::integration::workflow::ManagedWorktreeResolver;
    use releash_lib::test_support::integration::workflow::NodeExecutionStatus;
    use releash_lib::test_support::integration::workflow::NodeKindName;
    use releash_lib::test_support::integration::workflow::NodeSessionInfo;
    use releash_lib::test_support::integration::workflow::RuntimeExecutionState;
    use releash_lib::test_support::integration::workflow::WorkflowAgentSessionPort;
    use releash_lib::test_support::integration::workflow::WorkflowDefaults;
    use releash_lib::test_support::integration::workflow::WorkflowDefinition;
    use releash_lib::test_support::integration::workflow::WorkflowDefinitionResolver;
    use releash_lib::test_support::integration::workflow::WorkflowEvent;
    use releash_lib::test_support::integration::workflow::WorkflowExecutionInsert;
    use releash_lib::test_support::integration::workflow::WorkflowRuntimeCommandGateway;
    use releash_lib::test_support::integration::workflow::WorkflowRuntimeDependencies;
    use releash_lib::test_support::integration::workflow::WorkflowRuntimeError;
    use releash_lib::test_support::integration::workflow::WorkflowRuntimeHost;
    use releash_lib::test_support::integration::workflow::WorkflowSessionLaunchConfig;
    use releash_lib::test_support::integration::workspace::SqliteWorkspaceTreeRepository;
    use std::sync::Arc;

    use releash_lib::test_support::integration::workflow::definition_FacetRefs as FacetRefs;
    use releash_lib::test_support::integration::workflow::NodeCompletion;
    use releash_lib::test_support::integration::workflow::NodeDefinition;
    use releash_lib::test_support::integration::workflow::NodeFact;
    use releash_lib::test_support::integration::workflow::NodeFactMeta;
    use releash_lib::test_support::integration::workflow::NodeKind;
    use releash_lib::test_support::integration::workflow::SequenceSpec;
    use releash_lib::test_support::integration::workflow::SessionExecutionTreeRootFacts;
    use releash_lib::test_support::integration::workflow::SessionPermission;
    use releash_lib::test_support::integration::workflow::SessionSpec;
    use releash_lib::test_support::integration::workflow::StartedFact;
    use releash_lib::test_support::integration::workflow::TreeRootFact;

    use releash_lib::test_support::integration::providers::ProviderExecutionTreeStopCommand;
    use releash_lib::test_support::integration::workflow::ApprovalCommand;
    use releash_lib::test_support::integration::workflow::ManagedWorktreeResolverError;
    use releash_lib::test_support::integration::workflow::SubmitOutputCommand;
    use releash_lib::test_support::integration::workflow::WorkflowControlPlaneUsecase;
    use releash_lib::test_support::integration::workflow::WorkflowDefinitionResolverError;
    use releash_lib::test_support::integration::workspace::WorkspaceIdentity;
    use releash_lib::test_support::integration::workspace::WorkspaceNodeStatusClassification;
    use releash_lib::test_support::integration::workspace::WorkspaceTreeRepository;

    pub(crate) const EFFECT_WORKTREE_PATH: &str = "/repo/effect-test";
    const EFFECT_NODE_NAME: &str = "agent";

    pub(crate) struct UnusedWorkflowResolver;

    #[async_trait::async_trait]
    impl WorkflowDefinitionResolver for UnusedWorkflowResolver {
        async fn resolve(
            &self,
            _workflow_name: &str,
        ) -> Result<WorkflowDefinition, WorkflowDefinitionResolverError> {
            Err(WorkflowDefinitionResolverError::Infrastructure(
                "unused in startup recovery".to_string(),
            ))
        }
    }

    struct UnusedWorktreeResolver;

    pub(crate) struct AcceptingWorktreeResolver;

    #[async_trait::async_trait]
    impl ManagedWorktreeResolver for UnusedWorktreeResolver {
        async fn resolve(
            &self,
            _worktree_path: String,
        ) -> Result<String, ManagedWorktreeResolverError> {
            Err(ManagedWorktreeResolverError::Validation(
                "unused in startup recovery".to_string(),
            ))
        }
    }

    #[async_trait::async_trait]
    impl ManagedWorktreeResolver for AcceptingWorktreeResolver {
        async fn resolve(
            &self,
            worktree_path: String,
        ) -> Result<String, ManagedWorktreeResolverError> {
            Ok(worktree_path)
        }
    }

    struct FailingWorkflowAgentSessions;

    #[tokio::test]
    pub async fn test_command完了_承認要求ありなら承認後に完了し省略時は自動完了する() {
        // Given
        for parent in ["", "  main:\n    sequence: {children: [run]}\n"] {
            for (completion, exit_code) in [
                ("", 0),
                ("", 7),
                ("    completion: {require: approval}\n", 0),
                ("    completion: {require: approval}\n", 7),
            ] {
                let require_approval = !completion.is_empty();
                let directory = tempfile::tempdir().unwrap();
                let store = LocalEventStore::open(LocalEventStoreConfig::production(
                    directory.path().to_path_buf(),
                    std::sync::Arc::new(
                        releash_lib::test_support::integration::platform::RetryLimiter::new(),
                    ),
                ))
                .unwrap();
                let app = crate::adaptor_gateway_workflow_workflow_host_test_helpers::dependencies(
                    Some(store.clone()),
                );
                let host = Arc::new(WorkflowRuntimeHost::with_runtime_ports(
                    releash_lib::test_support::integration::platform::shared().clone(),
                    Arc::new(UnusedWorkflowResolver),
                    Arc::new(AcceptingWorktreeResolver),
                    crate::adaptor_gateway_workflow_workflow_host_test_helpers::workspace_query(store.clone()),
                    Arc::new(FailingWorkflowAgentSessions),
                    Arc::new(crate::adaptor_gateway_workflow_workflow_host_test_helpers::TestWorktrees::default()),
                    releash_lib::test_support::integration::daemon::serving(),
                ));
                let node_name = if parent.is_empty() { "main" } else { "run" };
                let workflow = serde_saphyr::from_str::<WorkflowDefinition>(&format!(
                    "name: command-completion\ndescription: test\nnodes:\n{parent}  {node_name}:\n    command: 'true'\n{completion}"
                ))
                .unwrap();
                let worktree_path = directory.path().to_string_lossy().into_owned();
                let now = current_timestamp();
                let execution_id = uuid::Uuid::new_v4().to_string();
                let (started, applied) = host
                    .insert_workflow_execution(WorkflowExecutionInsert::test_new(
                        execution_id.clone(),
                        workflow.clone(),
                        worktree_path.clone(),
                        None,
                        ExecutionOrigin::DesktopUi,
                        WorkflowDefaults,
                        now,
                    ))
                    .await
                    .unwrap();
                let mut start_events = vec![WorkflowEvent::ExecutionStarted {
                    repository_root: None,
                    execution_id: execution_id.clone(),
                    workflow_name: workflow.name.clone(),
                    worktree_path: worktree_path.clone(),
                    created_from: ExecutionOrigin::DesktopUi,
                    request: String::new(),
                    definition: workflow,
                    timestamp: now,
                }];
                start_events.extend(applied.events);
                host.write_log_required_batch(&app, &start_events)
                    .await
                    .unwrap();
                let node = started
                    .node_executions
                    .iter()
                    .find(|node| node.node_name == node_name)
                    .unwrap();
                assert_eq!(node.status, NodeExecutionStatus::Running);
                let node_execution_id = node.id.clone();
                let input = CommandExecutionInput::test_new(
                    (
                        execution_id.clone(),
                        node_execution_id.clone(),
                        node_name.to_string(),
                        node.attempt,
                    ),
                    worktree_path,
                    Some("true".to_string()),
                    Vec::new(),
                    None,
                    Default::default(),
                    None,
                );
                let mut broadcasts = record_workflow_execution_broadcasts(&app);

                // When
                host.commit_command_output(
                    &app,
                    input,
                    CommandRunOutput {
                        exit_code,
                        stdout: "command finished".to_string(),
                        stderr: String::new(),
                        duration_ms: 10,
                    },
                )
                .await
                .unwrap();

                // Then
                let records = releash_lib::test_support::integration::workflow::read_tree_records(
                    &store,
                    &execution_id,
                )
                .await
                .unwrap();
                assert!(records.iter().any(|record| matches!(
                    &record.fact,
                    NodeFact::ArtifactProduced(fact) if record.meta.node_execution_id == node_execution_id && fact.value["stdout"] == "command finished" && fact.value["ok"] == (exit_code == 0)
                )));
                assert!(!records
                    .iter()
                    .any(|record| matches!(record.fact, NodeFact::ApprovalGranted(_))));
                assert!(!take_workflow_execution_broadcasts(&mut broadcasts).is_empty());
                let snapshot = host
                    .get_state_by_execution_id(&app, &execution_id)
                    .await
                    .unwrap();
                let expected = if require_approval {
                    NodeExecutionStatus::WaitingApproval
                } else {
                    NodeExecutionStatus::Succeeded
                };
                assert_eq!(
                    snapshot
                        .node_executions
                        .iter()
                        .find(|node| node.id == node_execution_id)
                        .unwrap()
                        .status,
                    expected
                );
                if require_approval {
                    let snapshot = host
                        .get_state_by_execution_id(&app, &execution_id)
                        .await
                        .unwrap();
                    assert_eq!(
                        snapshot
                            .node_executions
                            .iter()
                            .find(|node| node.id == node_execution_id)
                            .unwrap()
                            .status,
                        NodeExecutionStatus::WaitingApproval
                    );
                    assert_ne!(snapshot.state, RuntimeExecutionState::Completed);
                    let gateway = Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
                        app.clone(),
                        host.clone(),
                    ));
                    WorkflowControlPlaneUsecase::new(
                        releash_lib::test_support::integration::platform::shared().clone(),
                        gateway,
                    )
                    .resolve_approval(ApprovalCommand {
                        execution_id: execution_id.clone(),
                        node_name: node_name.to_string(),
                        node_execution_id: Some(node_execution_id.clone()),
                        comment: None,
                    })
                    .await
                    .unwrap();
                    let records =
                        releash_lib::test_support::integration::workflow::read_tree_records(
                            &store,
                            &execution_id,
                        )
                        .await
                        .unwrap();
                    assert!(records
                        .iter()
                        .any(|record| record.meta.node_execution_id == node_execution_id
                            && matches!(record.fact, NodeFact::ApprovalGranted(_))));
                }
                let completed = host
                    .get_state_by_execution_id(&app, &execution_id)
                    .await
                    .unwrap();
                assert_eq!(completed.state, RuntimeExecutionState::Completed);
                assert!(completed
                    .node_executions
                    .iter()
                    .all(|node| node.status == NodeExecutionStatus::Succeeded));
                let folded = releash_lib::test_support::integration::workflow::fold_tree_from(
                    &releash_lib::test_support::integration::workflow::FactLogReadBackend::Live(
                        store.clone(),
                    ),
                    &execution_id,
                )
                .await
                .unwrap()
                .unwrap();
                let replayed = folded.aggregate.node_execution(&node_execution_id).unwrap();
                assert_eq!(replayed.status, NodeExecutionStatus::Succeeded);
                assert_eq!(replayed.artifact.as_ref().unwrap()["ok"], exit_code == 0);
                assert_eq!(replayed.artifact.as_ref().unwrap()["exit_code"], exit_code);
                assert_eq!(*folded.aggregate.state(), RuntimeExecutionState::Completed);
            }
        }
    }

    #[tokio::test]
    pub async fn test_command完了_追記結果不明でもdurableな完了へliveとactiveを収束する() {
        // Given
        for durable in [false, true] {
            let fixture =
                crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
            let started = fixture
                .persist_started("  main: {command: 'true'}\n", "/repo")
                .await;
            let execution_id = &started.execution_id;
            let node = &started.node_executions[0];
            fixture
                .host
                .register_started_execution_tree(&fixture.app, execution_id)
                .await
                .unwrap();
            let input = CommandExecutionInput::test_new(
                (
                    execution_id.clone(),
                    node.id.clone(),
                    node.node_name.clone(),
                    node.attempt,
                ),
                "/repo".into(),
                Some("true".into()),
                Vec::new(),
                None,
                Default::default(),
                None,
            );
            if durable {
                fixture.store.fault_injector().arm_drop_reply();
            } else {
                fixture.store.close_write_queue_for_tests();
            }

            // When
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                fixture.host.commit_command_output(
                    &fixture.app,
                    input.clone(),
                    CommandRunOutput {
                        exit_code: 0,
                        stdout: "done".into(),
                        stderr: String::new(),
                        duration_ms: 1,
                    },
                ),
            )
            .await;

            // Then
            if durable {
                result.unwrap().unwrap();
            } else {
                assert!(result.is_err());
            }
            let records = releash_lib::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                execution_id,
            )
            .await
            .unwrap();
            assert_eq!(
                records
                    .iter()
                    .filter(|record| matches!(record.fact, NodeFact::ExecutionCompleted))
                    .count(),
                usize::from(durable)
            );
            let expected_state = if durable {
                RuntimeExecutionState::Completed
            } else {
                RuntimeExecutionState::Running
            };
            let current = fixture
                .host
                .get_state_by_execution_id(&fixture.app, execution_id)
                .await
                .unwrap();
            assert_eq!(current.state, expected_state);
            let folded = releash_lib::test_support::integration::workflow::fold_tree_from(
                &releash_lib::test_support::integration::workflow::FactLogReadBackend::Live(
                    fixture.store.clone(),
                ),
                execution_id,
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(folded.aggregate.state(), &expected_state);
            if durable {
                assert!(
                    !fixture
                        .host
                        .command_execution_still_current(&fixture.app, &input)
                        .await
                );
                fixture
                    .host
                    .commit_command_output(
                        &fixture.app,
                        input,
                        CommandRunOutput {
                            exit_code: 0,
                            stdout: "duplicate".into(),
                            stderr: String::new(),
                            duration_ms: 1,
                        },
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    releash_lib::test_support::integration::workflow::read_tree_records(
                        &fixture.store,
                        execution_id
                    )
                    .await
                    .unwrap(),
                    records
                );
            }
        }
    }

    #[tokio::test]
    pub async fn test_承認完了_追記結果不明でもcanonicalな状態へliveとactiveを収束する() {
        // Given
        for durable in [false, true] {
            let fixture =
                crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
            let started = fixture
                .persist_started(
                    "  main:\n    session: {provider: claude}\n    completion: {require: approval}\n",
                    "/repo",
                )
                .await;
            let execution_id = &started.execution_id;
            let node = &started.node_executions[0];
            let timestamp = current_timestamp();
            fixture
                .host
                .write_log_required_batch(
                    &fixture.app,
                    &[
                        WorkflowEvent::NodeSubmitReceived {
                            execution_id: execution_id.clone(),
                            node_execution_id: node.id.clone(),
                            timestamp,
                        },
                        WorkflowEvent::NodeStopReceived {
                            execution_id: execution_id.clone(),
                            node_execution_id: node.id.clone(),
                            timestamp,
                        },
                    ],
                )
                .await
                .unwrap();
            fixture
                .host
                .register_started_execution_tree(&fixture.app, execution_id)
                .await
                .unwrap();
            let before = fixture
                .host
                .get_state_by_execution_id(&fixture.app, execution_id)
                .await
                .unwrap();
            assert_eq!(before.state, RuntimeExecutionState::Running);
            assert_eq!(
                before.node_executions[0].status,
                NodeExecutionStatus::WaitingApproval
            );
            let gateway = Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
                fixture.app.clone(),
                Arc::new(fixture.host.clone()),
            ));
            if durable {
                fixture.store.fault_injector().arm_drop_reply();
            } else {
                fixture.store.close_write_queue_for_tests();
            }

            // When
            let control_plane = WorkflowControlPlaneUsecase::new(
                releash_lib::test_support::integration::platform::shared().clone(),
                gateway,
            );
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                control_plane.resolve_approval(ApprovalCommand {
                    execution_id: execution_id.clone(),
                    node_name: node.node_name.clone(),
                    node_execution_id: Some(node.id.clone()),
                    comment: None,
                }),
            )
            .await;

            // Then
            if durable {
                result.unwrap().unwrap();
            } else {
                assert!(result.is_err());
            }
            let records = releash_lib::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                execution_id,
            )
            .await
            .unwrap();
            assert_eq!(
                records
                    .iter()
                    .filter(|record| matches!(record.fact, NodeFact::ApprovalGranted(_)))
                    .count(),
                usize::from(durable)
            );
            assert_eq!(
                records
                    .iter()
                    .filter(|record| matches!(record.fact, NodeFact::ExecutionCompleted))
                    .count(),
                usize::from(durable)
            );
            let expected_state = if durable {
                RuntimeExecutionState::Completed
            } else {
                RuntimeExecutionState::Running
            };
            let current = fixture
                .host
                .get_state_by_execution_id(&fixture.app, execution_id)
                .await
                .unwrap();
            assert_eq!(current.state, expected_state);
            assert_eq!(
                current.node_executions[0].status,
                if durable {
                    NodeExecutionStatus::Succeeded
                } else {
                    NodeExecutionStatus::WaitingApproval
                }
            );
            let folded = releash_lib::test_support::integration::workflow::fold_tree_from(
                &releash_lib::test_support::integration::workflow::FactLogReadBackend::Live(
                    fixture.store.clone(),
                ),
                execution_id,
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(folded.aggregate.state(), &expected_state);
        }
    }

    #[tokio::test]
    pub async fn test_command_env_未束縛inputではprocessを起動せずnode_failureにする() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ))
        .unwrap();
        let app = crate::adaptor_gateway_workflow_workflow_host_test_helpers::dependencies(Some(
            store.clone(),
        ));
        let host = WorkflowRuntimeHost::with_runtime_ports(
            releash_lib::test_support::integration::platform::shared().clone(),
            Arc::new(UnusedWorkflowResolver),
            Arc::new(AcceptingWorktreeResolver),
            crate::adaptor_gateway_workflow_workflow_host_test_helpers::workspace_query(
                store.clone(),
            ),
            Arc::new(FailingWorkflowAgentSessions),
            Arc::new(
                crate::adaptor_gateway_workflow_workflow_host_test_helpers::TestWorktrees::default(
                ),
            ),
            releash_lib::test_support::integration::daemon::serving(),
        );
        let workflow = serde_saphyr::from_str::<WorkflowDefinition>(
            r#"name: missing-command-env
description: missing command env
nodes:
  main:
    command: 'printf spawned > command-spawned.marker'
    input:
      - document
    env:
      DOCUMENT: document
"#,
        )
        .unwrap();

        let execution_id = host
            .start_resolved_workflow(
                &app,
                workflow,
                directory.path().to_string_lossy().into_owned(),
                None,
                ExecutionOrigin::DesktopUi,
            )
            .await
            .unwrap();

        assert!(!directory.path().join("command-spawned.marker").exists());
        crate::adaptor_gateway_workflow_workflow_host_test_helpers::wait_startup_retries(&host)
            .await;
        let snapshot = host
            .get_state_by_execution_id(&app, &execution_id)
            .await
            .unwrap();
        assert_eq!(snapshot.node_executions.len(), 1);
        assert_eq!(
            snapshot.node_executions.last().unwrap().status,
            NodeExecutionStatus::Running
        );
        let records = releash_lib::test_support::integration::workflow::read_tree_records(
            &store,
            &execution_id,
        )
        .await
        .unwrap();
        assert!(records.iter().any(|record| matches!(
            &record.fact,
            NodeFact::RuntimeFailureObserved(fact) if !fact.reason.is_empty()
        )));
        assert!(!records
            .iter()
            .any(|record| matches!(record.fact, NodeFact::CommandSpawned(_))));
        let restored = releash_lib::test_support::integration::workflow::fold_tree_from(
            &releash_lib::test_support::integration::workflow::FactLogReadBackend::Live(store),
            &execution_id,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            restored.aggregate.node_executions.last().unwrap().status,
            NodeExecutionStatus::Running
        );
        assert!(restored
            .aggregate
            .node_executions
            .last()
            .unwrap()
            .can_retry(
            releash_lib::test_support::integration::workflow::NodeProcessPresence::ConfirmedAbsent
        ));
    }

    #[tokio::test]
    pub async fn test_command_env_nulによるspawn失敗を既存node_failureにする() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ))
        .unwrap();
        let app = crate::adaptor_gateway_workflow_workflow_host_test_helpers::dependencies(Some(
            store.clone(),
        ));
        let host = WorkflowRuntimeHost::with_runtime_ports(
            releash_lib::test_support::integration::platform::shared().clone(),
            Arc::new(UnusedWorkflowResolver),
            Arc::new(AcceptingWorktreeResolver),
            crate::adaptor_gateway_workflow_workflow_host_test_helpers::workspace_query(
                store.clone(),
            ),
            Arc::new(FailingWorkflowAgentSessions),
            Arc::new(
                crate::adaptor_gateway_workflow_workflow_host_test_helpers::TestWorktrees::default(
                ),
            ),
            releash_lib::test_support::integration::daemon::serving(),
        );
        let workflow = serde_saphyr::from_str::<WorkflowDefinition>(
            r#"name: nul-command-env
description: nul command env
nodes:
  main:
    sequence:
      children:
        - run:
            inputs:
              document: request
  run:
    command: 'printf spawned > command-spawned.marker'
    input:
      - document
    env:
      DOCUMENT: document
"#,
        )
        .unwrap();

        let execution_id = host
            .start_resolved_workflow(
                &app,
                workflow,
                directory.path().to_string_lossy().into_owned(),
                Some("before\0after".to_string()),
                ExecutionOrigin::DesktopUi,
            )
            .await
            .unwrap();

        assert!(!directory.path().join("command-spawned.marker").exists());
        let snapshot = host
            .get_state_by_execution_id(&app, &execution_id)
            .await
            .unwrap();
        assert_eq!(
            snapshot
                .node_executions
                .iter()
                .rev()
                .find(|node| node.node_name == "run")
                .map(|node| node.status),
            Some(NodeExecutionStatus::Running)
        );
        let records = releash_lib::test_support::integration::workflow::read_tree_records(
            &store,
            &execution_id,
        )
        .await
        .unwrap();
        assert!(records.iter().any(|record| matches!(
            &record.fact,
            NodeFact::RuntimeFailureObserved(fact) if !fact.reason.is_empty()
        )));
        assert!(!records
            .iter()
            .any(|record| matches!(record.fact, NodeFact::CommandSpawned(_))));
        let restored = releash_lib::test_support::integration::workflow::fold_tree_from(
            &releash_lib::test_support::integration::workflow::FactLogReadBackend::Live(store),
            &execution_id,
        )
        .await
        .unwrap()
        .unwrap();
        let failed = restored
            .aggregate
            .node_executions
            .iter()
            .rev()
            .find(|node| node.node_name == "run")
            .unwrap();
        assert_eq!(failed.status, NodeExecutionStatus::Running);
        assert!(failed.can_retry(
            releash_lib::test_support::integration::workflow::NodeProcessPresence::ConfirmedAbsent
        ));
    }

    #[async_trait::async_trait]
    impl WorkflowAgentSessionPort for FailingWorkflowAgentSessions {
        async fn has_recoverable_conversation(
            &self,
            _id: &str,
        ) -> Result<bool, WorkflowRuntimeError> {
            Ok(true)
        }

        fn is_provider_available(&self, _provider: ProviderKind) -> bool {
            true
        }

        async fn prepare_workflow_agent_session(
            &self,
            _workspace_worktree_path: &str,
            _worktree_path: &str,
            _config: WorkflowSessionLaunchConfig,
            _workflow_execution_id: &str,
            _node_execution_id: &str,
            _initial_instruction: &str,
        ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
            Err(WorkflowRuntimeError::AgentSession(
                "intentional prepare failure".to_string(),
            ))
        }

        async fn activate_workflow_agent_session(
            &self,
            _node_session_id: &str,
            _node_execution_id: &str,
        ) -> Result<(), WorkflowRuntimeError> {
            unreachable!()
        }

        async fn confirm_workflow_agent_session_attachment(
            &self,
            _node_session_id: &str,
        ) -> Result<(), WorkflowRuntimeError> {
            unreachable!()
        }

        async fn dispatch_continuation(
            &self,
            _node_session_id: &str,
            _child_execution_id: &str,
            _instruction: &str,
        ) -> Result<(), WorkflowRuntimeError> {
            panic!("unexpected delegate continuation")
        }

        async fn recover_workflow_agent_session_provider(
            &self,
            _node_session_id: &str,
            _node_execution_id: &str,
        ) -> Result<(), WorkflowRuntimeError> {
            unreachable!()
        }

        async fn stop_agent_session_for_terminal_node_preserving_checkpoint(
            &self,
            _node_session_id: &str,
            _node_execution_id: &str,
        ) -> Result<(), WorkflowRuntimeError> {
            unreachable!()
        }

        async fn rollback_workflow_agent_session(
            &self,
            _node_session_id: &str,
            _node_execution_id: &str,
        ) -> Result<(), WorkflowRuntimeError> {
            unreachable!()
        }
    }

    pub(crate) mod runtime_effect_tests {
        use super::*;

        fn recording_agent_sessions(
            stop_calls: Arc<std::sync::Mutex<Vec<(String, String)>>>,
            provider_running_checks: Arc<std::sync::Mutex<Vec<(String, String)>>>,
            recovery_fails: Arc<std::sync::atomic::AtomicBool>,
            failing_agent_session_id: String,
        ) -> Arc<dyn WorkflowAgentSessionPort> {
            Arc::new(RecordingWorkflowAgentSessions {
                stop_calls,
                prepare_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
                provider_running_checks,
                recovery_fails,
                failing_agent_session_id,
            })
        }

        struct StopDuringActivationWorkflowAgentSessions {
            control_plane: tokio::sync::Mutex<Option<Arc<WorkflowControlPlaneUsecase>>>,
            execution_id: std::sync::Mutex<Option<String>>,
            activation_count: std::sync::atomic::AtomicUsize,
            confirmation_count: std::sync::atomic::AtomicUsize,
        }

        #[async_trait::async_trait]
        impl WorkflowAgentSessionPort for StopDuringActivationWorkflowAgentSessions {
            async fn has_recoverable_conversation(
                &self,
                _id: &str,
            ) -> Result<bool, WorkflowRuntimeError> {
                Ok(true)
            }

            fn is_provider_available(&self, _provider: ProviderKind) -> bool {
                true
            }

            async fn prepare_workflow_agent_session(
                &self,
                _workspace_worktree_path: &str,
                _worktree_path: &str,
                _config: WorkflowSessionLaunchConfig,
                workflow_execution_id: &str,
                _node_execution_id: &str,
                _initial_instruction: &str,
            ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
                *self.execution_id.lock().unwrap() = Some(workflow_execution_id.to_string());
                Ok(NodeSessionInfo {
                    id: EFFECT_AGENT_SESSION_ID.to_string(),
                })
            }

            async fn activate_workflow_agent_session(
                &self,
                node_session_id: &str,
                node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                self.activation_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let control_plane = self
                    .control_plane
                    .lock()
                    .await
                    .clone()
                    .expect("control plane is bound before activation");
                let execution_id = self
                    .execution_id
                    .lock()
                    .unwrap()
                    .clone()
                    .expect("execution id is recorded during prepare");
                control_plane
                    .record_provider_stop(
                        ProviderExecutionTreeStopCommand {
                            agent_session_id: node_session_id.to_string(),
                            tree_id: execution_id,
                            node_execution_id: node_execution_id.to_string(),
                            binding_id: "binding-stop-during-activation".to_string(),
                        },
                        Vec::new(),
                    )
                    .await
                    .map_err(|error| {
                        WorkflowRuntimeError::InvalidState(format!(
                            "provider Stop during activation was rejected: {error}"
                        ))
                    })
            }

            async fn confirm_workflow_agent_session_attachment(
                &self,
                _node_session_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                assert_eq!(
                    self.activation_count
                        .load(std::sync::atomic::Ordering::SeqCst),
                    1
                );
                self.confirmation_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }

            async fn dispatch_continuation(
                &self,
                _node_session_id: &str,
                _child_execution_id: &str,
                _instruction: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                panic!("unexpected delegate continuation")
            }

            async fn recover_workflow_agent_session_provider(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn stop_agent_session_for_terminal_node_preserving_checkpoint(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn rollback_workflow_agent_session(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }
        }

        #[derive(Debug, Clone, PartialEq, Eq)]
        enum RuntimeEffectCall {
            Activate {
                node_execution_id: String,
                agent_session_id: String,
            },
            Stop {
                node_execution_id: String,
                agent_session_id: String,
            },
        }

        struct OrderedWorkflowAgentSessions {
            calls: Arc<std::sync::Mutex<Vec<RuntimeEffectCall>>>,
        }

        #[async_trait::async_trait]
        impl WorkflowAgentSessionPort for OrderedWorkflowAgentSessions {
            async fn has_recoverable_conversation(
                &self,
                _id: &str,
            ) -> Result<bool, WorkflowRuntimeError> {
                Ok(true)
            }

            fn is_provider_available(&self, _provider: ProviderKind) -> bool {
                true
            }

            async fn prepare_workflow_agent_session(
                &self,
                _workspace_worktree_path: &str,
                _worktree_path: &str,
                _config: WorkflowSessionLaunchConfig,
                _workflow_execution_id: &str,
                node_execution_id: &str,
                _initial_instruction: &str,
            ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
                Ok(NodeSessionInfo {
                    id: format!("agent-session-{node_execution_id}"),
                })
            }

            async fn activate_workflow_agent_session(
                &self,
                node_session_id: &str,
                node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                self.calls
                    .lock()
                    .unwrap()
                    .push(RuntimeEffectCall::Activate {
                        node_execution_id: node_execution_id.to_string(),
                        agent_session_id: node_session_id.to_string(),
                    });
                Ok(())
            }

            async fn confirm_workflow_agent_session_attachment(
                &self,
                _node_session_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn dispatch_continuation(
                &self,
                _node_session_id: &str,
                _child_execution_id: &str,
                _instruction: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                panic!("unexpected delegate continuation")
            }

            async fn recover_workflow_agent_session_provider(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn stop_agent_session_for_terminal_node_preserving_checkpoint(
                &self,
                node_session_id: &str,
                node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                self.calls.lock().unwrap().push(RuntimeEffectCall::Stop {
                    node_execution_id: node_execution_id.to_string(),
                    agent_session_id: node_session_id.to_string(),
                });
                Ok(())
            }

            async fn rollback_workflow_agent_session(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }
        }

        struct NeverResolvingStopWorkflowAgentSessions;

        #[async_trait::async_trait]
        impl WorkflowAgentSessionPort for NeverResolvingStopWorkflowAgentSessions {
            async fn has_recoverable_conversation(
                &self,
                _id: &str,
            ) -> Result<bool, WorkflowRuntimeError> {
                Ok(true)
            }

            fn is_provider_available(&self, _provider: ProviderKind) -> bool {
                true
            }

            async fn prepare_workflow_agent_session(
                &self,
                _workspace_worktree_path: &str,
                _worktree_path: &str,
                _config: WorkflowSessionLaunchConfig,
                _workflow_execution_id: &str,
                _node_execution_id: &str,
                _initial_instruction: &str,
            ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
                Ok(NodeSessionInfo {
                    id: EFFECT_AGENT_SESSION_ID.to_string(),
                })
            }

            async fn activate_workflow_agent_session(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn confirm_workflow_agent_session_attachment(
                &self,
                _node_session_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn dispatch_continuation(
                &self,
                _node_session_id: &str,
                _child_execution_id: &str,
                _instruction: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                panic!("unexpected delegate continuation")
            }

            async fn recover_workflow_agent_session_provider(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn stop_agent_session_for_terminal_node_preserving_checkpoint(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                std::future::pending::<()>().await;
                unreachable!()
            }

            async fn rollback_workflow_agent_session(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }
        }

        struct RuntimeEffectFixture {
            app: WorkflowRuntimeDependencies,
            store: Arc<LocalEventStore>,
            fault: Arc<FaultInjector>,
            host: Arc<WorkflowRuntimeHost>,
            control_plane: WorkflowControlPlaneUsecase,
            stop_calls: Arc<std::sync::Mutex<Vec<(String, String)>>>,
            execution_id: String,
            node_execution_id: String,
            _directory: tempfile::TempDir,
        }

        pub(crate) struct SequentialRuntimeEffectFixture {
            pub(crate) _app: WorkflowRuntimeDependencies,
            pub(crate) store: Arc<LocalEventStore>,
            pub(crate) host: Arc<WorkflowRuntimeHost>,
            pub(crate) control_plane: WorkflowControlPlaneUsecase,
            calls: Arc<std::sync::Mutex<Vec<RuntimeEffectCall>>>,
            pub(crate) execution_id: String,
            pub(crate) first_node_execution_id: String,
            pub(crate) first_agent_session_id: String,
            _directory: tempfile::TempDir,
        }

        async fn runtime_effect_fixture(
            completion: NodeCompletion,
            stop_fails: bool,
        ) -> RuntimeEffectFixture {
            let stop_calls = Arc::new(std::sync::Mutex::new(Vec::new()));
            let sessions = recording_agent_sessions(
                stop_calls.clone(),
                Arc::new(std::sync::Mutex::new(Vec::new())),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                if stop_fails {
                    EFFECT_AGENT_SESSION_ID.to_string()
                } else {
                    String::new()
                },
            );
            runtime_effect_fixture_with_sessions(completion, sessions, stop_calls).await
        }

        async fn runtime_effect_fixture_with_sessions(
            completion: NodeCompletion,
            sessions: Arc<dyn WorkflowAgentSessionPort>,
            stop_calls: Arc<std::sync::Mutex<Vec<(String, String)>>>,
        ) -> RuntimeEffectFixture {
            let directory = tempfile::tempdir().unwrap();
            let fault = Arc::new(FaultInjector::new());
            let mut config = LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
                std::sync::Arc::new(
                    releash_lib::test_support::integration::platform::RetryLimiter::new(),
                ),
            );
            config.fault = fault.clone();
            let store = LocalEventStore::open(config).unwrap();
            let app = crate::adaptor_gateway_workflow_workflow_host_test_helpers::dependencies(
                Some(store.clone()),
            );
            let host = Arc::new(WorkflowRuntimeHost::with_runtime_ports(
                releash_lib::test_support::integration::platform::shared().clone(),
                Arc::new(UnusedWorkflowResolver),
                Arc::new(AcceptingWorktreeResolver),
                crate::adaptor_gateway_workflow_workflow_host_test_helpers::workspace_query(store.clone()),
                sessions,
                Arc::new(crate::adaptor_gateway_workflow_workflow_host_test_helpers::TestWorktrees::default()),
                releash_lib::test_support::integration::daemon::serving(),
            ));
            let nodes = vec![NodeDefinition {
                name: EFFECT_NODE_NAME.to_string(),
                kind: NodeKind::Session(SessionSpec {
                    provider: ProviderKind::Codex,
                    model: None,
                    permission: None,
                    facets: FacetRefs {
                        instruction: Some("policy-confirmation".to_string()),
                        ..FacetRefs::default()
                    },
                }),
                artifact: None,
                input: Vec::new(),
                completion,
                worktree: None,
            }];
            let workflow = WorkflowDefinition {
                name: "runtime-effect-test".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes,
                entry: EFFECT_NODE_NAME.to_string(),
            };
            let execution_id = host
                .start_resolved_workflow(
                    &app,
                    workflow,
                    EFFECT_WORKTREE_PATH.to_string(),
                    None,
                    ExecutionOrigin::DesktopUi,
                )
                .await
                .unwrap();
            let snapshot = host
                .get_state_by_execution_id(&app, &execution_id)
                .await
                .unwrap();
            let node_execution_id = snapshot
                .node_executions
                .iter()
                .find(|node| node.node_name == EFFECT_NODE_NAME)
                .unwrap()
                .id
                .clone();
            let node = snapshot
                .node_executions
                .iter()
                .find(|node| node.id == node_execution_id)
                .unwrap();
            assert_eq!(
                node.status,
                NodeExecutionStatus::Running,
                "unexpected activation state"
            );
            assert_eq!(node.session_id.as_deref(), Some(EFFECT_AGENT_SESSION_ID));
            let gateway = Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
                app.clone(),
                host.clone(),
            ));
            let control_plane = WorkflowControlPlaneUsecase::new(
                releash_lib::test_support::integration::platform::shared().clone(),
                gateway,
            );
            RuntimeEffectFixture {
                app,
                store,
                fault,
                host,
                control_plane,
                stop_calls,
                execution_id,
                node_execution_id,
                _directory: directory,
            }
        }

        pub(crate) async fn sequential_runtime_effect_fixture() -> SequentialRuntimeEffectFixture {
            let directory = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
                std::sync::Arc::new(
                    releash_lib::test_support::integration::platform::RetryLimiter::new(),
                ),
            ))
            .unwrap();
            let app = crate::adaptor_gateway_workflow_workflow_host_test_helpers::dependencies(
                Some(store.clone()),
            );
            let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
            let host = Arc::new(WorkflowRuntimeHost::with_runtime_ports(
                releash_lib::test_support::integration::platform::shared().clone(),
                Arc::new(UnusedWorkflowResolver),
                Arc::new(AcceptingWorktreeResolver),
                crate::adaptor_gateway_workflow_workflow_host_test_helpers::workspace_query(store.clone()),
                Arc::new(OrderedWorkflowAgentSessions {
                    calls: calls.clone(),
                }),
                Arc::new(crate::adaptor_gateway_workflow_workflow_host_test_helpers::TestWorktrees::default()),
                releash_lib::test_support::integration::daemon::serving(),
            ));
            let session_node = |name: &str| NodeDefinition {
                name: name.to_string(),
                kind: NodeKind::Session(SessionSpec {
                    provider: ProviderKind::Codex,
                    model: None,
                    permission: None,
                    facets: FacetRefs {
                        instruction: Some("policy-confirmation".to_string()),
                        ..FacetRefs::default()
                    },
                }),
                artifact: None,
                input: Vec::new(),
                completion: NodeCompletion::default(),
                worktree: None,
            };
            let workflow = WorkflowDefinition {
                name: "runtime-effect-order-test".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes: vec![
                    NodeDefinition {
                        name: "main".to_string(),
                        kind: NodeKind::Sequence(SequenceSpec {
                            entry: None,
                            children: vec![
                                ChildEntry::reference("agent-one"),
                                ChildEntry::reference("agent-two"),
                            ],
                        }),
                        artifact: None,
                        input: Vec::new(),
                        completion: NodeCompletion::default(),
                        worktree: None,
                    },
                    session_node("agent-one"),
                    session_node("agent-two"),
                ],
                entry: "main".to_string(),
            };
            let execution_id = host
                .start_resolved_workflow(
                    &app,
                    workflow,
                    EFFECT_WORKTREE_PATH.to_string(),
                    None,
                    ExecutionOrigin::DesktopUi,
                )
                .await
                .unwrap();
            let snapshot = host
                .get_state_by_execution_id(&app, &execution_id)
                .await
                .unwrap();
            let first = snapshot
                .node_executions
                .iter()
                .find(|node| node.node_name == "agent-one")
                .unwrap();
            assert_eq!(first.status, NodeExecutionStatus::Running);
            let first_node_execution_id = first.id.clone();
            let first_agent_session_id = first.session_id.clone().unwrap();
            let gateway = Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
                app.clone(),
                host.clone(),
            ));
            let control_plane = WorkflowControlPlaneUsecase::new(
                releash_lib::test_support::integration::platform::shared().clone(),
                gateway,
            );
            SequentialRuntimeEffectFixture {
                _app: app,
                store,
                host,
                control_plane,
                calls,
                execution_id,
                first_node_execution_id,
                first_agent_session_id,
                _directory: directory,
            }
        }

        fn provider_stop_command(
            fixture: &RuntimeEffectFixture,
        ) -> ProviderExecutionTreeStopCommand {
            ProviderExecutionTreeStopCommand {
                agent_session_id: EFFECT_AGENT_SESSION_ID.to_string(),
                tree_id: fixture.execution_id.clone(),
                node_execution_id: fixture.node_execution_id.clone(),
                binding_id: "binding-effect-test".to_string(),
            }
        }

        async fn persisted_node_status(fixture: &RuntimeEffectFixture) -> NodeExecutionStatus {
            persisted_node(fixture).await.status
        }

        async fn persisted_node(
            fixture: &RuntimeEffectFixture,
        ) -> releash_lib::test_support::integration::workflow::RuntimeNodeExecution {
            persisted_node_for(
                &fixture.store,
                &fixture.execution_id,
                &fixture.node_execution_id,
            )
            .await
        }

        async fn persisted_node_for(
            store: &Arc<LocalEventStore>,
            execution_id: &str,
            node_execution_id: &str,
        ) -> releash_lib::test_support::integration::workflow::RuntimeNodeExecution {
            let backend =
                releash_lib::test_support::integration::workflow::FactLogReadBackend::Live(
                    store.clone(),
                );
            releash_lib::test_support::integration::workflow::fold_tree_from(&backend, execution_id)
                .await
                .unwrap()
                .unwrap()
                .aggregate
                .node_executions
                .iter()
                .find(|node| node.id == node_execution_id)
                .unwrap()
                .clone()
        }

        #[tokio::test]
        pub async fn test_deleted実行木解放_facet本文を除去する() {
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            assert!(fixture
                .host
                .test_execution_facet_contents()
                .lock()
                .await
                .contains_key(&fixture.execution_id));
            fixture
                .host
                .release_deleted_execution_tree(&fixture.execution_id)
                .await
                .unwrap();
            assert!(!fixture
                .host
                .test_execution_facet_contents()
                .lock()
                .await
                .contains_key(&fixture.execution_id));
        }

        #[tokio::test]
        pub async fn test_started実行木登録_store未管理ならsession_storeを返す() {
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            let unmanaged_app =
                crate::adaptor_gateway_workflow_workflow_host_test_helpers::dependencies(None);

            let error = fixture
                .host
                .register_started_execution_tree(&unmanaged_app, "unmanaged-tree")
                .await
                .unwrap_err();

            assert!(matches!(error, WorkflowRuntimeError::SessionStore(_)));
        }

        #[tokio::test]
        pub async fn test_started実行木登録_tree不在ならexecution_not_foundを返す() {
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            let missing_tree_id = "missing-started-tree";

            let error = fixture
                .host
                .register_started_execution_tree(&fixture.app, missing_tree_id)
                .await
                .unwrap_err();

            assert!(matches!(
                error,
                WorkflowRuntimeError::ExecutionNotFound(tree_id) if tree_id == missing_tree_id
            ));
        }

        #[tokio::test]
        pub async fn test_started実行木登録_inactive_treeならinvalid_stateを返す() {
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            let session_id = "inactive-started-tree";
            LocalAgentSessionRepository::new(fixture.store.clone())
                .create(
                    AgentSession::create(
                        session_id,
                        WorkspaceIdentity::new(EFFECT_WORKTREE_PATH),
                        EFFECT_WORKTREE_PATH,
                        ProviderKind::Codex,
                        AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
                    )
                    .unwrap(),
                    "create-inactive-started-tree",
                )
                .await
                .unwrap();
            releash_lib::test_support::integration::workflow::append_facts_for_events(
                &fixture.store,
                &[WorkflowEvent::ExecutionAborted {
                    execution_id: session_id.to_string(),
                    aborted_node: None,
                    timestamp: 2.0,
                }],
            )
            .await
            .unwrap();

            let error = fixture
                .host
                .register_started_execution_tree(&fixture.app, session_id)
                .await
                .unwrap_err();

            assert!(matches!(error, WorkflowRuntimeError::InvalidState(_)));
        }

        #[tokio::test]
        pub async fn test_session実行木のreconciliationは完了済みnodeに喪失を記録せずstopを記録する(
        ) {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            let session_id = "agent-session-reserved-before-commit";
            LocalAgentSessionRepository::new(fixture.store.clone())
                .create(
                    AgentSession::create(
                        session_id,
                        WorkspaceIdentity::new(EFFECT_WORKTREE_PATH),
                        EFFECT_WORKTREE_PATH,
                        ProviderKind::Codex,
                        AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
                    )
                    .unwrap(),
                    "create-reserved-before-commit",
                )
                .await
                .unwrap();

            crate::adaptor_gateway_workflow_workflow_host_test_helpers::reconcile_startup(
                &fixture.host,
                &fixture.app,
            )
            .await
            .unwrap();

            let records = releash_lib::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                session_id,
            )
            .await
            .unwrap();
            assert!(!records
                .iter()
                .any(|record| matches!(record.fact, NodeFact::ProcessExited(_))));
            fixture
                .host
                .register_started_execution_tree(&fixture.app, session_id)
                .await
                .unwrap();

            // When
            fixture
                .control_plane
                .record_provider_stop(
                    ProviderExecutionTreeStopCommand {
                        agent_session_id: session_id.to_string(),
                        tree_id: session_id.to_string(),
                        node_execution_id: session_id.to_string(),
                        binding_id: "binding-reserved-before-commit".to_string(),
                    },
                    Vec::new(),
                )
                .await
                .unwrap();

            let records = releash_lib::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                session_id,
            )
            .await
            .unwrap();
            // Then
            assert!(records
                .iter()
                .any(|record| matches!(record.fact, NodeFact::StopReceived(_))));
            let backend =
                releash_lib::test_support::integration::workflow::FactLogReadBackend::Live(
                    fixture.store.clone(),
                );
            let folded = releash_lib::test_support::integration::workflow::fold_tree_from(
                &backend, session_id,
            )
            .await
            .unwrap()
            .unwrap();
            let node = folded
                .aggregate
                .node_executions
                .iter()
                .find(|node| node.id == session_id)
                .unwrap();
            assert_eq!(
                node.completion_signals,
                releash_lib::test_support::integration::workflow::NodeCompletionSignalState::Pending
            );
            assert_eq!(node.status, NodeExecutionStatus::Succeeded);
            assert_eq!(
                folded.session_activities[session_id],
                releash_lib::test_support::integration::workflow::AgentSessionActivity::AwaitingInstruction
            );
            let workspace_node = SqliteWorkspaceTreeRepository::new(fixture.store.clone())
                .load_node_by_node_execution_id(session_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                workspace_node.status_classification,
                WorkspaceNodeStatusClassification::Idle
            );

            let restarted = WorkflowRuntimeHost::with_runtime_ports(
                releash_lib::test_support::integration::platform::shared().clone(),
                Arc::new(UnusedWorkflowResolver),
                Arc::new(UnusedWorktreeResolver),
                crate::adaptor_gateway_workflow_workflow_host_test_helpers::workspace_query(fixture.store.clone()),
                Arc::new(FailingWorkflowAgentSessions),
                Arc::new(releash_lib::test_support::integration::workflow::RepositoryIsolatedWorktreeGateway),
                releash_lib::test_support::integration::daemon::serving(),
            );
            crate::adaptor_gateway_workflow_workflow_host_test_helpers::reconcile_startup(
                &restarted,
                &fixture.app,
            )
            .await
            .unwrap();

            let restarted_fold = releash_lib::test_support::integration::workflow::fold_tree_from(
                &backend, session_id,
            )
            .await
            .unwrap()
            .unwrap();
            let restarted_node = restarted_fold
                .aggregate
                .node_executions
                .iter()
                .find(|node| node.id == session_id)
                .unwrap();
            assert_eq!(
                restarted_node.completion_signals,
                releash_lib::test_support::integration::workflow::NodeCompletionSignalState::Pending
            );
            assert_eq!(
                restarted_fold.session_activities[session_id],
                releash_lib::test_support::integration::workflow::AgentSessionActivity::AwaitingInstruction
            );
            assert_eq!(
                SqliteWorkspaceTreeRepository::new(fixture.store.clone())
                    .load_node_by_node_execution_id(session_id)
                    .await
                    .unwrap()
                    .unwrap()
                    .status_classification,
                WorkspaceNodeStatusClassification::Idle
            );
            assert!(
                !releash_lib::test_support::integration::workflow::read_tree_records(
                    &fixture.store,
                    session_id
                )
                .await
                .unwrap()
                .iter()
                .any(|record| matches!(record.fact, NodeFact::ProcessExited(_)))
            );
        }

        #[tokio::test]
        pub async fn test_session実行木登録失敗後もreconciliationはプロセス喪失を記録しない() {
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            let session_id = "agent-session-registration-failed";
            LocalAgentSessionRepository::new(fixture.store.clone())
                .create(
                    AgentSession::create(
                        session_id,
                        WorkspaceIdentity::new(EFFECT_WORKTREE_PATH),
                        EFFECT_WORKTREE_PATH,
                        ProviderKind::Codex,
                        AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
                    )
                    .unwrap(),
                    "create-registration-failed",
                )
                .await
                .unwrap();
            let unmanaged_app =
                crate::adaptor_gateway_workflow_workflow_host_test_helpers::dependencies(None);
            assert!(fixture
                .host
                .register_started_execution_tree(&unmanaged_app, session_id)
                .await
                .is_err());

            crate::adaptor_gateway_workflow_workflow_host_test_helpers::reconcile_startup(
                &fixture.host,
                &fixture.app,
            )
            .await
            .unwrap();

            let records = releash_lib::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                session_id,
            )
            .await
            .unwrap();
            assert!(!records
                .iter()
                .any(|record| matches!(record.fact, NodeFact::ProcessExited(_))));
        }

        #[tokio::test]
        pub async fn test_session起動_provider起動時にはattach済みでstop_receivedになる() {
            // Given
            let directory = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
                std::sync::Arc::new(
                    releash_lib::test_support::integration::platform::RetryLimiter::new(),
                ),
            ))
            .unwrap();
            let app = crate::adaptor_gateway_workflow_workflow_host_test_helpers::dependencies(
                Some(store.clone()),
            );
            let sessions = Arc::new(StopDuringActivationWorkflowAgentSessions {
                control_plane: tokio::sync::Mutex::new(None),
                execution_id: std::sync::Mutex::new(None),
                activation_count: std::sync::atomic::AtomicUsize::new(0),
                confirmation_count: std::sync::atomic::AtomicUsize::new(0),
            });
            let host = Arc::new(WorkflowRuntimeHost::with_runtime_ports(
                releash_lib::test_support::integration::platform::shared().clone(),
                Arc::new(UnusedWorkflowResolver),
                Arc::new(AcceptingWorktreeResolver),
                crate::adaptor_gateway_workflow_workflow_host_test_helpers::workspace_query(store.clone()),
                sessions.clone(),
                Arc::new(crate::adaptor_gateway_workflow_workflow_host_test_helpers::TestWorktrees::default()),
                releash_lib::test_support::integration::daemon::serving(),
            ));
            let gateway = Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
                app.clone(),
                host.clone(),
            ));
            *sessions.control_plane.lock().await =
                Some(Arc::new(WorkflowControlPlaneUsecase::new(
                    releash_lib::test_support::integration::platform::shared().clone(),
                    gateway,
                )));
            let workflow = WorkflowDefinition {
                name: "stop-during-activation".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes: vec![NodeDefinition {
                    name: "main".to_string(),
                    kind: NodeKind::Session(SessionSpec {
                        provider: ProviderKind::Codex,
                        model: None,
                        permission: None,
                        facets: FacetRefs {
                            instruction: Some("policy-confirmation".to_string()),
                            ..FacetRefs::default()
                        },
                    }),
                    artifact: None,
                    input: Vec::new(),
                    completion: NodeCompletion::default(),
                    worktree: None,
                }],
                entry: "main".to_string(),
            };

            // When
            let execution_id = host
                .start_resolved_workflow(
                    &app,
                    workflow,
                    EFFECT_WORKTREE_PATH.to_string(),
                    None,
                    ExecutionOrigin::DesktopUi,
                )
                .await
                .unwrap();

            // Then
            let snapshot = host
                .get_state_by_execution_id(&app, &execution_id)
                .await
                .unwrap();
            let node = snapshot
                .node_executions
                .iter()
                .find(|node| node.node_name == "main")
                .unwrap();
            assert_eq!(
                node.status,
                NodeExecutionStatus::Running,
                "unexpected activation state"
            );
            assert_eq!(
                node.completion_signals,
                releash_lib::test_support::integration::workflow::NodeCompletionSignalState::StopReceived
            );
            assert_eq!(node.session_id.as_deref(), Some(EFFECT_AGENT_SESSION_ID));
            assert_eq!(
                sessions
                    .confirmation_count
                    .load(std::sync::atomic::Ordering::SeqCst),
                1
            );
            let records = releash_lib::test_support::integration::workflow::read_tree_records(
                &store,
                &execution_id,
            )
            .await
            .unwrap();
            let attached_seq = records
                .iter()
                .find_map(|record| match &record.fact {
                    NodeFact::SessionAttached(attached)
                        if attached.session_id == EFFECT_AGENT_SESSION_ID =>
                    {
                        Some(record.seq)
                    }
                    _ => None,
                })
                .unwrap();
            let stop_seq = records
                .iter()
                .find_map(|record| {
                    matches!(record.fact, NodeFact::StopReceived(_)).then_some(record.seq)
                })
                .unwrap();
            assert!(attached_seq < stop_seq);
        }

        #[tokio::test]
        pub async fn test_provider_stop_完了済み単独sessionと実行中workflowでnode完了信号を区別する(
        ) {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            let standalone_id = "agent-session-standalone-stop";
            LocalAgentSessionRepository::new(fixture.store.clone())
                .create(
                    AgentSession::create(
                        standalone_id,
                        WorkspaceIdentity::new(EFFECT_WORKTREE_PATH),
                        EFFECT_WORKTREE_PATH,
                        ProviderKind::Codex,
                        AgentSessionTreeLocation::session_tree_root(standalone_id).unwrap(),
                    )
                    .unwrap(),
                    "create-standalone-stop",
                )
                .await
                .unwrap();
            fixture
                .host
                .register_started_execution_tree(&fixture.app, standalone_id)
                .await
                .unwrap();

            // When
            fixture
                .control_plane
                .record_provider_stop(
                    ProviderExecutionTreeStopCommand {
                        agent_session_id: standalone_id.to_string(),
                        tree_id: standalone_id.to_string(),
                        node_execution_id: standalone_id.to_string(),
                        binding_id: "binding-standalone-stop".to_string(),
                    },
                    Vec::new(),
                )
                .await
                .unwrap();
            fixture
                .control_plane
                .record_provider_stop(provider_stop_command(&fixture), Vec::new())
                .await
                .unwrap();

            // Then
            let backend =
                releash_lib::test_support::integration::workflow::FactLogReadBackend::Live(
                    fixture.store.clone(),
                );
            for (tree_id, node_execution_id) in [
                (standalone_id, standalone_id),
                (
                    fixture.execution_id.as_str(),
                    fixture.node_execution_id.as_str(),
                ),
            ] {
                let folded = releash_lib::test_support::integration::workflow::fold_tree_from(
                    &backend, tree_id,
                )
                .await
                .unwrap()
                .unwrap();
                let node = folded
                    .aggregate
                    .node_executions
                    .iter()
                    .find(|node| node.id == node_execution_id)
                    .unwrap();
                assert_eq!(
                    node.completion_signals,
                    if tree_id == standalone_id {
                        releash_lib::test_support::integration::workflow::NodeCompletionSignalState::Pending
                    } else {
                        releash_lib::test_support::integration::workflow::NodeCompletionSignalState::StopReceived
                    }
                );
                assert_eq!(
                    node.status,
                    if tree_id == standalone_id {
                        NodeExecutionStatus::Succeeded
                    } else {
                        NodeExecutionStatus::Running
                    }
                );
            }
        }

        #[tokio::test]
        pub async fn test_session起動由来のactive木と同一worktreeでworkflowを起動できる() {
            // Given: 同じ worktree に active な Session 起動由来の木が登録されている
            let directory = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
                std::sync::Arc::new(
                    releash_lib::test_support::integration::platform::RetryLimiter::new(),
                ),
            ))
            .unwrap();
            let app = crate::adaptor_gateway_workflow_workflow_host_test_helpers::dependencies(
                Some(store.clone()),
            );
            let session_id = "agent-session-workflow-coexistence";
            LocalAgentSessionRepository::new(store.clone())
                .create(
                    AgentSession::create(
                        session_id,
                        WorkspaceIdentity::new(EFFECT_WORKTREE_PATH),
                        EFFECT_WORKTREE_PATH,
                        ProviderKind::Codex,
                        AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
                    )
                    .unwrap(),
                    "create-session-workflow-coexistence",
                )
                .await
                .unwrap();
            let host = Arc::new(WorkflowRuntimeHost::with_runtime_ports(
                releash_lib::test_support::integration::platform::shared().clone(),
                Arc::new(UnusedWorkflowResolver),
                Arc::new(AcceptingWorktreeResolver),
                crate::adaptor_gateway_workflow_workflow_host_test_helpers::workspace_query(store.clone()),
                Arc::new(RecordingWorkflowAgentSessions {
                    stop_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
                    prepare_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
                    provider_running_checks: Arc::new(std::sync::Mutex::new(Vec::new())),
                    recovery_fails: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    failing_agent_session_id: String::new(),
                }),
                Arc::new(crate::adaptor_gateway_workflow_workflow_host_test_helpers::TestWorktrees::default()),
                releash_lib::test_support::integration::daemon::serving(),
            ));
            host.register_started_execution_tree(&app, session_id)
                .await
                .unwrap();
            let workflow = WorkflowDefinition {
                name: "coexisting-workflow".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes: vec![NodeDefinition {
                    name: "main".to_string(),
                    kind: NodeKind::Session(SessionSpec {
                        provider: ProviderKind::Codex,
                        model: None,
                        permission: None,
                        facets: FacetRefs {
                            instruction: Some("policy-confirmation".to_string()),
                            ..FacetRefs::default()
                        },
                    }),
                    artifact: None,
                    input: Vec::new(),
                    completion: NodeCompletion::default(),
                    worktree: None,
                }],
                entry: "main".to_string(),
            };

            // When: workflow の実行として同じ worktree に木を起こす
            let workflow_id = host
                .start_resolved_workflow(
                    &app,
                    workflow.clone(),
                    EFFECT_WORKTREE_PATH.to_string(),
                    None,
                    ExecutionOrigin::DesktopUi,
                )
                .await
                .unwrap();

            // Then: cache は両方を保持し、workflow registry は workflow だけを保持する
            assert!(host
                .get_state_by_execution_id(&app, session_id)
                .await
                .is_some());
            assert!(host
                .get_state_by_execution_id(&app, &workflow_id)
                .await
                .is_some());
            let second = host
                .start_resolved_workflow(
                    &app,
                    workflow,
                    EFFECT_WORKTREE_PATH.to_string(),
                    None,
                    ExecutionOrigin::DesktopUi,
                )
                .await;
            assert!(matches!(
                second,
                Err(WorkflowRuntimeError::AlreadyActive(_))
            ));
        }

        async fn wait_for_single_terminal_stop(fixture: &RuntimeEffectFixture) {
            let observed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if !fixture.stop_calls.lock().unwrap().is_empty() {
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await;
            assert!(
                observed.is_ok(),
                "terminal stop effect must run after durable commit"
            );
            assert_eq!(
                fixture.stop_calls.lock().unwrap().as_slice(),
                &[(
                    fixture.node_execution_id.clone(),
                    EFFECT_AGENT_SESSION_ID.to_string(),
                )]
            );
        }

        #[tokio::test]
        pub async fn test_provider_stop_provider_lifecycle_commit失敗後も停止effectを実行する() {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            fixture
                .control_plane
                .submit_output(SubmitOutputCommand {
                    node_execution_id: fixture.node_execution_id.clone(),
                    artifact: None,
                })
                .await
                .unwrap();
            fixture.fault.arm_fail_before_commit();
            let scope = ProviderLifecycleScope::new(EFFECT_AGENT_SESSION_ID).unwrap();
            let lifecycle_events = vec![ScopedProviderLifecycleEvent::new(
                scope,
                ProviderLifecycleEvent::stop_observed("binding-effect-test").unwrap(),
            )];

            // When
            let result = fixture
                .control_plane
                .record_provider_stop(provider_stop_command(&fixture), lifecycle_events)
                .await;

            // Then
            assert!(result.is_ok(), "unexpected provider Stop error: {result:?}");
            assert_eq!(
                persisted_node_status(&fixture).await,
                NodeExecutionStatus::Succeeded
            );
            wait_for_single_terminal_stop(&fixture).await;
            let page = fixture
                .store
                .load_stream(LoadStreamRequest {
                    stream_id: StreamId::provider_lifecycle(EFFECT_AGENT_SESSION_ID).unwrap(),
                    after: None,
                    limit: 10,
                })
                .await
                .unwrap();
            assert!(page.events.is_empty());
        }

        #[tokio::test]
        pub async fn test_submit_agent_session停止失敗でも成功とsucceededを維持する() {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::default(), true).await;
            fixture
                .control_plane
                .record_provider_stop(provider_stop_command(&fixture), Vec::new())
                .await
                .unwrap();

            // When
            let result = fixture
                .control_plane
                .submit_output(SubmitOutputCommand {
                    node_execution_id: fixture.node_execution_id.clone(),
                    artifact: None,
                })
                .await;

            // Then
            assert!(result.is_ok(), "unexpected Submit error: {result:?}");
            assert_eq!(
                persisted_node_status(&fixture).await,
                NodeExecutionStatus::Succeeded
            );
            wait_for_single_terminal_stop(&fixture).await;
        }

        #[tokio::test]
        pub async fn test_session終端_後続activateを停止完了に依存させず両方を実行する() {
            // Given
            let fixture = sequential_runtime_effect_fixture().await;
            fixture.calls.lock().unwrap().clear();
            fixture
                .control_plane
                .record_provider_stop(
                    ProviderExecutionTreeStopCommand {
                        agent_session_id: fixture.first_agent_session_id.clone(),
                        tree_id: fixture.execution_id.clone(),
                        node_execution_id: fixture.first_node_execution_id.clone(),
                        binding_id: "binding-order-test".to_string(),
                    },
                    Vec::new(),
                )
                .await
                .unwrap();
            assert!(fixture.calls.lock().unwrap().is_empty());

            // When
            fixture
                .control_plane
                .submit_output(SubmitOutputCommand {
                    node_execution_id: fixture.first_node_execution_id.clone(),
                    artifact: None,
                })
                .await
                .unwrap();

            // Then
            let snapshot = fixture
                .host
                .get_state_by_execution_id(&fixture._app, &fixture.execution_id)
                .await
                .unwrap();
            let second = snapshot
                .node_executions
                .iter()
                .find(|node| node.node_name == "agent-two")
                .unwrap();
            assert_eq!(second.status, NodeExecutionStatus::Running);
            let activate = RuntimeEffectCall::Activate {
                node_execution_id: second.id.clone(),
                agent_session_id: second.session_id.clone().unwrap(),
            };
            assert!(
                fixture.calls.lock().unwrap().contains(&activate),
                "Submit acceptance must activate the next Session without waiting for the stop effect"
            );
            let expected_stop = RuntimeEffectCall::Stop {
                node_execution_id: fixture.first_node_execution_id.clone(),
                agent_session_id: fixture.first_agent_session_id.clone(),
            };
            let observed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if fixture.calls.lock().unwrap().contains(&expected_stop) {
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await;
            assert!(
                observed.is_ok(),
                "terminal stop effect must run after durable commit"
            );
        }

        #[tokio::test]
        pub async fn test_provider_stop受理_停止effect未完了でもcommitと後続処理が完了する() {
            // Given
            let fixture = runtime_effect_fixture_with_sessions(
                NodeCompletion::default(),
                Arc::new(NeverResolvingStopWorkflowAgentSessions),
                Arc::new(std::sync::Mutex::new(Vec::new())),
            )
            .await;
            fixture
                .control_plane
                .submit_output(SubmitOutputCommand {
                    node_execution_id: fixture.node_execution_id.clone(),
                    artifact: None,
                })
                .await
                .unwrap();

            // When
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                fixture
                    .control_plane
                    .record_provider_stop(provider_stop_command(&fixture), Vec::new()),
            )
            .await;

            // Then
            let result =
                result.expect("provider Stop acceptance must not block on the session stop effect");
            assert!(result.is_ok(), "unexpected provider Stop error: {result:?}");
            assert_eq!(
                persisted_node_status(&fixture).await,
                NodeExecutionStatus::Succeeded
            );
        }

        #[tokio::test]
        pub async fn test_終端済みsessionへの再stop_確定状態と停止回数を変えない() {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            fixture
                .control_plane
                .record_provider_stop(provider_stop_command(&fixture), Vec::new())
                .await
                .unwrap();
            fixture
                .control_plane
                .submit_output(SubmitOutputCommand {
                    node_execution_id: fixture.node_execution_id.clone(),
                    artifact: None,
                })
                .await
                .unwrap();
            assert_eq!(
                persisted_node_status(&fixture).await,
                NodeExecutionStatus::Succeeded
            );
            wait_for_single_terminal_stop(&fixture).await;

            // When
            let result = fixture
                .control_plane
                .record_provider_stop(provider_stop_command(&fixture), Vec::new())
                .await;

            // Then
            assert!(result.is_ok(), "unexpected repeated Stop error: {result:?}");
            assert_eq!(
                persisted_node_status(&fixture).await,
                NodeExecutionStatus::Succeeded
            );
            wait_for_single_terminal_stop(&fixture).await;
        }

        #[tokio::test]
        pub async fn test_承認_agent_session停止失敗でも成功とsucceededを維持する() {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::require_approval(), true).await;
            fixture
                .control_plane
                .submit_output(SubmitOutputCommand {
                    node_execution_id: fixture.node_execution_id.clone(),
                    artifact: None,
                })
                .await
                .unwrap();
            fixture
                .control_plane
                .record_provider_stop(provider_stop_command(&fixture), Vec::new())
                .await
                .unwrap();
            let waiting = fixture
                .host
                .get_state_by_execution_id(&fixture.app, &fixture.execution_id)
                .await
                .unwrap();
            assert_eq!(
                waiting
                    .node_executions
                    .iter()
                    .find(|node| node.id == fixture.node_execution_id)
                    .unwrap()
                    .status,
                NodeExecutionStatus::WaitingApproval
            );

            // When
            let result = fixture
                .control_plane
                .resolve_approval(ApprovalCommand {
                    execution_id: fixture.execution_id.clone(),
                    node_name: EFFECT_NODE_NAME.to_string(),
                    node_execution_id: Some(fixture.node_execution_id.clone()),
                    comment: None,
                })
                .await;

            // Then
            assert!(result.is_ok(), "unexpected approval error: {result:?}");
            assert_eq!(
                persisted_node_status(&fixture).await,
                NodeExecutionStatus::Succeeded
            );
            wait_for_single_terminal_stop(&fixture).await;
        }

        #[tokio::test]
        pub async fn test_失敗確定_版の競合では事実とnodeの状態を変更しない() {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            let before = releash_lib::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                &fixture.execution_id,
            )
            .await
            .unwrap()
            .len();
            let runtime_error = WorkflowRuntimeError::Conflict("version conflict".into());

            // When
            let result = fixture
                .host
                .settle_runtime_failure_for_node(
                    &fixture.app,
                    &fixture.execution_id,
                    &fixture.node_execution_id,
                    &runtime_error,
                )
                .await;

            // Then
            assert!(matches!(
                result,
                Err(WorkflowRuntimeError::Conflict(reason)) if reason == "version conflict"
            ));
            assert_eq!(
                releash_lib::test_support::integration::workflow::read_tree_records(
                    &fixture.store,
                    &fixture.execution_id
                )
                .await
                .unwrap()
                .len(),
                before
            );
            assert_eq!(
                persisted_node_status(&fixture).await,
                NodeExecutionStatus::Running
            );
            assert!(fixture.stop_calls.lock().unwrap().is_empty());
        }

        #[tokio::test]
        pub async fn test_failure_settlement_異常の記録はsessionをrunningのまま維持する() {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::default(), true).await;
            let runtime_error = WorkflowRuntimeError::AgentSession("runtime failed".to_string());

            // When
            let result = fixture
                .host
                .settle_runtime_failure_for_node(
                    &fixture.app,
                    &fixture.execution_id,
                    &fixture.node_execution_id,
                    &runtime_error,
                )
                .await;

            // Then
            assert!(
                result.is_ok(),
                "unexpected failure settlement error: {result:?}"
            );
            let settled = fixture
                .host
                .get_state_by_execution_id(&fixture.app, &fixture.execution_id)
                .await
                .unwrap();
            assert_eq!(
                settled
                    .node_executions
                    .iter()
                    .find(|node| node.id == fixture.node_execution_id)
                    .unwrap()
                    .status,
                NodeExecutionStatus::Running
            );
            assert_eq!(fixture.stop_calls.lock().unwrap().len(), 0);
        }

        #[tokio::test]
        pub async fn test_abort_agent_session停止失敗でも成功とabortedを維持する() {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::default(), true).await;

            // When
            let result = fixture
                .host
                .abort_workflow_execution(&fixture.app, &fixture.execution_id, None)
                .await;

            // Then
            assert!(result.is_ok(), "unexpected abort error: {result:?}");
            assert_eq!(
                persisted_node_status(&fixture).await,
                NodeExecutionStatus::Aborted
            );
            wait_for_single_terminal_stop(&fixture).await;
        }
    }

    pub(crate) mod startup_recovery_tests {
        use super::*;

        #[tokio::test]
        pub async fn test_startup_reconciliation_stop事実がある完了済みsession木でもleafを再起動しない(
        ) {
            // Given
            let directory = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
                std::sync::Arc::new(
                    releash_lib::test_support::integration::platform::RetryLimiter::new(),
                ),
            ))
            .unwrap();
            let session_id = "agent-session-startup";
            LocalAgentSessionRepository::new(store.clone())
                .create(
                    AgentSession::create(
                        session_id,
                        WorkspaceIdentity::new("/repo/session-startup"),
                        "/repo/session-startup",
                        ProviderKind::Codex,
                        AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
                    )
                    .unwrap(),
                    "create-startup-session",
                )
                .await
                .unwrap();
            releash_lib::test_support::integration::workflow::append_facts_for_events(
                &store,
                &[WorkflowEvent::NodeStopReceived {
                    execution_id: session_id.to_string(),
                    node_execution_id: session_id.to_string(),
                    timestamp: 2.0,
                }],
            )
            .await
            .unwrap();
            let before = releash_lib::test_support::integration::workflow::read_tree_records(
                &store, session_id,
            )
            .await
            .unwrap()
            .len();
            let app = crate::adaptor_gateway_workflow_workflow_host_test_helpers::dependencies(
                Some(store.clone()),
            );
            let host = WorkflowRuntimeHost::with_runtime_ports(
                releash_lib::test_support::integration::platform::shared().clone(),
                Arc::new(UnusedWorkflowResolver),
                Arc::new(UnusedWorktreeResolver),
                crate::adaptor_gateway_workflow_workflow_host_test_helpers::workspace_query(store.clone()),
                Arc::new(FailingWorkflowAgentSessions),
                Arc::new(releash_lib::test_support::integration::workflow::RepositoryIsolatedWorktreeGateway),
                releash_lib::test_support::integration::daemon::serving(),
            );

            // When
            crate::adaptor_gateway_workflow_workflow_host_test_helpers::reconcile_startup(
                &host, &app,
            )
            .await
            .unwrap();

            let snapshot = host
                .get_state_by_execution_id(&app, session_id)
                .await
                .unwrap();
            let node = snapshot
                .node_executions
                .iter()
                .find(|node| node.id == session_id)
                .unwrap();
            // Then
            assert_eq!(
                node.completion_signals,
                releash_lib::test_support::integration::workflow::NodeCompletionSignalState::Pending
            );
            assert_eq!(
                releash_lib::test_support::integration::workflow::read_tree_records(
                    &store, session_id
                )
                .await
                .unwrap()
                .len(),
                before
            );
        }

        async fn append_started_session_tree(
            store: &Arc<LocalEventStore>,
            tree_id: &str,
            worktree_path: &str,
            timestamp_ms: i64,
        ) {
            let definition = WorkflowDefinition {
                name: format!("workflow-{tree_id}"),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes: vec![
                    NodeDefinition {
                        name: "main".to_string(),
                        kind: NodeKind::Sequence(SequenceSpec {
                            entry: None,
                            children: vec![ChildEntry::reference("impl")],
                        }),
                        artifact: None,
                        input: Vec::new(),
                        completion: releash_lib::test_support::integration::workflow::NodeCompletion::default(),
                        worktree: None,
                    },
                    NodeDefinition {
                        name: "impl".to_string(),
                        kind: NodeKind::Session(SessionSpec {
                            provider: ProviderKind::Codex,
                            model: None,
                            permission: None,
                            facets: Default::default(),
                        }),
                        artifact: None,
                        input: Vec::new(),
                        completion: releash_lib::test_support::integration::workflow::NodeCompletion::default(),
                        worktree: None,
                    },
                ],
                entry: "main".to_string(),
            };
            let root_meta = NodeFactMeta {
                tree_id: tree_id.to_string(),
                node_execution_id: tree_id.to_string(),
                parent_id: None,
                node_name: "main".to_string(),
                kind: NodeKindName::Sequence,
                attempt: 1,
            };
            releash_lib::test_support::integration::workflow::append_single_fact(
                store,
                &root_meta,
                &NodeFact::Started(StartedFact {
                    worktree: None,
                    parent: None,
                    root: Some(Box::new(TreeRootFact {
                        repository_root: None,
                        workspace_identity: worktree_path.to_string(),
                        worktree_path: worktree_path.to_string(),
                        created_from: ExecutionOrigin::DesktopUi,
                        request: String::new(),
                        workflow_name: definition.name.clone(),
                        definition: Some(definition),
                        launched_as: ExecutionTreeLaunch::Workflow,
                    })),
                }),
                timestamp_ms,
            )
            .await
            .unwrap();
            let child_meta = NodeFactMeta {
                tree_id: tree_id.to_string(),
                node_execution_id: format!("{tree_id}-session"),
                parent_id: Some(tree_id.to_string()),
                node_name: "impl".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
            };
            releash_lib::test_support::integration::workflow::append_single_fact(
                store,
                &child_meta,
                &NodeFact::Started(StartedFact {
                    worktree: None,
                    parent: Some(ExecutionParentRef::sequence_child(tree_id)),
                    root: None,
                }),
                timestamp_ms + 1,
            )
            .await
            .unwrap();
        }

        #[tokio::test]
        pub async fn test_startup_reconciliation_壊れたtreeの後続treeも処理する() {
            const CORRUPT_TREE_ID: &str = "00000000-0000-4000-8000-000000000001";
            const VALID_TREE_ID: &str = "00000000-0000-4000-8000-000000000002";

            let directory = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
                std::sync::Arc::new(
                    releash_lib::test_support::integration::platform::RetryLimiter::new(),
                ),
            ))
            .unwrap();
            append_started_session_tree(&store, CORRUPT_TREE_ID, "/repo/corrupt", 1).await;
            store
                .append_node_event(
                    NewNodeEventRow {
                        tree_id: CORRUPT_TREE_ID.to_string(),
                        node_execution_id: CORRUPT_TREE_ID.to_string(),
                        parent_id: None,
                        node_name: "main".to_string(),
                        kind: "session".to_string(),
                        attempt: 1,
                        event_type: "submit_received".to_string(),
                        session_id: None,
                        detail: "{".to_string(),
                    },
                    Some(4),
                )
                .await
                .unwrap();
            append_started_session_tree(&store, VALID_TREE_ID, "/repo/valid", 5).await;
            let valid_records =
                releash_lib::test_support::integration::workflow::read_tree_records(
                    &store,
                    VALID_TREE_ID,
                )
                .await
                .unwrap();
            releash_lib::test_support::integration::workflow::append_facts_for_events(
                &store,
                &[WorkflowEvent::SessionAttached {
                    execution_id: VALID_TREE_ID.into(),
                    node_execution_id: valid_records[1].meta.node_execution_id.clone(),
                    session_id: "already-started".into(),
                    timestamp: 0.006,
                }],
            )
            .await
            .unwrap();
            let corrupt_count =
                releash_lib::test_support::integration::workflow::read_tree_records(
                    &store,
                    CORRUPT_TREE_ID,
                )
                .await
                .unwrap_err();
            assert!(corrupt_count.to_string().contains("decode"));
            let valid_count = releash_lib::test_support::integration::workflow::read_tree_records(
                &store,
                VALID_TREE_ID,
            )
            .await
            .unwrap()
            .len();

            let app = crate::adaptor_gateway_workflow_workflow_host_test_helpers::dependencies(
                Some(store.clone()),
            );
            let host = WorkflowRuntimeHost::with_runtime_ports(
                releash_lib::test_support::integration::platform::shared().clone(),
                Arc::new(UnusedWorkflowResolver),
                Arc::new(UnusedWorktreeResolver),
                crate::adaptor_gateway_workflow_workflow_host_test_helpers::workspace_query(store.clone()),
                Arc::new(FailingWorkflowAgentSessions),
                Arc::new(releash_lib::test_support::integration::workflow::RepositoryIsolatedWorktreeGateway),
                releash_lib::test_support::integration::daemon::serving(),
            );

            let error =
                crate::adaptor_gateway_workflow_workflow_host_test_helpers::reconcile_startup(
                    &host, &app,
                )
                .await
                .unwrap_err();

            assert!(matches!(error, WorkflowRuntimeError::SessionStore(_)));
            assert_eq!(
                releash_lib::test_support::integration::workflow::read_tree_records(
                    &store,
                    VALID_TREE_ID
                )
                .await
                .unwrap()
                .len(),
                valid_count
            );
        }

        #[tokio::test]
        pub async fn test_起動時復旧_未対応permissionはabortせず要対応を記録する() {
            const TREE_ID: &str = "00000000-0000-4000-8000-000000000004";
            let directory = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
                std::sync::Arc::new(
                    releash_lib::test_support::integration::platform::RetryLimiter::new(),
                ),
            ))
            .unwrap();
            let mut fact = SessionExecutionTreeRootFacts::new(
                TREE_ID,
                "/repo",
                "/repo",
                ProviderKind::Claude,
                None,
            )
            .unwrap()
            .started;
            let NodeFact::Started(StartedFact {
                worktree: None,
                root: Some(root),
                ..
            }) = &mut fact
            else {
                unreachable!();
            };
            let NodeKind::Session(spec) = &mut root.definition.as_mut().unwrap().nodes[0].kind
            else {
                unreachable!();
            };
            spec.permission = Some(SessionPermission::Auto);
            let legacy_detail =
                releash_lib::test_support::integration::workflow::encode_detail(&fact)
                    .unwrap()
                    .replace(
                        r#""permission":"auto""#,
                        r#""permission":"bypassPermissions""#,
                    );
            store
                .append_node_event(
                    NewNodeEventRow {
                        tree_id: TREE_ID.to_string(),
                        node_execution_id: TREE_ID.to_string(),
                        parent_id: None,
                        node_name: "session".to_string(),
                        kind: "session".to_string(),
                        attempt: 1,
                        event_type: "started".to_string(),
                        session_id: None,
                        detail: legacy_detail,
                    },
                    Some(1),
                )
                .await
                .unwrap();

            assert!(
                releash_lib::test_support::integration::workflow::read_tree_records(
                    &store, TREE_ID
                )
                .await
                .is_err()
            );

            let app = crate::adaptor_gateway_workflow_workflow_host_test_helpers::dependencies(
                Some(store.clone()),
            );
            let host = WorkflowRuntimeHost::with_runtime_ports(
                releash_lib::test_support::integration::platform::shared().clone(),
                Arc::new(UnusedWorkflowResolver),
                Arc::new(UnusedWorktreeResolver),
                crate::adaptor_gateway_workflow_workflow_host_test_helpers::workspace_query(store.clone()),
                Arc::new(FailingWorkflowAgentSessions),
                Arc::new(releash_lib::test_support::integration::workflow::RepositoryIsolatedWorktreeGateway),
                releash_lib::test_support::integration::daemon::serving(),
            );

            use releash_lib::test_support::integration::workflow::HostWorkflowStartup;
            use releash_lib::test_support::integration::workflow::StoredWorkflowStartupRepository;
            use releash_lib::test_support::integration::workflow::WorkflowStartupRepository;
            let repository = Arc::new(StoredWorkflowStartupRepository(store));
            let connection = rusqlite::Connection::open(
                releash_lib::test_support::integration::persistence::StoreLayout::new(
                    directory.path(),
                )
                .database_path(),
            )
            .unwrap();
            let (queue, failure_store) =
                releash_lib::test_support::integration::platform::test_retrying_with_store();
            let runtime = Arc::new(HostWorkflowStartup {
                host: Arc::new(host),
                app,
            });
            for _ in 0..2 {
                let startup =
                    releash_lib::test_support::integration::workflow::WorkflowStartupUsecase::new(
                        repository.clone(),
                        runtime.clone(),
                    );
                assert!(releash_lib::test_support::integration::platform::recover(
                    &queue, &startup
                )
                .await
                .unwrap_err()
                .to_string()
                .contains("bypassPermissions"));
                let after = repository.load(TREE_ID).await.unwrap().unwrap();
                let count: i64 = connection
                    .query_row("SELECT COUNT(*) FROM node_events", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(count, 1);
                assert!(after.execution.is_active());
                let observations = failure_store.records(TREE_ID);
                assert_eq!(observations.len(), 1);
                assert_eq!(
                    observations[0].record.kind,
                    releash_lib::test_support::integration::platform::Failure::Business(
                        releash_lib::test_support::integration::platform::BusinessFailure::Other
                    )
                );
                assert!(observations[0].requires_attention);
            }
        }
    }
}
