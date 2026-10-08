use crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture;
use releashd::test_support::integration::process::CommandRunOutput;
use releashd::test_support::integration::process::CommandRunnerError;
use releashd::test_support::integration::workflow::CommandExecutionInput;
use releashd::test_support::integration::workflow::NodeExecutionFailureKind;
use releashd::test_support::integration::workflow::NodeExecutionStatus;
use releashd::test_support::integration::workflow::NodeFact;
use releashd::test_support::integration::workflow::NodeKindName;
use std::collections::BTreeMap;
use std::sync::Arc;

async fn started_command(fixture: &Fixture, nodes: &str) -> CommandExecutionInput {
    let cwd = fixture._directory.path().to_str().unwrap();
    let snapshot = fixture.persist_started(nodes, cwd).await;
    let node = snapshot
        .node_executions
        .iter()
        .find(|node| node.kind == NodeKindName::Command)
        .unwrap();
    let input = CommandExecutionInput::test_new(
        (
            snapshot.execution_id.clone(),
            node.id.clone(),
            node.node_name.clone(),
            node.attempt,
        ),
        cwd.into(),
        None,
        Vec::new(),
        None,
        BTreeMap::new(),
        None,
    );
    assert!(fixture
        .host
        .commit_command_spawned(&fixture.app, &input, "true".into())
        .await
        .unwrap());
    input
}

#[tokio::test]
pub async fn test_command起動_起動済みの記録か登録があれば同じプロセスを生成しない() {
    for persisted in [false, true] {
        // Given
        let fixture = Fixture::new(0);
        let snapshot = fixture
            .persist_started("  main: {command: true}", "/missing/worktree")
            .await;
        let node = &snapshot.node_executions[0];
        let input = CommandExecutionInput::test_new(
            (
                snapshot.execution_id.clone(),
                node.id.clone(),
                node.node_name.clone(),
                node.attempt,
            ),
            "/missing/worktree".into(),
            Some("must-not-start".into()),
            Vec::new(),
            None,
            BTreeMap::new(),
            None,
        );
        if persisted {
            fixture
                .host
                .commit_command_spawned(&fixture.app, &input, "true".into())
                .await
                .unwrap();
        } else {
            fixture
                .host
                .node_processes
                .test_active_commands()
                .lock()
                .unwrap()
                .insert(
                    node.id.clone(),
                    releashd::test_support::integration::process::ActiveCommandHandle::for_test(),
                );
        }
        let before = releashd::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &snapshot.execution_id,
        )
        .await
        .unwrap();

        // When
        fixture
            .host
            .spawn_command_execution(&fixture.app, input)
            .await
            .unwrap();

        // Then
        assert_eq!(
            releashd::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                &snapshot.execution_id
            )
            .await
            .unwrap(),
            before
        );
        assert_eq!(
            fixture
                .host
                .node_processes
                .test_active_commands()
                .lock()
                .unwrap()
                .len(),
            usize::from(!persisted)
        );
        assert!(fixture
            .host
            .test_command_completion_observers()
            .lock()
            .await
            .is_empty());
    }
}

