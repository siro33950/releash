use super::test_helpers::Fixture;
use super::*;
use crate::domain::workflow::NodeFact;

async fn started_command(fixture: &Fixture, nodes: &str) -> CommandExecutionInput {
    let cwd = fixture._directory.path().to_str().unwrap();
    let snapshot = fixture.persist_started(nodes, cwd).await;
    let node = snapshot
        .node_executions
        .iter()
        .find(|node| node.kind == NodeKindName::Command)
        .unwrap();
    let input = CommandExecutionInput {
        execution_id: snapshot.execution_id.clone(),
        node_execution_id: node.id.clone(),
        node_name: node.node_name.clone(),
        attempt: node.attempt,
        worktree_path: cwd.into(),
        raw_command: None,
        definition_env: Vec::new(),
        contract: None,
        schemas: BTreeMap::new(),
        session_id: None,
    };
    assert!(fixture
        .host
        .commit_command_spawned(&fixture.app, &input, "true".into())
        .await
        .unwrap());
    input
}

#[tokio::test]
async fn test_終了処理_commandの終了結果を保存せず次回起動で喪失を記録する() {
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
            let execution_id = input.execution_id.clone();
            let before = workflow_fact_log::read_tree_records(&fixture.store, &execution_id)
                .unwrap()
                .len();

            // When
            fixture.host.shutdown_all_active_commands().await;
            fixture
                .host
                .finish_command_execution(&fixture.app, input, output)
                .await;

            // Then
            let records =
                workflow_fact_log::read_tree_records(&fixture.store, &execution_id).unwrap();
            assert_eq!(
                records.len(),
                before,
                "shutdown must not append command facts"
            );
            let folded = workflow_fact_log::fold_tree_from(
                &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
                &execution_id,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                folded.aggregate.node_executions[0].status,
                NodeExecutionStatus::Running
            );
            let restarted = fixture.restarted_host();
            restarted.reconcile_startup(&fixture.app).await.unwrap();
            let records =
                workflow_fact_log::read_tree_records(&fixture.store, &execution_id).unwrap();
            let exits = records
                .iter()
                .filter_map(|record| match &record.fact {
                    NodeFact::ProcessExited(fact) => Some(fact),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(exits.len(), 1);
            assert_eq!(exits[0].exit_code, None);
            assert_eq!(
                exits[0].failure_reason.as_deref(),
                Some("process lost across application restart")
            );
            assert!(!records
                .iter()
                .any(|record| matches!(record.fact, NodeFact::ArtifactProduced(_))));
        }
    }
}

#[tokio::test]
async fn test_終了処理_commandの保存中は待ち後続commandの起動前に進む() {
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
        let input = started_command(&fixture, "  main: {sequence: {children: [{work: {on_failure: ignore}}, next]}}\n  work: {command: true}\n  next: {command: true}").await;
        let execution_id = input.execution_id.clone();
        let node_execution_id = input.node_execution_id.clone();
        let succeeded = output.is_ok();
        let executions = fixture.host.executions.lock().await;
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
                            &input.execution_id,
                            &input.node_execution_id,
                            error.to_string(),
                            NodeExecutionFailureKind::InfrastructureCrash,
                        )
                        .await
                }
            }
            .unwrap();
        });
        assert!(futures_util::poll!(completion.as_mut()).is_pending());

        // When
        let mut shutdown = Box::pin(fixture.host.shutdown_all_active_commands());
        assert!(futures_util::poll!(shutdown.as_mut()).is_pending());
        drop(executions);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::join!(completion, shutdown);
        })
        .await
        .unwrap();

        // Then
        let records = workflow_fact_log::read_tree_records(&fixture.store, &execution_id).unwrap();
        assert!(records.iter().any(|record| {
            record.meta.node_execution_id == node_execution_id
                && if succeeded {
                    matches!(record.fact, NodeFact::ProcessExited(_))
                } else {
                    matches!(record.fact, NodeFact::RuntimeFailureObserved(_))
                }
        }));
        assert!(!records.iter().any(|record| {
            record.meta.node_name == "next" && matches!(record.fact, NodeFact::CommandSpawned(_))
        }));
        assert!(!fixture
            .host
            .command_admission
            .read()
            .await
            .accepts_completion());
    }
}

#[tokio::test]
async fn test_終了処理_command起動の完了を待ち以降の起動を止める() {
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
    let input = CommandExecutionInput {
        execution_id: snapshot.execution_id.clone(),
        node_execution_id: node.id.clone(),
        node_name: node.node_name.clone(),
        attempt: 1,
        worktree_path: cwd.into(),
        raw_command: Some("exec sleep 60".into()),
        definition_env: Vec::new(),
        contract: None,
        schemas: BTreeMap::new(),
        session_id: None,
    };
    let executions = fixture.host.executions.lock().await;
    let mut spawn = Box::pin(
        fixture
            .host
            .spawn_command_execution(&fixture.app, input.clone()),
    );
    assert!(futures_util::poll!(spawn.as_mut()).is_pending());

    // When
    let mut shutdown = Box::pin(fixture.host.shutdown_all_active_commands());
    assert!(futures_util::poll!(shutdown.as_mut()).is_pending());
    drop(executions);
    spawn.await.unwrap();
    assert!(fixture
        .host
        .active_commands
        .lock()
        .await
        .contains_key(&node.id));
    assert!(fixture
        .host
        .command_completion_observers
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
    assert!(fixture.host.active_commands.lock().await.is_empty());
    assert!(fixture
        .host
        .command_completion_observers
        .lock()
        .await
        .is_empty());
    assert!(fixture
        .host
        .active_command_executions
        .lock()
        .await
        .is_empty());
    assert!(!fixture.host.command_admission.read().await.accepts_start());
    let folded = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &snapshot.execution_id,
    )
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
async fn test_command起動_別executionのobserver登録を待たずプロセスを登録する() {
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
        inputs.push(CommandExecutionInput {
            execution_id: snapshot.execution_id,
            node_execution_id: node.id.clone(),
            node_name: node.node_name.clone(),
            attempt: 1,
            worktree_path: cwd.into(),
            raw_command: Some("exec sleep 60".into()),
            definition_env: Vec::new(),
            contract: None,
            schemas: BTreeMap::new(),
            session_id: None,
        });
    }
    let observers = fixture.host.command_completion_observers.lock().await;
    let mut first = Box::pin(
        fixture
            .host
            .spawn_command_execution(&fixture.app, inputs[0].clone()),
    );
    assert!(futures_util::poll!(first.as_mut()).is_pending());
    assert!(fixture
        .host
        .active_commands
        .lock()
        .await
        .contains_key(&inputs[0].node_execution_id));
    // When
    let mut second = Box::pin(
        fixture
            .host
            .spawn_command_execution(&fixture.app, inputs[1].clone()),
    );
    assert!(futures_util::poll!(second.as_mut()).is_pending());
    let second_registered = fixture
        .host
        .active_commands
        .lock()
        .await
        .contains_key(&inputs[1].node_execution_id);
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
    assert!(fixture.host.active_commands.lock().await.is_empty());
    assert!(fixture
        .host
        .command_completion_observers
        .lock()
        .await
        .is_empty());
}
