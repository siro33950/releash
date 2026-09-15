use super::test_helpers::*;
use super::*;
use crate::domain::workflow::{NodeFact, ProcessExitedFact};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn test_隔離起動_commandのcwdと環境変数は生成済みworktreeを指す() {
    // Given
    let (fixture, root) = Fixture::with_repository();

    // When
    let execution_id = fixture.start_at("  main: {sequence: {children: [work]}}\n  work:\n    worktree: isolated\n    command: |\n      pwd -P\n      printf '%s\\n' \"$RELEASH_WORKTREE_PATH\"", &root).await;
    let artifact = fixture.command_artifact(&execution_id).await;

    // Then
    let path = artifact["worktree"]["path"].as_str().unwrap();
    assert_ne!(path, root);
    assert_eq!(artifact["stdout"], format!("{path}\n{path}\n"));
    assert_eq!(artifact["ok"], true);
    let repo = git2::Repository::open(path).unwrap();
    assert_eq!(
        repo.head().unwrap().shorthand().unwrap(),
        artifact["worktree"]["branch"].as_str().unwrap()
    );
    let records = workflow_fact_log::read_tree_records(&fixture.store, &execution_id).unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(record.fact, NodeFact::CommandSpawned(_)))
            .count(),
        1
    );
}

#[tokio::test]
async fn test_隔離復旧_生成後かつ起動記録前のleafと合成子を同一attemptで起動する() {
    for nodes in [
        "  main: {worktree: isolated, session: {provider: codex, facets: {instruction: policy-confirmation}}}",
        "  main:\n    worktree: isolated\n    command: |\n      pwd -P\n      printf '%s\\n' \"$RELEASH_WORKTREE_PATH\"",
        "  main: {worktree: isolated, sequence: {children: [work]}}\n  work: {session: {provider: codex, facets: {instruction: policy-confirmation}}}",
        "  main: {worktree: isolated, fanout: {children: [work]}}\n  work: {session: {provider: codex, facets: {instruction: policy-confirmation}}}",
    ] {
        // Given
        let (fixture, root) = Fixture::with_repository();
        let started = fixture.persist_started(nodes, &root).await;
        let node = &started.node_executions[0];
        let worktree = node.worktree.as_ref().unwrap();
        fixture.host.isolated_worktrees.create(&root, worktree).unwrap();
        std::fs::write(format!("{}/keep", worktree.path), "uncommitted").unwrap();
        let before = workflow_fact_log::read_tree_records(&fixture.store, &started.execution_id).unwrap();
        assert!(!before.iter().any(|record| matches!(record.fact, NodeFact::SessionAttached(_) | NodeFact::CommandSpawned(_))));
        let restored = fixture.restarted_host();

        // When
        restored.reconcile_startup(fixture.app.handle()).await.unwrap();

        // Then
        if node.kind == NodeKindName::Command {
            let artifact = fixture.command_artifact(&started.execution_id).await;
            assert_eq!(artifact["stdout"], format!("{0}\n{0}\n", worktree.path));
        } else {
            let prepared = fixture.sessions.prepared.lock().unwrap();
            assert_eq!(prepared.len(), 1);
            assert_eq!(prepared[0].0, root);
            assert_eq!(prepared[0].1, worktree.path);
            if node.kind == NodeKindName::Session {
                assert_eq!(prepared[0].2, node.id);
            }
            assert_eq!(fixture.sessions.activated.lock().unwrap().len(), 1);
        }
        let folded = workflow_fact_log::fold_tree_from(
            &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
            &started.execution_id,
        ).unwrap().unwrap();
        let attempts = folded.aggregate.node_executions.iter().filter(|attempt| attempt.node_name == "main").collect::<Vec<_>>();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].id, node.id);
        assert_eq!(attempts[0].attempt, 1);
        assert_eq!(attempts[0].worktree.as_ref(), Some(worktree));
        assert!(attempts.iter().all(|attempt| attempt.status != NodeExecutionStatus::Failed));
        assert_eq!(std::fs::read_to_string(format!("{}/keep", worktree.path)).unwrap(), "uncommitted");
    }
}