#[tokio::test]
pub async fn test_終了処理_commandの終了結果も次回起動での喪失も記録しない() {
    for completion in ["", "\n    completion:\n      require: approval"] {
        for output in [
            Ok(CommandRunOutput {
                exit_code: 0,
                stdout: "finished".into(),
                stderr: String::new(),
                duration_ms: 1,
            }),
            Ok(CommandRunOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: "failed".into(),
                duration_ms: 1,
            }),
            Err(CommandRunnerError::Wait(std::io::Error::other(
                "wait failed",
            ))),
            Err(CommandRunnerError::Output(std::io::Error::other(
                "read failed",
            ))),
            Err(CommandRunnerError::Cancelled),
        ] {
            // Given
            let fixture = Fixture::new(0);
            let input =
                started_command(&fixture, &format!("  main:\n    command: true{completion}")).await;
            let execution_id = input.test_execution_id().clone();
            let before = releashd::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                &execution_id,
            )
            .await
            .unwrap()
            .len();

            // When
            releashd::test_support::integration::daemon::DaemonUsecase::test_with_repository(
                fixture.host.test_daemon().clone(),
            )
            .stop(releashd::test_support::integration::daemon::StopRequest::Exit { code: 23 })
            .await;
            fixture.host.shutdown_all_active_commands().await;
            fixture
                .host
                .finish_command_execution(&fixture.app, input, output)
                .await;

            // Then
            let records = releashd::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                &execution_id,
            )
            .await
            .unwrap();
            assert_eq!(
                records.len(),
                before,
                "shutdown must not append command facts"
            );
            let folded = releashd::test_support::integration::workflow::fold_tree_from(
                &releashd::test_support::integration::workflow::FactLogReadBackend::Live(
                    fixture.store.clone(),
                ),
                &execution_id,
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(
                folded.aggregate.node_executions[0].status,
                NodeExecutionStatus::Running
            );
            let restarted = fixture.restarted_host();
            crate::adaptor_gateway_workflow_workflow_host_test_helpers::reconcile_startup(
                &restarted,
                &fixture.app,
            )
            .await
            .unwrap();
            let records = releashd::test_support::integration::workflow::read_tree_records(
                &fixture.store,
                &execution_id,
            )
            .await
            .unwrap();
            let exits = records
                .iter()
                .filter_map(|record| match &record.fact {
                    NodeFact::ProcessExited(fact) => Some(fact),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(exits.is_empty());
            assert_eq!(records.len(), before);
            assert!(!records
                .iter()
                .any(|record| matches!(record.fact, NodeFact::ArtifactProduced(_))));
        }
    }
}

#[tokio::test]
pub async fn test_終了処理_commandの保存待ちを終了し結果も後続commandも取り込まない() {
    for output in [
        Ok(CommandRunOutput {
            exit_code: 0,
            stdout: "finished".into(),
            stderr: String::new(),
            duration_ms: 1,
        }),
        Err(CommandRunnerError::Wait(std::io::Error::other(
            "wait failed",
        ))),
    ] {
        // Given
        let fixture = Fixture::new(0);
        let input = started_command(&fixture, "  main: {sequence: {children: [work, next]}}\n  work: {command: true}\n  next: {command: true}").await;
        let execution_id = input.test_execution_id().clone();
        let node_execution_id = input.test_node_execution_id().clone();
        let commit_lock = fixture.host.commit_lock(&execution_id).await;
        let executions = commit_lock.lock().await;
        let mut completion = Box::pin(async {
            match output {
                Ok(output) => {
                    fixture
                        .host
                        .commit_command_output(&fixture.app, input, output)
                        .await
                }
                Err(error) => {
                    fixture
                        .host
                        .settle_node_failure_for_node(
                            &fixture.app,
                            &input.test_execution_id(),
                            &input.test_node_execution_id(),
                            error.to_string(),
                            NodeExecutionFailureKind::InfrastructureCrash,
                        )
                        .await
                }
            }
            .unwrap();
        });
        crate::adaptor_gateway_workflow_workflow_host_test_helpers::poll_until_pending(
            completion.as_mut(),
            || Arc::strong_count(&commit_lock) > 1,
        )
        .await;

        // When
        releashd::test_support::integration::daemon::DaemonUsecase::test_with_repository(
            fixture.host.test_daemon().clone(),
        )
        .stop(releashd::test_support::integration::daemon::StopRequest::Exit { code: 23 })
        .await;
        let mut shutdown = Box::pin(fixture.host.shutdown_all_active_commands());
        assert!(futures_util::poll!(shutdown.as_mut()).is_pending());
        drop(executions);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::join!(completion, shutdown);
        })
        .await
        .unwrap();

        // Then
        let records = releashd::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &execution_id,
        )
        .await
        .unwrap();
        assert!(!records.iter().any(|record| {
            record.meta.node_execution_id == node_execution_id
                && matches!(
                    record.fact,
                    NodeFact::ProcessExited(_)
                        | NodeFact::RuntimeFailureObserved(_)
                        | NodeFact::ArtifactProduced(_)
                )
        }));
        assert!(!records.iter().any(|record| {
            record.meta.node_name == "next" && matches!(record.fact, NodeFact::CommandSpawned(_))
        }));
        assert!(!fixture
            .host
            .test_daemon()
            .admission()
            .await
            .admits(releashd::test_support::integration::daemon::DaemonRequest::Operation));
    }
}

