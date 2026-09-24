use super::test_helpers::*;
use super::*;
use crate::adaptor::gateway::workflow::fact_codec;
use crate::domain::workflow::{NodeFact, NodeProcessPresence};
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
    let records = workflow_fact_log::read_tree_records(&fixture.store, &execution_id)
        .await
        .unwrap();
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
        let before = workflow_fact_log::read_tree_records(&fixture.store, &started.execution_id).await.unwrap();
        assert!(!before.iter().any(|record| matches!(record.fact, NodeFact::SessionAttached(_) | NodeFact::CommandSpawned(_))));
        let restored = fixture.restarted_host();

        // When
        reconcile_startup(&restored, &fixture.app).await.unwrap();

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
        ).await.unwrap().unwrap();
        let attempts = folded.aggregate.node_executions.iter().filter(|attempt| attempt.node_name == "main").collect::<Vec<_>>();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].id, node.id);
        assert_eq!(attempts[0].attempt, 1);
        assert_eq!(attempts[0].worktree.as_ref(), Some(worktree));
        assert!(attempts.iter().all(|attempt| matches!(attempt.status, NodeExecutionStatus::Running | NodeExecutionStatus::Succeeded)));
        assert_eq!(std::fs::read_to_string(format!("{}/keep", worktree.path)).unwrap(), "uncommitted");
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
    let records = workflow_fact_log::read_tree_records(&fixture.store, &execution_id)
        .await
        .unwrap();
    assert!(records
        .iter()
        .all(|record| !fact_codec::event_type(&record.fact).starts_with("isolated_worktree")));
}

#[tokio::test]
async fn test_隔離起動_生成失敗後は自動で新しいattemptだけを起動する() {
    // Given
    let fixture = Fixture::new(1);

    // When
    let execution_id = fixture.start("  main: {sequence: {children: [work]}}\n  work: {worktree: isolated, session: {provider: codex, facets: {instruction: policy-confirmation}}}").await;
    fixture.wait_startup_retries().await;

    // Then
    let snapshot = fixture
        .host
        .get_state_by_execution_id(&fixture.app, &execution_id)
        .await
        .unwrap();
    let attempts = snapshot
        .node_executions
        .iter()
        .filter(|node| node.node_name == "work")
        .collect::<Vec<_>>();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].status, NodeExecutionStatus::Aborted);
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
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        folded
            .aggregate
            .node_execution(&attempts[0].id)
            .unwrap()
            .status,
        NodeExecutionStatus::Aborted
    );
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
        .get_state_by_execution_id(&fixture.app, &execution_id)
        .await
        .unwrap();
    assert_eq!(snapshot.node_executions.len(), 1);
    assert_eq!(
        snapshot.node_executions[0].status,
        NodeExecutionStatus::Running
    );
    assert!(!snapshot.node_executions[0].can_retry(NodeProcessPresence::ConfirmedAbsent));
    assert!(fixture.sessions.prepared.lock().unwrap().is_empty());
    let restored = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &execution_id,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        restored.aggregate.node_executions[0].status,
        NodeExecutionStatus::Running
    );
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
            .await
            .unwrap()
            .unwrap();
            let target_name = if root { "main" } else { "isolated" };
            let target = folded
                .aggregate
                .node_executions
                .iter()
                .find(|node| node.node_name == target_name)
                .unwrap();
            assert_eq!(target.status, NodeExecutionStatus::Running);
            let records = workflow_fact_log::read_tree_records(&fixture.store, &execution_id)
                .await
                .unwrap();
            assert!(records.iter().any(|record| matches!(&record.fact, NodeFact::RuntimeFailureObserved(failure) if failure.reason.contains("isolated composite child start append failed"))));
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
async fn test_隔離合成子_子開始commitの状態通知は一度だけ送る() {
    // Given
    let fixture = Fixture::new(0);
    let snapshot = fixture.persist_started("  main: {worktree: isolated, sequence: {children: [work]}}\n  work: {session: {provider: codex}}", "/repo").await;
    let starts = fixture
        .host
        .load_control_plane_execution(&fixture.app, &snapshot.execution_id)
        .await
        .unwrap()
        .unwrap()
        .isolated_composite_start(&snapshot.node_executions[0].id)
        .unwrap();
    let mut broadcasts = record_workflow_execution_broadcasts(&fixture.app);
    // When
    let leaves = fixture
        .host
        .prepare_isolated_starts(
            &fixture.app,
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
    let mut broadcasts = record_workflow_execution_broadcasts(&fixture.app);
    // When
    let id = fixture.start("  main: {worktree: isolated, fanout: {items: [], children: [work]}}\n  work: {session: {provider: codex}}").await;
    let folded = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &id,
    )
    .await
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

#[tokio::test]
async fn startup_exhaustion_leaves_five_distinct_attempts_with_only_the_latest_running() {
    for kind in ["session: {provider: codex}", "command: 'must-not-start'"] {
        let fixture = Fixture::new(usize::MAX);
        let id = fixture
            .start(&format!("  main: {{worktree: isolated, {kind}}}"))
            .await;
        fixture.wait_startup_retries().await;
        let folded = workflow_fact_log::fold_tree_from(
            &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
            &id,
        )
        .await
        .unwrap()
        .unwrap();
        let attempts = folded.aggregate.node_executions();
        assert_eq!(attempts.len(), 5);
        assert!(attempts[..4]
            .iter()
            .all(|node| node.status == NodeExecutionStatus::Aborted));
        let last = &attempts[4];
        assert_eq!(last.status, NodeExecutionStatus::Running);
        assert_eq!(last.attempt, 5);
        assert_eq!(
            last.can_retry(NodeProcessPresence::ConfirmedAbsent),
            last.kind == NodeKindName::Command
        );
        assert_eq!(
            last.can_resume_session(NodeProcessPresence::ConfirmedAbsent),
            last.kind == NodeKindName::Session
        );
        assert_eq!(fixture.worktrees.calls.lock().unwrap().len(), 5);
        let worktrees = attempts
            .iter()
            .map(|node| node.worktree.as_ref().unwrap().path.clone())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(worktrees.len(), 5);
        assert!(fixture.sessions.prepared.lock().unwrap().is_empty());
        assert!(fixture
            .host
            .node_processes
            .active_commands
            .lock()
            .unwrap()
            .is_empty());
        let records = workflow_fact_log::read_tree_records(&fixture.store, &id)
            .await
            .unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record.fact, NodeFact::RuntimeFailureObserved(_)))
                .count(),
            5
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record.fact, NodeFact::RetryRequested))
                .count(),
            4
        );
        let restored = fixture.restarted_host();
        reconcile_startup(&restored, &fixture.app).await.unwrap();
        let after = restored.load_executions(&fixture.app, &id).await.unwrap()[&id].clone();
        assert_eq!(after.node_executions(), attempts);
        assert_eq!(fixture.worktrees.calls.lock().unwrap().len(), 5);
        assert_eq!(
            workflow_fact_log::read_tree_records(&fixture.store, &id)
                .await
                .unwrap(),
            records
        );
    }
}