#[tokio::test]
async fn test_隔離起動_on_failure省略のleafは生成失敗後に手動retryを待つ() {
    for kind in [
        "session: {provider: codex, facets: {instruction: policy-confirmation}}",
        "command: 'this-process-must-not-start'",
    ] {
        // Given
        let fixture = Fixture::new(1);

        // When
        let execution_id = fixture.start(&format!("  main: {{sequence: {{children: [work]}}}}\n  work: {{worktree: isolated, {kind}}}")).await;

        // Then
        let folded = workflow_fact_log::fold_tree_from(
            &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
            &execution_id,
        )
        .unwrap()
        .unwrap();
        let attempts = folded
            .aggregate
            .node_executions
            .iter()
            .filter(|node| node.node_name == "work")
            .collect::<Vec<_>>();
        assert_eq!(attempts.len(), 1);
        let failed = attempts[0];
        assert_eq!(failed.attempt, 1);
        assert_eq!(failed.status, NodeExecutionStatus::Failed);
        assert!(failed.can_retry());
        assert_eq!(fixture.worktrees.calls.lock().unwrap().len(), 1);
        assert!(fixture.sessions.prepared.lock().unwrap().is_empty());
        assert!(fixture.sessions.activated.lock().unwrap().is_empty());
        assert!(fixture.host.active_commands.lock().await.is_empty());
        let records = workflow_fact_log::read_tree_records(&fixture.store, &execution_id).unwrap();
        assert!(!records.iter().any(|record| matches!(
            record.fact,
            NodeFact::SessionAttached(_) | NodeFact::CommandSpawned(_)
        )));

        // When
        fixture.worktrees.failures.store(1, Ordering::SeqCst);
        let gateway =
            crate::adaptor::gateway::workflow::TauriWorkflowRuntimeCommandGateway::new_with_driver(
                fixture.app.handle().clone(),
                Arc::new(fixture.host.clone()),
                fixture.store.clone(),
                fixture.store.installation_id().to_string(),
            );
        let control = crate::usecase::workflow::control_plane::WorkflowControlPlaneUsecase::new(
            Arc::new(gateway),
        );
        control
            .retry_node(crate::usecase::workflow::command::RetryNodeCommand {
                execution_id: execution_id.clone(),
                node_execution_id: failed.id.clone(),
            })
            .await
            .unwrap();

        // Then
        let snapshot = fixture
            .host
            .get_state_by_execution_id(&execution_id)
            .await
            .unwrap();
        let attempts = snapshot
            .node_executions
            .iter()
            .filter(|node| node.node_name == "work")
            .collect::<Vec<_>>();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[1].attempt, 2);
        assert_ne!(attempts[0].worktree, attempts[1].worktree);
        assert_eq!(fixture.worktrees.calls.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn test_隔離起動_合成子の生成後に同じcwdとroot所属でsessionを準備する() {
    // Given
    let fixture = Fixture::new(0);

    // When
    let execution_id = fixture.start("  main: {worktree: isolated, fanout: {children: [one, two]}}\n  one: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n  two: {session: {provider: codex, facets: {instruction: policy-confirmation}}}").await;

    // Then
    let generated = fixture.worktrees.calls.lock().unwrap();
    assert_eq!(generated.len(), 1);
    assert_eq!(generated[0].0, "/repo-worktrees/development");
    let prepared = fixture.sessions.prepared.lock().unwrap();
    assert_eq!(prepared.len(), 2);
    assert!(prepared.iter().all(
        |(workspace, cwd, _)| workspace == "/repo-worktrees/development"
            && cwd == &generated[0].1.path
    ));
    assert_eq!(fixture.sessions.activated.lock().unwrap().len(), 2);
    let records = workflow_fact_log::read_tree_records(&fixture.store, &execution_id).unwrap();
    assert!(records
        .iter()
        .all(|record| !record.fact.event_type().starts_with("isolated_worktree")));
}

#[tokio::test]
async fn test_隔離起動_生成失敗をnode失敗にして新しいattemptだけを起動する() {
    // Given
    let fixture = Fixture::new(1);

    // When
    let execution_id = fixture.start("  main: {sequence: {children: [{work: {on_failure: {retry: 1}}}]}}\n  work: {worktree: isolated, session: {provider: codex, facets: {instruction: policy-confirmation}}}").await;

    // Then
    let snapshot = fixture
        .host
        .get_state_by_execution_id(&execution_id)
        .await
        .unwrap();
    let attempts = snapshot
        .node_executions
        .iter()
        .filter(|node| node.node_name == "work")
        .collect::<Vec<_>>();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].status, NodeExecutionStatus::Failed);
    assert_eq!(attempts[1].status, NodeExecutionStatus::Running);
    assert_eq!(attempts[1].attempt, 2);
    assert_ne!(attempts[0].worktree, attempts[1].worktree);
    assert_eq!(fixture.worktrees.calls.lock().unwrap().len(), 2);
    assert_eq!(fixture.sessions.prepared.lock().unwrap().len(), 1);
    assert_eq!(
        fixture.sessions.activated.lock().unwrap().as_slice(),
        &[attempts[1].id.clone()]
    );
    let folded = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &execution_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        folded
            .aggregate
            .node_execution(&attempts[0].id)
            .unwrap()
            .status,
        NodeExecutionStatus::Failed
    );
}