#[tokio::test]
pub async fn test_終了処理_commandの起動待ちを終了し停止後は起動しない() {
    // Given
    let fixture = Fixture::new(0);
    let cwd = fixture._directory.path().to_str().unwrap();
    let snapshot = fixture
        .persist_started("  main:\n    command: true", cwd)
        .await;
    let node = snapshot
        .node_executions
        .iter()
        .find(|node| node.node_name == "main")
        .unwrap();
    let input = CommandExecutionInput::test_new(
        (
            snapshot.execution_id.clone(),
            node.id.clone(),
            node.node_name.clone(),
            1,
        ),
        cwd.into(),
        Some("exec sleep 60".into()),
        Vec::new(),
        None,
        BTreeMap::new(),
        None,
    );
    let commit_lock = fixture.host.commit_lock(&input.test_execution_id()).await;
    let executions = commit_lock.lock().await;
    let mut spawn = Box::pin(
        fixture
            .host
            .spawn_command_execution(&fixture.app, input.clone()),
    );
    crate::adaptor_gateway_workflow_workflow_host_test_helpers::poll_until_pending(
        spawn.as_mut(),
        || Arc::strong_count(&commit_lock) > 1,
    )
    .await;

    // When
    releashd::test_support::integration::daemon::DaemonUsecase::test_with_repository(
        fixture.host.test_daemon().clone(),
    )
    .stop(releashd::test_support::integration::daemon::StopRequest::Exit { code: 23 })
    .await;
    let mut shutdown = Box::pin(fixture.host.shutdown_all_active_commands());
    assert!(futures_util::poll!(shutdown.as_mut()).is_pending());
    drop(executions);
    spawn.await.unwrap();
    assert!(!fixture
        .host
        .node_processes
        .test_active_commands()
        .lock()
        .unwrap()
        .contains_key(&node.id));
    assert!(!fixture
        .host
        .test_command_completion_observers()
        .lock()
        .await
        .contains_key(&node.id));
    tokio::time::timeout(std::time::Duration::from_secs(12), shutdown)
        .await
        .unwrap();

    // Then
    fixture
        .host
        .spawn_command_execution(&fixture.app, input)
        .await
        .unwrap();
    assert!(fixture
        .host
        .node_processes
        .test_active_commands()
        .lock()
        .unwrap()
        .is_empty());
    assert!(fixture
        .host
        .test_command_completion_observers()
        .lock()
        .await
        .is_empty());
    assert!(fixture
        .host
        .test_active_command_executions()
        .lock()
        .await
        .is_empty());
    assert!(!fixture
        .host
        .test_daemon()
        .admission()
        .await
        .admits(releashd::test_support::integration::daemon::DaemonRequest::Operation));
    let folded = releashd::test_support::integration::workflow::fold_tree_from(
        &releashd::test_support::integration::workflow::FactLogReadBackend::Live(
            fixture.store.clone(),
        ),
        &snapshot.execution_id,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        folded
            .aggregate
            .node_executions
            .iter()
            .find(|current| current.id == node.id)
            .unwrap()
            .status,
        NodeExecutionStatus::Running
    );
}