#[tokio::test]
async fn test_自動再試行_abortとshutdownは待機を終了し追加起動しない() {
    for abort in [true, false] {
        let fixture = Fixture::new(usize::MAX);
        let tree = fixture
            .start("  main: {worktree: isolated, session: {provider: codex}}")
            .await;
        assert_eq!(fixture.host.startup_retries.lock().await.len(), 1);
        if abort {
            fixture
                .host
                .abort_workflow_execution(&fixture.app, &tree, None)
                .await
                .unwrap();
        } else {
            fixture.host.shutdown_all_active_commands().await;
        }
        fixture.wait_startup_retries().await;
        tokio::time::pause();
        tokio::time::advance(std::time::Duration::from_secs(20)).await;
        tokio::time::resume();
        assert_eq!(fixture.worktrees.calls.lock().unwrap().len(), 1);
        assert!(!workflow_fact_log::read_tree_records(&fixture.store, &tree)
            .await
            .unwrap()
            .iter()
            .any(|record| matches!(record.fact, NodeFact::RetryRequested)));
        assert!(fixture.sessions.activated.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn test_自動再試行_待機中の手動resume後に旧attemptを再起動しない() {
    let fixture = Fixture::new(0);
    fixture
        .sessions
        .preparation_fails
        .store(true, Ordering::SeqCst);
    let tree = fixture
        .start("  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}")
        .await;
    let previous = fixture
        .host
        .load_executions(&fixture.app, &tree)
        .await
        .unwrap()[&tree]
        .node_executions[0]
        .clone();
    fixture
        .sessions
        .preparation_fails
        .store(false, Ordering::SeqCst);
    control(&fixture)
        .resume_session_node(
            crate::usecase::workflow::command::ResumeSessionNodeCommand {
                execution_id: tree.clone(),
                node_execution_id: previous.id,
            },
        )
        .await
        .unwrap();
    fixture.wait_startup_retries().await;
    assert_eq!(
        fixture
            .host
            .load_executions(&fixture.app, &tree)
            .await
            .unwrap()[&tree]
            .node_executions
            .len(),
        2
    );
    assert_eq!(fixture.sessions.activated.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_自動再試行_shutdownは進行中の準備の終了を待つ() {
    let fixture = Fixture::new(0);
    fixture
        .sessions
        .preparation_fails
        .store(true, Ordering::SeqCst);
    let tree = fixture
        .start("  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}")
        .await;
    fixture
        .sessions
        .block_preparation
        .store(true, Ordering::SeqCst);
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        fixture.sessions.preparation_entered.notified(),
    )
    .await
    .unwrap();
    let mut shutdown = Box::pin(fixture.host.shutdown_all_active_commands());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(1), &mut shutdown)
            .await
            .is_err()
    );
    fixture.sessions.preparation_release.notify_one();
    shutdown.await;
    assert!(fixture.host.startup_retries.lock().await.is_empty());
    assert_eq!(
        fixture
            .host
            .load_executions(&fixture.app, &tree)
            .await
            .unwrap()[&tree]
            .node_executions
            .len(),
        2
    );
    assert_eq!(fixture.sessions.prepared.lock().unwrap().len(), 2);
    assert!(fixture.sessions.activated.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_session起動_準備済みの旧attemptを除外して兄弟だけを起動する() {
    let (fixture, root) = Fixture::with_repository();
    let snapshot = fixture.persist_started(
        "  main: {fanout: {children: [one, two]}}\n  one: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n  two: {session: {provider: codex, facets: {instruction: policy-confirmation}}}", &root,
    ).await;
    let before = fixture
        .host
        .load_executions(&fixture.app, &snapshot.execution_id)
        .await
        .unwrap()[&snapshot.execution_id]
        .clone();
    let first = before
        .node_executions
        .iter()
        .find(|node| node.node_name == "one")
        .unwrap();
    let leaves = before
        .node_executions
        .iter()
        .filter(|node| node.kind == NodeKindName::Session)
        .map(|node| NodeStart::Leaf(before.leaf_start_for(&node.id).unwrap()))
        .collect();
    control(&fixture)
        .resume_session_node(
            crate::usecase::workflow::command::ResumeSessionNodeCommand {
                execution_id: snapshot.execution_id.clone(),
                node_execution_id: first.id.clone(),
            },
        )
        .await
        .unwrap();
    fixture
        .host
        .start_nodes(&fixture.app, &snapshot.execution_id, &root, leaves)
        .await
        .unwrap();
    let after = fixture
        .host
        .load_executions(&fixture.app, &snapshot.execution_id)
        .await
        .unwrap()[&snapshot.execution_id]
        .clone();
    assert_eq!(after.node_executions.len(), 4);
    assert_eq!(
        after.node_execution(&first.id).unwrap().status,
        NodeExecutionStatus::Aborted
    );
    assert_eq!(fixture.sessions.activated.lock().unwrap().len(), 2);
    assert!(!fixture
        .sessions
        .activated
        .lock()
        .unwrap()
        .contains(&first.id));
    assert!(after
        .node_executions
        .iter()
        .filter(|node| node.kind == NodeKindName::Session
            && node.status == NodeExecutionStatus::Running)
        .all(|node| node.session_id.is_some()));
}

fn control(
    fixture: &Fixture,
) -> crate::usecase::workflow::control_plane::WorkflowControlPlaneUsecase {
    crate::usecase::workflow::control_plane::WorkflowControlPlaneUsecase::new(Arc::new(
        crate::adaptor::gateway::workflow::WorkflowRuntimeCommandGateway::new_with_driver(
            fixture.app.clone(),
            Arc::new(fixture.host.clone()),
        ),
    ))
}

#[tokio::test]
async fn resume_after_never_successful_session_launch_creates_a_new_attempt_with_initial_instruction(
) {
    let fixture = Fixture::new(0);
    fixture
        .sessions
        .preparation_fails
        .store(true, Ordering::SeqCst);
    let id = fixture.start("  main: {worktree: isolated, session: {provider: codex, facets: {instruction: policy-confirmation}}}").await;
    fixture.wait_startup_retries().await;
    let current = fixture
        .host
        .load_executions(&fixture.app, &id)
        .await
        .unwrap()[&id]
        .node_executions
        .last()
        .unwrap()
        .clone();
    assert_eq!(current.attempt, 5);
    assert_eq!(current.status, NodeExecutionStatus::Running);
    assert!(current.session_id.is_none());
    fixture
        .sessions
        .preparation_fails
        .store(false, Ordering::SeqCst);
    control(&fixture)
        .resume_session_node(
            crate::usecase::workflow::command::ResumeSessionNodeCommand {
                execution_id: id.clone(),
                node_execution_id: current.id,
            },
        )
        .await
        .unwrap();
    let executions = fixture
        .host
        .load_executions(&fixture.app, &id)
        .await
        .unwrap();
    let next = executions[&id].node_executions.last().unwrap();
    assert_eq!(next.attempt, 6);
    assert_eq!(next.status, NodeExecutionStatus::Running);
    assert!(next.session_id.is_some());
    assert_eq!(
        fixture.sessions.initial_instructions.lock().unwrap().len(),
        1
    );
    assert!(!fixture.sessions.initial_instructions.lock().unwrap()[0].is_empty());
    assert!(fixture.sessions.recovered.lock().unwrap().is_empty());
}

#[tokio::test]
async fn resume_recreates_a_lost_isolated_worktree_but_recovers_an_existing_conversation_in_place()
{
    for lost_worktree in [false, true] {
        let (fixture, root) = Fixture::with_repository();
        let id = fixture.start_at("  main: {worktree: isolated, session: {provider: codex, facets: {instruction: policy-confirmation}}}", &root).await;
        let previous = fixture
            .host
            .load_executions(&fixture.app, &id)
            .await
            .unwrap()[&id]
            .node_executions[0]
            .clone();
        fixture.sessions.live_sessions.lock().unwrap().clear();
        if lost_worktree {
            std::fs::remove_dir_all(&previous.worktree.as_ref().unwrap().path).unwrap();
        }
        control(&fixture)
            .resume_session_node(
                crate::usecase::workflow::command::ResumeSessionNodeCommand {
                    execution_id: id.clone(),
                    node_execution_id: previous.id.clone(),
                },
            )
            .await
            .unwrap();
        let executions = fixture
            .host
            .load_executions(&fixture.app, &id)
            .await
            .unwrap();
        let nodes = &executions[&id].node_executions;
        let current = nodes.last().unwrap();
        assert_eq!(current.status, NodeExecutionStatus::Running);
        assert!(std::path::Path::new(&current.worktree.as_ref().unwrap().path).is_dir());
        if lost_worktree {
            assert_eq!(nodes.len(), 2);
            assert_eq!(current.attempt, 2);
            assert_ne!(current.worktree, previous.worktree);
            assert_eq!(
                fixture.sessions.initial_instructions.lock().unwrap().len(),
                2
            );
            assert!(fixture.sessions.recovered.lock().unwrap().is_empty());
        } else {
            assert_eq!(nodes.len(), 1);
            assert_eq!(current.id, previous.id);
            assert_eq!(current.session_id, previous.session_id);
            assert_eq!(
                fixture.sessions.initial_instructions.lock().unwrap().len(),
                1
            );
            assert_eq!(
                *fixture.sessions.recovered.lock().unwrap(),
                vec![previous.id]
            );
        }
    }
}

#[tokio::test]
async fn session_resume_requires_confirmed_absence_and_manual_retry_is_rejected() {
    let (fixture, root) = Fixture::with_repository();
    let id = fixture
        .start_at(
            "  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}",
            &root,
        )
        .await;
    let previous = fixture
        .host
        .load_executions(&fixture.app, &id)
        .await
        .unwrap()[&id]
        .node_executions[0]
        .clone();
    for unknown in [false, true] {
        fixture
            .sessions
            .presence_unknown
            .store(unknown, Ordering::SeqCst);
        assert!(control(&fixture)
            .resume_session_node(
                crate::usecase::workflow::command::ResumeSessionNodeCommand {
                    execution_id: id.clone(),
                    node_execution_id: previous.id.clone()
                }
            )
            .await
            .is_err());
    }
    fixture
        .sessions
        .presence_unknown
        .store(false, Ordering::SeqCst);
    fixture.sessions.live_sessions.lock().unwrap().clear();
    assert!(control(&fixture)
        .retry_node(crate::usecase::workflow::command::RetryNodeCommand {
            execution_id: id.clone(),
            node_execution_id: previous.id.clone()
        })
        .await
        .is_err());
    assert_eq!(
        fixture
            .host
            .load_executions(&fixture.app, &id)
            .await
            .unwrap()[&id]
            .node_executions
            .len(),
        1
    );
    assert!(fixture.sessions.recovered.lock().unwrap().is_empty());
}

#[tokio::test]
async fn resume_without_a_conversation_starts_a_new_attempt_even_when_the_worktree_exists() {
    let (fixture, root) = Fixture::with_repository();
    let id = fixture
        .start_at(
            "  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}",
            &root,
        )
        .await;
    let previous = fixture
        .host
        .load_executions(&fixture.app, &id)
        .await
        .unwrap()[&id]
        .node_executions[0]
        .clone();
    fixture.sessions.live_sessions.lock().unwrap().clear();
    fixture
        .sessions
        .conversation_missing
        .store(true, Ordering::SeqCst);
    control(&fixture)
        .resume_session_node(
            crate::usecase::workflow::command::ResumeSessionNodeCommand {
                execution_id: id.clone(),
                node_execution_id: previous.id.clone(),
            },
        )
        .await
        .unwrap();
    let executions = fixture
        .host
        .load_executions(&fixture.app, &id)
        .await
        .unwrap();
    let nodes = &executions[&id].node_executions;
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].status, NodeExecutionStatus::Aborted);
    assert_eq!(nodes[1].attempt, 2);
    assert_ne!(nodes[1].session_id, previous.session_id);
    let instructions = fixture.sessions.initial_instructions.lock().unwrap();
    assert_eq!(instructions.len(), 2);
    assert!(!instructions[1].is_empty());
    assert!(fixture.sessions.recovered.lock().unwrap().is_empty());
}

#[tokio::test]
async fn missing_worktree_rejects_command_retry_but_does_not_prevent_abort() {
    let mut fixture = Fixture::new(0);
    fixture.host.worktree_resolver = Arc::new(CanonicalizingWorktreeResolver);
    let missing = fixture._directory.path().join("missing");
    let missing = missing.to_str().unwrap();
    let snapshot = fixture
        .persist_started("  main: {command: 'must-not-start'}", missing)
        .await;
    let id = snapshot.execution_id;
    fixture
        .host
        .register_started_execution_tree(&fixture.app, &id)
        .await
        .unwrap();
    let node = &snapshot.node_executions[0];
    assert!(fixture
        .host
        .worktree_resolver
        .resolve(missing.into())
        .await
        .is_err());
    let error = control(&fixture)
        .retry_node(crate::usecase::workflow::command::RetryNodeCommand {
            execution_id: id.clone(),
            node_execution_id: node.id.clone(),
        })
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("execution worktree does not exist"),
        "{error}"
    );
    fixture
        .host
        .abort_workflow_execution(&fixture.app, &id, None)
        .await
        .unwrap();
    let folded = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &id,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        folded.aggregate.node_executions[0].status,
        NodeExecutionStatus::Aborted
    );
    assert!(fixture
        .host
        .node_processes
        .active_commands
        .lock()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn new_attempt_commit_rechecks_process_presence_before_recording_retry() {
    let fixture = Fixture::new(0);
    let id = fixture
        .start("  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}")
        .await;
    let before = fixture
        .host
        .load_executions(&fixture.app, &id)
        .await
        .unwrap()[&id]
        .clone();
    let node_id = before.node_executions[0].id.clone();
    let mut candidate = before.clone();
    let restarted = candidate
        .restart_node_attempt_at(&node_id, "next-attempt".into(), current_timestamp())
        .unwrap();
    let events = [
        WorkflowEvent::NodeRetryRequested {
            execution_id: id.clone(),
            node_execution_id: node_id.clone(),
            timestamp: current_timestamp(),
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: id.clone(),
            node_execution_id: restarted.attempt.id,
            node_name: restarted.attempt.node_name,
            kind: restarted.attempt.kind,
            attempt: restarted.attempt.attempt,
            parent: restarted.attempt.parent,
            timestamp: current_timestamp(),
        },
    ];
    let result = fixture
        .host
        .commit_control_plane_candidate(
            &fixture.app,
            ControlPlaneCommitCandidate {
                execution_id: &id,
                snapshot_before: before.clone(),
                candidate,
                transition_outcome: TransitionOutcome::Applied,
                events: &events,
                provider_events: Vec::new(),
            },
        )
        .await;
    assert!(result.is_err());
    assert_eq!(
        fixture
            .host
            .load_executions(&fixture.app, &id)
            .await
            .unwrap()[&id],
        before
    );
    assert!(!workflow_fact_log::read_tree_records(&fixture.store, &id)
        .await
        .unwrap()
        .iter()
        .any(|record| matches!(record.fact, NodeFact::RetryRequested)));
}

#[tokio::test]
async fn test_session再開_fanoutの準備中は起動を待ち兄弟を取り消さない() {
    let (fixture, root) = Fixture::with_repository();
    fixture
        .sessions
        .block_preparation
        .store(true, Ordering::SeqCst);
    let mut start = Box::pin(fixture.start_at(
        "  main: {fanout: {children: [one, two]}}\n  one: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n  two: {session: {provider: codex, facets: {instruction: policy-confirmation}}}",
        &root,
    ));
    tokio::select! {
        _ = fixture.sessions.preparation_entered.notified() => {}
        _ = &mut start => panic!("preparation must wait"),
    }
    let tree_id = workflow_fact_log::list_tree_ids(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        None,
    )
    .await
    .unwrap()
    .remove(0);
    let before = fixture
        .host
        .load_control_plane_execution(&fixture.app, &tree_id)
        .await
        .unwrap()
        .unwrap();
    let target = before
        .node_executions
        .iter()
        .find(|node| node.node_name == "one")
        .unwrap();
    let control = control(&fixture);
    let mut resume = Box::pin(control.resume_session_node(
        crate::usecase::workflow::command::ResumeSessionNodeCommand {
            execution_id: before.id.clone(),
            node_execution_id: target.id.clone(),
        },
    ));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut resume)
            .await
            .is_err()
    );
    assert_eq!(
        fixture
            .host
            .load_executions(&fixture.app, &before.id)
            .await
            .unwrap()[&before.id],
        before
    );
    fixture.sessions.preparation_release.notify_one();
    let (_, result) = tokio::join!(start, resume);
    assert!(result.is_err());
    let after = fixture
        .host
        .load_executions(&fixture.app, &before.id)
        .await
        .unwrap()[&before.id]
        .clone();
    assert_eq!(after.node_executions.len(), 3);
    assert!(after
        .node_executions
        .iter()
        .all(|node| node.status == NodeExecutionStatus::Running));
    assert_eq!(fixture.sessions.activated.lock().unwrap().len(), 2);
    assert!(
        !workflow_fact_log::read_tree_records(&fixture.store, &before.id)
            .await
            .unwrap()
            .iter()
            .any(|record| matches!(record.fact, NodeFact::RetryRequested))
    );
}