#[tokio::test]
async fn test_隔離起動_commandの生成失敗はprocessを起動せずretry予算を使う() {
    // Given
    let fixture = Fixture::new(2);

    // When
    let execution_id = fixture.start("  main: {sequence: {children: [{work: {on_failure: {retry: 1}}}]}}\n  work: {worktree: isolated, command: 'this-process-must-not-start'}").await;

    // Then
    let snapshot = fixture
        .host
        .get_state_by_execution_id(&execution_id)
        .await
        .unwrap();
    let attempts = snapshot
        .node_executions
        .iter()
        .filter(|node| node.node_name == "work")
        .collect::<Vec<_>>();
    assert_eq!(attempts.len(), 2);
    assert!(attempts
        .iter()
        .all(|node| node.status == NodeExecutionStatus::Failed));
    let facts = workflow_fact_log::read_tree_records(&fixture.store, &execution_id).unwrap();
    assert!(!facts
        .iter()
        .any(|record| matches!(record.fact, NodeFact::CommandSpawned(_))));
    assert_eq!(fixture.worktrees.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn test_隔離再開_実体喪失によるprovider起動失敗をnode失敗にし手動retryで新規生成する() {
    // Given
    let fixture = Fixture::new(0);
    let execution_id = fixture.start("  main: {worktree: isolated, session: {provider: codex, facets: {instruction: policy-confirmation}}}").await;
    let snapshot = fixture
        .host
        .get_state_by_execution_id(&execution_id)
        .await
        .unwrap();
    let node = &snapshot.node_executions[0];
    let facts = workflow_fact_log::read_tree_records(&fixture.store, &execution_id).unwrap();
    let meta = &facts
        .iter()
        .find(|record| record.meta.node_execution_id == node.id)
        .unwrap()
        .meta;
    workflow_fact_log::append_single_fact(
        &fixture.store,
        meta,
        &NodeFact::ProcessExited(ProcessExitedFact {
            exit_code: Some(1),
            result_summary: None,
            failure_reason: Some("provider exited".into()),
            failure_kind: Some(NodeExecutionFailureKind::InfrastructureCrash),
        }),
        9999,
    )
    .unwrap();
    fixture
        .sessions
        .recovery_fails
        .store(true, Ordering::SeqCst);

    // When
    let result = fixture
        .host
        .resume_workflow_execution(fixture.app.handle(), &execution_id)
        .await;

    // Then
    assert!(result.is_err());
    let restored = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &execution_id,
    )
    .unwrap()
    .unwrap();
    let failed = restored.aggregate.node_execution(&node.id).unwrap();
    assert_eq!(failed.status, NodeExecutionStatus::Failed);
    assert_eq!(failed.failure.as_ref().unwrap().origin, crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionFailureOrigin::Runtime);
    assert!(failed.recovery_reason.is_none());
    assert!(failed.can_retry());
    assert!(failed.resume_previous_state().is_none());
    let gateway =
        crate::adaptor::gateway::workflow::TauriWorkflowRuntimeCommandGateway::new_with_driver(
            fixture.app.handle().clone(),
            Arc::new(fixture.host.clone()),
            fixture.store.clone(),
            fixture.store.installation_id().to_string(),
        );
    let control = crate::usecase::workflow::control_plane::WorkflowControlPlaneUsecase::new(
        Arc::new(gateway),
    );
    control
        .retry_node(crate::usecase::workflow::command::RetryNodeCommand {
            execution_id: execution_id.clone(),
            node_execution_id: node.id.clone(),
        })
        .await
        .unwrap();
    let calls = fixture.worktrees.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_ne!(calls[0].1, calls[1].1);
    assert!(calls[1].1.branch.ends_with("-a2"));
}

#[tokio::test]
async fn test_隔離起動_合成子の生成失敗では子を起動せず復旧専用理由を追加しない() {
    // Given
    let fixture = Fixture::new(1);

    // When
    let execution_id = fixture.start("  main: {worktree: isolated, sequence: {children: [work]}}\n  work: {session: {provider: codex, facets: {instruction: policy-confirmation}}}").await;

    // Then
    let snapshot = fixture
        .host
        .get_state_by_execution_id(&execution_id)
        .await
        .unwrap();
    assert_eq!(snapshot.node_executions.len(), 1);
    assert_eq!(
        snapshot.node_executions[0].status,
        NodeExecutionStatus::Failed
    );
    assert!(snapshot.node_executions[0].recovery_reason.is_none());
    assert!(!snapshot.node_executions[0].can_retry());
    assert!(fixture.sessions.prepared.lock().unwrap().is_empty());
    let restored = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &execution_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        restored.aggregate.node_executions[0].status,
        NodeExecutionStatus::Failed
    );
}