#[tokio::test]
pub async fn test_command起動_別executionのobserver登録を待たずプロセスを登録する() {
    // Given
    let fixture = Fixture::new(0);
    let mut inputs = Vec::new();
    for index in 0..2 {
        let directory = fixture._directory.path().join(format!("worktree-{index}"));
        std::fs::create_dir(&directory).unwrap();
        let cwd = directory.to_str().unwrap();
        let snapshot = fixture
            .persist_started("  main:\n    command: true", cwd)
            .await;
        let node = snapshot
            .node_executions
            .iter()
            .find(|node| node.node_name == "main")
            .unwrap();
        inputs.push(CommandExecutionInput::test_new(
            (
                snapshot.execution_id,
                node.id.clone(),
                node.node_name.clone(),
                1,
            ),
            cwd.into(),
            Some("exec sleep 60".into()),
            Vec::new(),
            None,
            BTreeMap::new(),
            None,
        ));
    }
    let observers = fixture
        .host
        .test_command_completion_observers()
        .lock()
        .await;
    let mut first = Box::pin(
        fixture
            .host
            .spawn_command_execution(&fixture.app, inputs[0].clone()),
    );
    crate::adaptor_gateway_workflow_workflow_host_test_helpers::poll_until_pending(
        first.as_mut(),
        || {
            fixture
                .host
                .node_processes
                .test_active_commands()
                .lock()
                .unwrap()
                .contains_key(&inputs[0].test_node_execution_id())
        },
    )
    .await;
    assert!(fixture
        .host
        .node_processes
        .test_active_commands()
        .lock()
        .unwrap()
        .contains_key(&inputs[0].test_node_execution_id()));
    // When
    let mut second = Box::pin(
        fixture
            .host
            .spawn_command_execution(&fixture.app, inputs[1].clone()),
    );
    crate::adaptor_gateway_workflow_workflow_host_test_helpers::poll_until_pending(
        second.as_mut(),
        || {
            fixture
                .host
                .node_processes
                .test_active_commands()
                .lock()
                .unwrap()
                .contains_key(&inputs[1].test_node_execution_id())
        },
    )
    .await;
    let second_registered = fixture
        .host
        .node_processes
        .test_active_commands()
        .lock()
        .unwrap()
        .contains_key(&inputs[1].test_node_execution_id());
    releashd::test_support::integration::daemon::DaemonUsecase::test_with_repository(
        fixture.host.test_daemon().clone(),
    )
    .stop(releashd::test_support::integration::daemon::StopRequest::Exit { code: 23 })
    .await;
    let mut shutdown = Box::pin(fixture.host.shutdown_all_active_commands());
    assert!(futures_util::poll!(shutdown.as_mut()).is_pending());
    drop(observers);
    let (first, second) = tokio::join!(first, second);
    first.unwrap();
    second.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(12), shutdown)
        .await
        .unwrap();
    // Then
    assert!(
        second_registered,
        "independent execution waited for another command's observer registration"
    );
    assert!(fixture
        .host
        .node_processes
        .test_active_commands()
        .lock()
        .unwrap()
        .is_empty());
    assert!(fixture
        .host
        .test_command_completion_observers()
        .lock()
        .await
        .is_empty());
}

#[tokio::test]
pub async fn test_終了処理_workflowの後始末だけでは停止要求の終了コードを決めない() {
    // Given
    let fixture = Fixture::new(0);
    let daemon = releashd::test_support::integration::daemon::DaemonUsecase::test_with_repository(
        fixture.host.test_daemon().clone(),
    );
    // When
    fixture.host.shutdown_all_active_commands().await;
    // Then
    assert_eq!(
        daemon
            .stop(releashd::test_support::integration::daemon::StopRequest::Exit { code: 23 })
            .await,
        releashd::test_support::integration::daemon::StopAcceptance::Started { code: 23 }
    );
    assert_eq!(
        daemon
            .stop(releashd::test_support::integration::daemon::StopRequest::Exit { code: 99 })
            .await,
        releashd::test_support::integration::daemon::StopAcceptance::AlreadyAccepted
    );
    assert!(
        !daemon
            .admits(releashd::test_support::integration::daemon::DaemonRequest::Operation)
            .await
    );
}