struct CanonicalizingWorktreeResolver;

#[async_trait::async_trait]
impl ManagedWorktreeResolver for CanonicalizingWorktreeResolver {
    async fn resolve(
        &self,
        path: String,
    ) -> Result<String, crate::usecase::workflow::runtime_resolver::ManagedWorktreeResolverError>
    {
        crate::adaptor::gateway::workflow::worktree_gateway::normalize_worktree_filter_path(&path)
            .map_err(
            crate::usecase::workflow::runtime_resolver::ManagedWorktreeResolverError::Validation,
        )
    }
}

#[tokio::test]
async fn test_command再試行_プロセス無しでフォルダがあれば新attemptを起動して完了する() {
    // Given
    let fixture = Fixture::new(0);
    let root = fixture._directory.path().to_str().unwrap();
    let snapshot = fixture
        .persist_started("  main: {command: 'printf retried'}", root)
        .await;
    let old = &snapshot.node_executions[0];
    // When
    control(&fixture)
        .retry_node(crate::usecase::workflow::command::RetryNodeCommand {
            execution_id: snapshot.execution_id.clone(),
            node_execution_id: old.id.clone(),
        })
        .await
        .unwrap();
    // Then
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let current = workflow_fact_log::fold_tree_from(
                &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
                &snapshot.execution_id,
            )
            .await
            .unwrap()
            .unwrap();
            if current.aggregate.node_executions.last().unwrap().status
                == NodeExecutionStatus::Succeeded
            {
                assert_eq!(
                    current
                        .aggregate
                        .node_executions
                        .last()
                        .unwrap()
                        .artifact
                        .as_ref()
                        .unwrap()["stdout"],
                    "retried"
                );
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let folded = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &snapshot.execution_id,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(folded.aggregate.node_executions.len(), 2);
    assert_eq!(
        folded.aggregate.node_executions[0].status,
        NodeExecutionStatus::Aborted
    );
    let next = &folded.aggregate.node_executions[1];
    assert_ne!(next.id, old.id);
    assert_eq!(next.attempt, 2);
    assert_eq!(next.status, NodeExecutionStatus::Succeeded);
    assert!(
        workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id)
            .await
            .unwrap()
            .iter()
            .any(|record| record.meta.node_execution_id == next.id
                && matches!(record.fact, NodeFact::CommandSpawned(_)))
    );
}