#[tokio::test]
async fn test_隔離起動_session準備の失敗でもnode失敗として記録する() {
    // Given
    let fixture = Fixture::new(0);
    fixture
        .sessions
        .preparation_fails
        .store(true, Ordering::SeqCst);

    // When
    let execution_id = fixture.start("  main: {worktree: isolated, session: {provider: codex, facets: {instruction: policy-confirmation}}}").await;

    // Then
    let snapshot = fixture
        .host
        .get_state_by_execution_id(&execution_id)
        .await
        .unwrap();
    assert_eq!(
        snapshot.node_executions[0].status,
        NodeExecutionStatus::Failed
    );
    assert!(snapshot.node_executions[0].can_retry());
    assert!(fixture.sessions.activated.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_隔離合成子_子開始のappend失敗はrootと入れ子の対象だけを失敗にする() {
    for root in [true, false] {
        for composite in ["sequence: {children: [work]}", "fanout: {children: [work]}"] {
            // Given
            let fixture = Fixture::new(0);
            rusqlite::Connection::open(fixture._directory.path().join("local-event-store.sqlite3"))
                .unwrap().execute_batch("CREATE TRIGGER fail_isolated_child_start BEFORE INSERT ON node_events WHEN NEW.event_type = 'started' AND NEW.node_name = 'work' BEGIN SELECT RAISE(ABORT, 'injected child start failure'); END;").unwrap();
            let nodes = if root {
                format!("  main: {{worktree: isolated, {composite}}}\n  work: {{session: {{provider: codex, facets: {{instruction: policy-confirmation}}}}}}")
            } else {
                format!("  main: {{fanout: {{children: [isolated, other]}}}}\n  isolated: {{worktree: isolated, {composite}}}\n  other: {{session: {{provider: codex, facets: {{instruction: policy-confirmation}}}}}}\n  work: {{session: {{provider: codex, facets: {{instruction: policy-confirmation}}}}}}")
            };
            // When
            let execution_id = fixture.start(&nodes).await;
            // Then
            let folded = workflow_fact_log::fold_tree_from(
                &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
                &execution_id,
            )
            .unwrap()
            .unwrap();
            let target_name = if root { "main" } else { "isolated" };
            let target = folded
                .aggregate
                .node_executions
                .iter()
                .find(|node| node.node_name == target_name)
                .unwrap();
            assert_eq!(target.status, NodeExecutionStatus::Failed);
            assert!(target
                .failure
                .as_ref()
                .unwrap()
                .reason
                .contains("isolated composite child start append failed"));
            assert!(!folded
                .aggregate
                .node_executions
                .iter()
                .any(|node| node.node_name == "work"));
            if !root {
                let other = folded
                    .aggregate
                    .node_executions
                    .iter()
                    .find(|node| node.node_name == "other")
                    .unwrap();
                assert_eq!(other.status, NodeExecutionStatus::Running);
                assert!(fixture
                    .sessions
                    .activated
                    .lock()
                    .unwrap()
                    .contains(&other.id));
            }
        }
    }
}

#[tokio::test]
async fn test_隔離再開_dispatch失敗は対象だけを失敗にし他の未dispatch状態を復元する() {
    // Given
    let fixture = Fixture::new(0);
    let execution_id = fixture.start("  main: {fanout: {items: [w, x, y, z], children: [work]}}\n  work: {worktree: isolated, session: {provider: codex, facets: {instruction: policy-confirmation}}}").await;
    let records = workflow_fact_log::read_tree_records(&fixture.store, &execution_id).unwrap();
    let nodes = fixture.sessions.activated.lock().unwrap().clone();
    assert_eq!(nodes.len(), 4);
    for (index, id) in nodes.iter().enumerate() {
        let meta = &records
            .iter()
            .find(|record| record.meta.node_execution_id == *id)
            .unwrap()
            .meta;
        workflow_fact_log::append_single_fact(
            &fixture.store,
            meta,
            &NodeFact::ProcessExited(ProcessExitedFact {
                exit_code: Some(if index == 3 { 1 } else { 0 }),
                result_summary: None,
                failure_reason: (index == 3).then(|| "previous failure".into()),
                failure_kind: (index == 3).then_some(NodeExecutionFailureKind::InfrastructureCrash),
            }),
            records.last().unwrap().timestamp_ms + 1,
        )
        .unwrap();
    }
    *fixture.sessions.dispatch_fails_on.lock().unwrap() = Some(nodes[1].clone());
    // When
    let result = fixture
        .host
        .resume_workflow_execution(fixture.app.handle(), &execution_id)
        .await;
    // Then
    assert!(
        matches!(result, Err(WorkflowRuntimeError::AgentSession(reason)) if reason == "dispatch failed")
    );
    assert_eq!(*fixture.sessions.recovered.lock().unwrap(), nodes);
    assert_eq!(*fixture.sessions.dispatched.lock().unwrap(), nodes[..2]);
    let folded = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &execution_id,
    )
    .unwrap()
    .unwrap();
    let first = folded.aggregate.node_execution(&nodes[0]).unwrap();
    let failed = folded.aggregate.node_execution(&nodes[1]).unwrap();
    let paused = folded.aggregate.node_execution(&nodes[2]).unwrap();
    let restored = folded.aggregate.node_execution(&nodes[3]).unwrap();
    assert_eq!(first.status, NodeExecutionStatus::Running);
    assert_eq!(failed.status, NodeExecutionStatus::Failed);
    assert!(failed
        .failure
        .as_ref()
        .unwrap()
        .reason
        .contains("dispatch failed"));
    assert!(failed.resume_previous_state().is_none());
    assert!(failed.recovery_reason.is_none());
    assert_eq!(paused.status, NodeExecutionStatus::Paused);
    assert!(paused.resume_previous_state().is_some());
    assert_eq!(restored.status, NodeExecutionStatus::Failed);
    assert_eq!(
        restored.failure.as_ref().unwrap().reason,
        "previous failure"
    );
    assert!(restored.resume_previous_state().is_some());
    assert_eq!(fixture.worktrees.calls.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn test_隔離合成子_子開始commitの状態通知は一度だけ送る() {
    // Given
    let fixture = Fixture::new(0);
    let snapshot = fixture.persist_started("  main: {worktree: isolated, sequence: {children: [work]}}\n  work: {session: {provider: codex}}", "/repo").await;
    let starts = fixture
        .host
        .executions
        .lock()
        .await
        .get(&snapshot.execution_id)
        .unwrap()
        .isolated_composite_start(&snapshot.node_executions[0].id)
        .unwrap();
    let mut broadcasts = record_workflow_execution_broadcasts(fixture.app.handle());
    // When
    let leaves = fixture
        .host
        .prepare_isolated_starts(
            fixture.app.handle(),
            &snapshot.execution_id,
            "/repo",
            vec![super::isolated_worktree::NodePreparation::Composite(starts)],
        )
        .await
        .unwrap();
    // Then
    assert_eq!(leaves.leaves.len(), 1);
    assert_eq!(take_workflow_execution_broadcasts(&mut broadcasts).len(), 1);
}

#[tokio::test]
async fn test_空の隔離fanout_liveと再読取で同じworktree成果を持ち完了する() {
    // Given
    let fixture = Fixture::new(0);
    let mut broadcasts = record_workflow_execution_broadcasts(fixture.app.handle());
    // When
    let id = fixture.start("  main: {worktree: isolated, fanout: {items: [], children: [work]}}\n  work: {session: {provider: codex}}").await;
    let folded = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &id,
    )
    .unwrap()
    .unwrap();
    let read = crate::domain::workflow::services::fact_replay::derive_read_model(&folded);
    // Then
    assert_eq!(
        read.status,
        crate::domain::workflow::ExecutionStatus::Completed
    );
    assert_eq!(fixture.worktrees.calls.lock().unwrap().len(), 1);
    assert!(fixture.sessions.prepared.lock().unwrap().is_empty());
    let views = take_workflow_execution_broadcasts(&mut broadcasts);
    let live = &views.last().unwrap().workflow_execution;
    assert_eq!(
        live.status,
        crate::adaptor::protocol::workflow::ExecutionStatusView::Completed
    );
    assert_eq!(live.node_executions.len(), 1);
    assert_eq!(
        live.node_executions[0].artifact.as_ref().unwrap().value,
        read.node_executions[0].artifact.as_ref().unwrap().value
    );
}