#[tokio::test]
async fn test_プロセス在否_実供給元の変化が読取と通知と操作可否に反映される() {
    use crate::domain::workflow::NodeProcessReader;
    use crate::domain::workspace_tree::{
        WorkspaceNodeStatusClassification, WorkspaceTreeRepository,
    };
    use crate::usecase::workflow::ports::WorkflowExecutionProjectionRepository;
    // Given
    for kind in [NodeKindName::Command, NodeKindName::Session] {
        let fixture = Fixture::new(0);
        let tree = if kind == NodeKindName::Session {
            fixture.start("  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}").await
        } else {
            fixture
                .persist_started(
                    "  main: {command: 'must-not-start'}",
                    "/repo-worktrees/development",
                )
                .await
                .execution_id
        };
        let current = fixture
            .host
            .load_executions(&fixture.app, &tree)
            .await
            .unwrap()[&tree]
            .clone();
        let node = &current.node_executions[0];
        if kind == NodeKindName::Session {
            let records = workflow_fact_log::read_tree_records(&fixture.store, &tree)
                .await
                .unwrap();
            let meta = &records
                .iter()
                .find(|record| record.meta.node_execution_id == node.id)
                .unwrap()
                .meta;
            workflow_fact_log::append_single_fact(
                &fixture.store,
                meta,
                &NodeFact::AgentActivityObserved(
                    crate::domain::workflow::AgentActivityObservedFact {
                        activity: crate::domain::workflow::AgentSessionActivity::Working,
                    },
                ),
                (current_timestamp() * 1000.0) as i64,
            )
            .await
            .unwrap();
        }
        let mut projection = crate::adaptor::gateway::workflow::execution_projection_repository::WorkflowExecutionProjectionLogRepository::new(fixture.store.clone());
        projection.processes = Some(fixture.host.node_processes.clone());
        let mut workspace =
            crate::adaptor::gateway::workspace_tree::SqliteWorkspaceTreeRepository::new(
                fixture.store.clone(),
            );
        Arc::get_mut(&mut workspace).unwrap().processes = Some(fixture.host.node_processes.clone());
        for expected in [
            NodeProcessPresence::Live,
            NodeProcessPresence::ConfirmedAbsent,
            NodeProcessPresence::Unknown,
        ] {
            if kind == NodeKindName::Command && expected == NodeProcessPresence::Unknown {
                continue;
            }
            // When
            if kind == NodeKindName::Command {
                let mut commands = fixture.host.node_processes.active_commands.lock().unwrap();
                if expected == NodeProcessPresence::Live {
                    commands.insert(node.id.clone(), crate::infrastructure::process::command_runner::ActiveCommandHandle::for_test());
                } else {
                    commands.remove(&node.id);
                }
            } else {
                fixture
                    .sessions
                    .presence_unknown
                    .store(expected == NodeProcessPresence::Unknown, Ordering::SeqCst);
                let mut sessions = fixture.sessions.live_sessions.lock().unwrap();
                if expected == NodeProcessPresence::Live {
                    sessions.insert(node.session_id.clone().unwrap());
                } else {
                    sessions.clear();
                }
            }
            // Then
            assert_eq!(
                fixture
                    .host
                    .node_processes
                    .presence(
                        &current.worktree_path,
                        &node.id,
                        kind,
                        node.session_id.as_deref()
                    )
                    .unwrap(),
                expected
            );
            let retry =
                kind == NodeKindName::Command && expected == NodeProcessPresence::ConfirmedAbsent;
            let resume =
                kind == NodeKindName::Session && expected == NodeProcessPresence::ConfirmedAbsent;
            let model = projection
                .get_execution(
                    &crate::domain::workflow::ExecutionTreeId::new(tree.clone()).unwrap(),
                )
                .await
                .unwrap()
                .unwrap();
            let read = model
                .node_executions
                .iter()
                .find(|read| read.id == node.id)
                .unwrap();
            assert_eq!(read.process_presence, expected);
            assert_eq!(read.can_retry(), retry);
            assert_eq!(read.can_resume_session(), resume);
            let read = workspace
                .load_node_by_node_execution_id(&node.id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(read.process_presence, expected);
            assert_eq!(read.can_retry, retry);
            assert_eq!(read.can_resume_session, resume);
            assert_eq!(
                read.status_classification,
                if expected == NodeProcessPresence::ConfirmedAbsent {
                    WorkspaceNodeStatusClassification::Attention
                } else {
                    WorkspaceNodeStatusClassification::Active
                }
            );
            let mut receiver = record_workflow_execution_broadcasts(&fixture.app);
            workflow_runtime_session::broadcast_state(
                &fixture.app,
                &current.worktree_path,
                RuntimeCommitSnapshot::from_execution(&current).unwrap(),
            )
            .await;
            let broadcasts = take_workflow_execution_broadcasts(&mut receiver);
            let pushed = broadcasts
                .last()
                .unwrap()
                .workflow_execution
                .node_executions
                .iter()
                .find(|read| read.id == node.id)
                .unwrap();
            assert_eq!(pushed.process_presence, expected.as_str());
            assert_eq!(pushed.can_retry, retry);
            assert_eq!(pushed.can_resume_session, resume);
        }
    }
}

#[tokio::test]
async fn test_空の隔離fanout_記録で完了が導出されても準備してsequenceの次を開始する() {
    // Given
    let fixture = Fixture::new(0);
    // When
    let id = fixture.start("  main: {sequence: {children: [empty, next]}}\n  empty: {worktree: isolated, fanout: {items: [], children: [work]}}\n  work: {session: {provider: codex}}\n  next: {session: {provider: codex, facets: {instruction: policy-confirmation}}}").await;
    // Then
    let execution = fixture
        .host
        .load_execution(&fixture.app, &id)
        .await
        .unwrap();
    assert_eq!(fixture.worktrees.calls.lock().unwrap().len(), 1);
    assert!(execution
        .node_executions
        .iter()
        .any(|node| node.node_name == "next" && node.status == NodeExecutionStatus::Running));
    assert_eq!(fixture.sessions.activated.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_隔離合成子競合_最新記録で子開始を再評価し競合を障害にしない() {
    use crate::usecase::workflow::command::CONTROL_PLANE_MAX_ATTEMPTS;
    for change in ["sibling", "abort", "exhausted"] {
        for kind in ["sequence", "fanout"] {
            // Given
            let fixture = Fixture::new(0);
            let snapshot = fixture.persist_started(&format!(
                "  main: {{fanout: {{children: [isolated, other]}}}}\n  isolated: {{worktree: isolated, {kind}: {{children: [work]}}}}\n  other: {{command: 'must-not-start'}}\n  work: {{session: {{provider: codex}}}}"
            ), "/repo").await;
            let target = snapshot
                .node_executions
                .iter()
                .find(|node| node.node_name == "isolated")
                .unwrap();
            let sibling = snapshot
                .node_executions
                .iter()
                .find(|node| node.node_name == "other")
                .unwrap();
            let commit_lock = fixture.host.commit_lock(&snapshot.execution_id).await;
            let mut guard = commit_lock.lock().await;
            let mut operation = Box::pin(fixture.host.commit_prepared_composite(
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
                super::test_helpers::poll_until_pending(operation.as_mut(), || {
                    Arc::strong_count(&commit_lock) > 1
                })
                .await;
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
                "exhausted" => {
                    let error = result.unwrap_err();
                    assert!(matches!(error, WorkflowRuntimeError::Conflict(_)));
                    assert!(matches!(
                        fixture
                            .host
                            .settle_runtime_failure_for_node(
                                &fixture.app,
                                &snapshot.execution_id,
                                &target.id,
                                &error,
                            )
                            .await,
                        Err(WorkflowRuntimeError::Conflict(_))
                    ));
                }
                "abort" => assert!(result.unwrap().is_none()),
                _ => {
                    let (_, decision) = result.unwrap().unwrap();
                    assert!(matches!(decision,
                        crate::domain::workflow::entities::workflow_execution::ExecutionAdvanceDecision::StartNodes(ref starts) if starts.len() == 1));
                }
            }
            let records =
                workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id)
                    .await
                    .unwrap();
            assert_eq!(
                records
                    .iter()
                    .filter(|record| record.meta.node_name == "work"
                        && matches!(record.fact, NodeFact::Started(_)))
                    .count(),
                usize::from(change == "sibling")
            );
            assert!(!records
                .iter()
                .any(|record| matches!(record.fact, NodeFact::RuntimeFailureObserved(_))));
            let execution = fixture
                .host
                .load_execution(&fixture.app, &snapshot.execution_id)
                .await
                .unwrap();
            assert_eq!(
                execution.node_execution(&target.id).unwrap().status,
                if change == "abort" {
                    NodeExecutionStatus::Aborted
                } else {
                    NodeExecutionStatus::Running
                }
            );
        }
    }
}

#[tokio::test]
async fn test_隔離合成子競合_上限後も兄弟と競合したchildを起動して生成失敗の自動再試行を続ける() {
    // Given
    let fixture = Fixture::new(1);
    let snapshot = fixture.persist_started(
        "  main: {fanout: {children: [failed, other, isolated, writer]}}\n  failed: {worktree: isolated, session: {provider: codex, facets: {instruction: policy-confirmation}}}\n  other: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n  isolated: {worktree: isolated, sequence: {children: [work]}}\n  work: {session: {provider: codex, facets: {instruction: policy-confirmation}}}\n  writer: {command: must-not-start}", "/repo"
    ).await;
    let execution = fixture
        .host
        .load_execution(&fixture.app, &snapshot.execution_id)
        .await
        .unwrap();
    let node = |name: &str| {
        execution
            .node_executions
            .iter()
            .find(|node| node.node_name == name)
            .unwrap()
            .id
            .clone()
    };
    let failed = node("failed");
    let other = node("other");
    let isolated = node("isolated");
    let writer = node("writer");
    let starts = vec![
        NodeStart::Leaf(execution.leaf_start_for(&failed).unwrap()),
        NodeStart::Leaf(execution.leaf_start_for(&other).unwrap()),
        NodeStart::PrepareComposite(execution.isolated_composite_start(&isolated).unwrap()),
    ];
    let barrier = Arc::new(std::sync::Barrier::new(2));
    *fixture.worktrees.creation_barrier.lock().unwrap() = Some(barrier.clone());
    let commit_lock = fixture.host.commit_lock(&snapshot.execution_id).await;
    let mut guard = commit_lock.lock().await;
    let mut operation = Box::pin(fixture.host.start_nodes(
        &fixture.app,
        &snapshot.execution_id,
        "/repo",
        starts,
    ));
    // When
    // 2件のworktree準備が完了してからchild commitを競合させる。
    for _ in 0..2 {
        let barrier = barrier.clone();
        let preparation = tokio::task::spawn_blocking(move || barrier.wait());
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::select! {
                result = preparation => { result.unwrap(); },
                _ = operation.as_mut() => panic!("operation must wait for child commit"),
            }
        })
        .await
        .unwrap();
    }
    *fixture.worktrees.creation_barrier.lock().unwrap() = None;
    for attempt in 0..crate::usecase::workflow::command::CONTROL_PLANE_MAX_ATTEMPTS {
        super::test_helpers::poll_until_pending(operation.as_mut(), || {
            Arc::strong_count(&commit_lock) > 1
        })
        .await;
        workflow_fact_log::append_facts_for_events(
            &fixture.store,
            &[WorkflowEvent::CommandSpawned {
                execution_id: snapshot.execution_id.clone(),
                node_execution_id: writer.clone(),
                display_command: format!("external-{attempt}"),
                timestamp: current_timestamp(),
            }],
        )
        .await
        .unwrap();
        if attempt + 1 == crate::usecase::workflow::command::CONTROL_PLANE_MAX_ATTEMPTS {
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
    operation.await.unwrap();
    fixture.wait_startup_retries().await;
    // Then
    let records = workflow_fact_log::read_tree_records(&fixture.store, &snapshot.execution_id)
        .await
        .unwrap();
    assert!(!records
        .iter()
        .any(|record| record.meta.node_execution_id == isolated
            && matches!(record.fact, NodeFact::RuntimeFailureObserved(_))));
    assert_eq!(
        records
            .iter()
            .filter(|record| record.meta.node_name == "work"
                && matches!(record.fact, NodeFact::Started(_)))
            .count(),
        1
    );
    let child = records
        .iter()
        .find(|record| record.meta.node_name == "work")
        .unwrap();
    assert!(fixture
        .sessions
        .activated
        .lock()
        .unwrap()
        .contains(&child.meta.node_execution_id));
    assert!(!records
        .iter()
        .any(|record| record.meta.node_execution_id == isolated
            && matches!(record.fact, NodeFact::RetryRequested)));
    assert!(fixture.sessions.activated.lock().unwrap().contains(&other));
    let execution = fixture
        .host
        .load_execution(&fixture.app, &snapshot.execution_id)
        .await
        .unwrap();
    let retried = execution
        .node_executions
        .iter()
        .find(|node| node.node_name == "failed" && node.attempt == 2)
        .unwrap();
    assert!(fixture
        .sessions
        .activated
        .lock()
        .unwrap()
        .contains(&retried.id));
}

#[tokio::test]
async fn test_session再開_完了したnodeは状態を変えずに会話だけを再開し会話が無ければ拒否する() {
    // Given
    let (fixture, root) = Fixture::with_repository();
    let id = fixture
        .start_at(
            "  main:\n    session:\n      provider: codex\n      facets:\n        instruction: policy-confirmation",
            &root,
        )
        .await;
    let node = fixture
        .host
        .load_executions(&fixture.app, &id)
        .await
        .unwrap()[&id]
        .node_executions[0]
        .clone();
    let control = control(&fixture);
    control
        .submit_output(crate::usecase::workflow::command::SubmitOutputCommand {
            node_execution_id: node.id.clone(),
            artifact: None,
        })
        .await
        .unwrap();
    control
        .record_provider_stop(
            crate::usecase::provider_lifecycle::ProviderExecutionTreeStopCommand {
                agent_session_id: node.session_id.clone().unwrap(),
                tree_id: id.clone(),
                node_execution_id: node.id.clone(),
                binding_id: "binding".into(),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    let completed = fixture
        .host
        .load_executions(&fixture.app, &id)
        .await
        .unwrap()[&id]
        .clone();
    assert_eq!(completed.state(), &RuntimeExecutionState::Completed);
    assert_eq!(
        completed.node_executions[0].status,
        NodeExecutionStatus::Succeeded
    );
    fixture.sessions.live_sessions.lock().unwrap().clear();
    let resume = || {
        control.resume_session_node(
            crate::usecase::workflow::command::ResumeSessionNodeCommand {
                execution_id: id.clone(),
                node_execution_id: node.id.clone(),
            },
        )
    };

    // When
    resume().await.unwrap();

    // Then
    assert_eq!(
        *fixture.sessions.recovered.lock().unwrap(),
        vec![node.id.clone()]
    );
    let resumed = fixture
        .host
        .load_executions(&fixture.app, &id)
        .await
        .unwrap()[&id]
        .clone();
    assert_eq!(resumed.state(), &RuntimeExecutionState::Completed);
    assert_eq!(resumed.node_executions.len(), 1);
    assert_eq!(
        resumed.node_executions[0].status,
        NodeExecutionStatus::Succeeded
    );

    // Given
    fixture.sessions.live_sessions.lock().unwrap().clear();
    fixture
        .sessions
        .conversation_missing
        .store(true, Ordering::SeqCst);

    // When / Then
    assert!(resume().await.is_err());
    let rejected = fixture
        .host
        .load_executions(&fixture.app, &id)
        .await
        .unwrap()[&id]
        .clone();
    assert_eq!(rejected.node_executions.len(), 1);
    assert_eq!(
        *fixture.sessions.recovered.lock().unwrap(),
        vec![node.id.clone()]
    );
}
