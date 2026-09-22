use super::super::test_helpers::*;
use super::*;
use crate::adaptor::gateway::workflow::fact_codec;
use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution;
use crate::domain::workflow::NodeFact;
use crate::usecase::workflow::command::{SubmitOutputArtifact, SubmitOutputCommand};
use crate::usecase::workflow::control_plane::WorkflowControlPlaneUsecase;
use std::sync::atomic::Ordering;

fn control(fixture: &Fixture, host: &WorkflowRuntimeHost) -> WorkflowControlPlaneUsecase {
    let gateway = crate::adaptor::gateway::workflow::WorkflowRuntimeCommandGateway::new_with_driver(
        fixture.app.clone(),
        Arc::new(host.clone()),
    );
    WorkflowControlPlaneUsecase::new(Arc::new(gateway))
}

async fn submit(control: &WorkflowControlPlaneUsecase, id: &str, value: serde_json::Value) {
    control
        .submit_output(SubmitOutputCommand {
            node_execution_id: id.into(),
            artifact: Some(SubmitOutputArtifact {
                contract: "result".into(),
                value,
            }),
        })
        .await
        .unwrap();
}

async fn stop(control: &WorkflowControlPlaneUsecase, tree: &str, node: &RuntimeNodeExecution) {
    control
        .record_provider_stop(
            crate::usecase::provider_lifecycle::ProviderExecutionTreeStopCommand {
                agent_session_id: node.session_id.clone().unwrap(),
                tree_id: tree.into(),
                node_execution_id: node.id.clone(),
                binding_id: "binding".into(),
            },
            Vec::new(),
        )
        .await
        .unwrap();
}

fn definition(child_worktree: &str) -> String {
    format!("  main: {{worktree: isolated, artifact: result, session: {{provider: codex, facets: {{instruction: policy-confirmation}}}}, completion: {{delegate: {{child: check, when: child.passed, max_iterations: 2}}}}}}\n  check: {{{child_worktree}artifact: result, session: {{provider: codex, facets: {{instruction: policy-confirmation}}}}}}\nschemas:\n  result: {{type: object, properties: {{passed: {{type: boolean}}}}, required: [passed]}}")
}

#[tokio::test]
async fn test_delegate_child実行中の親再開とchild再開をまたいで継続先と再生が一致する() {
    for lost_worktree in [false, true] {
        let (fixture, root) = Fixture::with_repository();
        let tree = fixture
            .start_at(
                &definition(if lost_worktree {
                    "worktree: isolated, "
                } else {
                    ""
                }),
                &root,
            )
            .await;
        let control = control(&fixture, &fixture.host);
        let original_parent =
            fixture.host.executions.lock().await[&tree].node_executions[0].clone();
        submit(
            &control,
            &original_parent.id,
            serde_json::json!({"passed": false}),
        )
        .await;
        stop(&control, &tree, &original_parent).await;
        let child = fixture.host.executions.lock().await[&tree]
            .node_executions
            .last()
            .unwrap()
            .clone();
        let child_path = fixture.host.executions.lock().await[&tree]
            .execution_worktree_path(&child.id)
            .unwrap()
            .to_string();
        let mut parent = original_parent.clone();
        for _ in 0..2 {
            fixture
                .sessions
                .live_sessions
                .lock()
                .unwrap()
                .remove(parent.session_id.as_ref().unwrap());
            if lost_worktree {
                std::fs::remove_dir_all(&parent.worktree.as_ref().unwrap().path).unwrap();
            } else {
                fixture
                    .sessions
                    .conversation_missing
                    .store(true, Ordering::SeqCst);
            }
            control
                .resume_session_node(
                    crate::usecase::workflow::command::ResumeSessionNodeCommand {
                        execution_id: tree.clone(),
                        node_execution_id: parent.id.clone(),
                    },
                )
                .await
                .unwrap();
            parent = fixture.host.executions.lock().await[&tree]
                .node_executions
                .last()
                .unwrap()
                .clone();
        }
        assert_eq!(parent.attempt, 3);
        assert_eq!(
            fixture.host.executions.lock().await[&tree].execution_worktree_path(&child.id),
            Some(child_path.as_str())
        );
        let child = if lost_worktree {
            child
        } else {
            fixture
                .sessions
                .live_sessions
                .lock()
                .unwrap()
                .remove(child.session_id.as_ref().unwrap());
            fixture
                .sessions
                .conversation_missing
                .store(true, Ordering::SeqCst);
            control
                .resume_session_node(
                    crate::usecase::workflow::command::ResumeSessionNodeCommand {
                        execution_id: tree.clone(),
                        node_execution_id: child.id.clone(),
                    },
                )
                .await
                .unwrap();
            fixture.host.executions.lock().await[&tree]
                .node_executions
                .last()
                .unwrap()
                .clone()
        };
        fixture
            .sessions
            .conversation_missing
            .store(false, Ordering::SeqCst);
        submit(&control, &child.id, serde_json::json!({"passed": false})).await;
        stop(&control, &tree, &child).await;
        let live = fixture.host.executions.lock().await[&tree].clone();
        assert_eq!(
            live.node_execution(&child.id).unwrap().status,
            NodeExecutionStatus::Succeeded
        );
        assert_eq!(child.parent.as_ref().unwrap().parent_id, original_parent.id);
        assert!(live.pending_delegate_injections().is_empty());
        let deliveries = fixture.sessions.continuations.lock().unwrap();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(Some(&deliveries[0].0), parent.session_id.as_ref());
        let delivered: serde_json::Value =
            serde_json::from_str(deliveries[0].1.split_once("\n\n").unwrap().1).unwrap();
        assert_eq!(
            delivered["worktree"]["path"],
            parent.worktree.as_ref().unwrap().path
        );
        drop(deliveries);
        let folded = workflow_fact_log::fold_tree_from(
            &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
            &tree,
        )
        .unwrap()
        .unwrap();
        for id in [&original_parent.id, &parent.id, &child.id] {
            let actual = folded.aggregate.node_execution(id).unwrap();
            let expected = live.node_execution(id).unwrap();
            assert_eq!(actual.status, expected.status);
            assert_eq!(actual.artifact, expected.artifact);
            assert_eq!(actual.completion_signals, expected.completion_signals);
        }
        assert!(folded.aggregate.pending_delegate_injections().is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn test_submit受付_child起動の自動再試行待機より前に応答する() {
    let fixture = Fixture::new(0);
    let tree = fixture.start(&definition("")).await;
    let control = control(&fixture, &fixture.host);
    let parent = fixture.host.executions.lock().await[&tree].node_executions[0].clone();
    stop(&control, &tree, &parent).await;
    fixture
        .sessions
        .preparation_fails
        .store(true, Ordering::SeqCst);
    let before = tokio::time::Instant::now();
    submit(&control, &parent.id, serde_json::json!({"passed": false})).await;
    assert!(before.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(
        fixture.host.executions.lock().await[&tree]
            .node_executions
            .len(),
        2
    );
    assert_eq!(fixture.host.startup_retries.lock().await.len(), 1);
    fixture.wait_startup_retries().await;
    let live = fixture.host.executions.lock().await[&tree].clone();
    assert_eq!(
        live.node_executions
            .iter()
            .filter(|node| node.node_name == "check")
            .count(),
        5
    );
}

#[tokio::test]
async fn test_delegate_注入を永続化して同じsessionへ戻しchildのworktreeを親から継承する() {
    for isolated in [false, true] {
        // Given
        let fixture = Fixture::new(0);
        let tree = fixture
            .start(&definition(if isolated {
                "worktree: isolated, "
            } else {
                ""
            }))
            .await;
        let control = control(&fixture, &fixture.host);
        let parent = fixture.host.executions.lock().await[&tree].node_executions[0].clone();
        for round in 1..=2 {
            // When
            submit(&control, &parent.id, serde_json::json!({"passed": false})).await;
            let child = fixture.host.executions.lock().await[&tree]
                .node_executions
                .last()
                .unwrap()
                .clone();
            assert_eq!(child.node_name, "check");
            assert_eq!(child.attempt, round);
            stop(&control, &tree, &parent).await;
            submit(&control, &child.id, serde_json::json!({"passed": false})).await;
            stop(&control, &tree, &child).await;
            // Then
            let continuations = fixture.sessions.continuations.lock().unwrap();
            assert_eq!(continuations.len(), round as usize);
            assert_eq!(
                continuations.last().unwrap().0,
                parent.session_id.as_ref().unwrap().as_str()
            );
            let injected: serde_json::Value = serde_json::from_str(
                continuations
                    .last()
                    .unwrap()
                    .1
                    .split_once("\n\n")
                    .unwrap()
                    .1,
            )
            .unwrap();
            assert_eq!(injected["child"]["passed"], false);
            drop(continuations);
            let execution = fixture.host.executions.lock().await;
            let execution = &execution[&tree];
            let current_parent = execution.node_execution(&parent.id).unwrap();
            assert_eq!(current_parent.attempt, 1);
            assert_eq!(current_parent.session_id, parent.session_id);
            assert_eq!(current_parent.status, NodeExecutionStatus::Running);
            assert!(execution.pending_delegate_injections().is_empty());
            let parent_path = execution.execution_worktree_path(&parent.id).unwrap();
            let child_path = execution.execution_worktree_path(&child.id).unwrap();
            if isolated {
                assert_ne!(child_path, parent_path);
                let worktree = execution
                    .node_execution(&child.id)
                    .unwrap()
                    .worktree
                    .as_ref()
                    .unwrap();
                let expected =
                    serde_json::json!({"branch": worktree.branch, "path": worktree.path});
                assert_eq!(injected["child"]["worktree"], expected);
                assert_eq!(
                    current_parent.artifact.as_ref().unwrap()["child"]["worktree"],
                    expected
                );
                assert_eq!(worktree.path, child_path);
                assert!(worktree.branch.ends_with(&format!("-a{round}")));
            } else {
                assert_eq!(child_path, parent_path);
            }
        }
        let calls = fixture.worktrees.calls.lock().unwrap();
        assert_eq!(calls.len(), if isolated { 3 } else { 1 });
        assert!(calls
            .iter()
            .skip(1)
            .all(|(from, _)| from == &parent.worktree.as_ref().unwrap().path));
        drop(calls);
        submit(&control, &parent.id, serde_json::json!({"passed": false})).await;
        stop(&control, &tree, &parent).await;
        let records = workflow_fact_log::read_tree_records(&fixture.store, &tree).unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record.fact, NodeFact::DelegateResultInjected(_)))
                .count(),
            2
        );
        let folded = workflow_fact_log::fold_tree_from(
            &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
            &tree,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            folded.aggregate.node_execution(&parent.id).unwrap().status,
            NodeExecutionStatus::Succeeded
        );
        assert_eq!(fixture.sessions.prepared.lock().unwrap().len(), 3);
    }
}

#[tokio::test]
async fn test_delegate_結果注入の失敗は親をrunningに保ち注入済み事実を残さない() {
    // Given
    let fixture = Fixture::new(0);
    let tree = fixture.start(&definition("")).await;
    let control = control(&fixture, &fixture.host);
    let parent = fixture.host.executions.lock().await[&tree].node_executions[0].clone();
    submit(&control, &parent.id, serde_json::json!({"passed": false})).await;
    let child = fixture.host.executions.lock().await[&tree]
        .node_executions
        .last()
        .unwrap()
        .clone();
    stop(&control, &tree, &parent).await;
    submit(&control, &child.id, serde_json::json!({"passed": false})).await;
    fixture
        .sessions
        .continuation_fails
        .store(true, Ordering::SeqCst);
    // When
    stop(&control, &tree, &child).await;
    // Then
    let records = workflow_fact_log::read_tree_records(&fixture.store, &tree).unwrap();
    assert!(!records
        .iter()
        .any(|record| matches!(record.fact, NodeFact::DelegateResultInjected(_))));
    let folded = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &tree,
    )
    .unwrap()
    .unwrap();
    let failed = folded.aggregate.node_execution(&parent.id).unwrap();
    assert_eq!(failed.status, NodeExecutionStatus::Running);
    assert!(!failed.can_retry(crate::domain::workflow::NodeProcessPresence::ConfirmedAbsent));
}

#[tokio::test]
async fn test_delegate_再起動後のresumeは完了childを再実行せず未注入結果だけを送る() {
    for injected_before in [false, true] {
        // Given
        let fixture = Fixture::new(0);
        let tree = fixture.start(&definition("")).await;
        let initial_control = control(&fixture, &fixture.host);
        let parent = fixture.host.executions.lock().await[&tree].node_executions[0].clone();
        submit(
            &initial_control,
            &parent.id,
            serde_json::json!({"passed": false}),
        )
        .await;
        let child = fixture.host.executions.lock().await[&tree]
            .node_executions
            .last()
            .unwrap()
            .clone();
        stop(&initial_control, &tree, &parent).await;
        submit(
            &initial_control,
            &child.id,
            serde_json::json!({"passed": false}),
        )
        .await;
        if injected_before {
            stop(&initial_control, &tree, &child).await;
            fixture.sessions.live_sessions.lock().unwrap().clear();
        } else {
            workflow_fact_log::append_facts_for_events(
                &fixture.store,
                &[WorkflowEvent::NodeStopReceived {
                    execution_id: tree.clone(),
                    node_execution_id: child.id.clone(),
                    timestamp: current_timestamp(),
                }],
            )
            .unwrap();
        }
        fixture.sessions.live_sessions.lock().unwrap().clear();
        let restored = fixture.restarted_host();
        reconcile_startup(&restored, &fixture.app).await.unwrap();
        // When
        restored
            .resume_session_process(
                &fixture.app,
                &tree,
                &parent.id,
                parent.session_id.as_deref().unwrap(),
            )
            .await
            .unwrap();
        // Then
        let records = workflow_fact_log::read_tree_records(&fixture.store, &tree).unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record.fact, NodeFact::DelegateResultInjected(_)))
                .count(),
            1
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| record.meta.node_name == "check"
                    && matches!(record.fact, NodeFact::Started(_)))
                .count(),
            1
        );
        let continuations = fixture.sessions.continuations.lock().unwrap();
        assert_eq!(
            continuations
                .iter()
                .filter(|(_, instruction)| instruction.contains("\"child\""))
                .count(),
            1
        );
        assert!(continuations
            .iter()
            .all(|(session, _)| Some(session) == parent.session_id.as_ref()));
        drop(continuations);
        assert_eq!(fixture.sessions.prepared.lock().unwrap().len(), 2);
        submit(
            &control(&fixture, &restored),
            &parent.id,
            serde_json::json!({"passed": false}),
        )
        .await;
        assert_eq!(
            restored.executions.lock().await[&tree]
                .node_executions
                .last()
                .unwrap()
                .attempt,
            2
        );
    }
}

#[tokio::test]
async fn test_delegate_送信成功後の注入済み事実保存失敗からresumeしても再送しない() {
    // Given
    let fixture = Fixture::new(0);
    let tree = fixture.start(&definition("")).await;
    let initial_control = control(&fixture, &fixture.host);
    let parent = fixture.host.executions.lock().await[&tree].node_executions[0].clone();
    submit(
        &initial_control,
        &parent.id,
        serde_json::json!({"passed": false}),
    )
    .await;
    stop(&initial_control, &tree, &parent).await;
    let child = fixture.host.executions.lock().await[&tree]
        .node_executions
        .last()
        .unwrap()
        .clone();
    submit(
        &initial_control,
        &child.id,
        serde_json::json!({"passed": false}),
    )
    .await;
    workflow_fact_log::append_facts_for_events(
        &fixture.store,
        &[WorkflowEvent::NodeStopReceived {
            execution_id: tree.clone(),
            node_execution_id: child.id.clone(),
            timestamp: current_timestamp(),
        }],
    )
    .unwrap();
    let folded = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &tree,
    )
    .unwrap()
    .unwrap();
    let injection = folded
        .aggregate
        .pending_delegate_injection(&parent.id)
        .unwrap();
    fixture
        .host
        .executions
        .lock()
        .await
        .insert(tree.clone(), folded.aggregate);
    let connection =
        rusqlite::Connection::open(fixture._directory.path().join("local-event-store.sqlite3"))
            .unwrap();
    connection.execute_batch("CREATE TRIGGER fail_delegate_injected BEFORE INSERT ON node_events WHEN NEW.event_type = 'delegate_result_injected' BEGIN SELECT RAISE(ABORT, 'injected delegate fact failure'); END;").unwrap();
    assert!(fixture
        .host
        .inject_delegate_result(&fixture.app, &tree, &injection)
        .await
        .is_err());
    assert_eq!(fixture.sessions.continuations.lock().unwrap().len(), 1);
    let records = workflow_fact_log::read_tree_records(&fixture.store, &tree).unwrap();
    assert!(!records
        .iter()
        .any(|record| matches!(record.fact, NodeFact::DelegateResultInjected(_))));
    connection
        .execute_batch("DROP TRIGGER fail_delegate_injected;")
        .unwrap();
    fixture.sessions.live_sessions.lock().unwrap().clear();
    let restored = fixture.restarted_host();
    reconcile_startup(&restored, &fixture.app).await.unwrap();

    // When
    restored
        .resume_session_process(
            &fixture.app,
            &tree,
            &parent.id,
            parent.session_id.as_deref().unwrap(),
        )
        .await
        .unwrap();

    // Then
    let records = workflow_fact_log::read_tree_records(&fixture.store, &tree).unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|record| record.meta.node_name == "check"
                && matches!(record.fact, NodeFact::Started(_)))
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(record.fact, NodeFact::DelegateResultInjected(_)))
            .count(),
        1
    );
    let executions = restored.executions.lock().await;
    let execution = &executions[&tree];
    let resumed_parent = execution.node_execution(&parent.id).unwrap();
    assert_eq!(resumed_parent.status, NodeExecutionStatus::Running);
    assert_eq!(resumed_parent.session_id, parent.session_id);
    assert_eq!(resumed_parent.attempt, parent.attempt);
    assert!(execution.pending_delegate_injections().is_empty());
    assert_eq!(fixture.sessions.continuations.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_delegate_共有worktreeでもresume時のprovider復元失敗は親のattemptを維持する() {
    // Given
    let fixture = Fixture::new(0);
    let tree = fixture
        .start(&definition("").replace("worktree: isolated, ", ""))
        .await;
    let parent = fixture.host.executions.lock().await[&tree].node_executions[0].clone();
    fixture.sessions.live_sessions.lock().unwrap().clear();
    fixture
        .sessions
        .recovery_fails
        .store(true, Ordering::SeqCst);
    // When
    let result = fixture
        .host
        .resume_session_process(
            &fixture.app,
            &tree,
            &parent.id,
            parent.session_id.as_deref().unwrap(),
        )
        .await;
    // Then
    assert!(result.is_err());
    let folded = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &tree,
    )
    .unwrap()
    .unwrap();
    let failed = folded.aggregate.node_execution(&parent.id).unwrap();
    assert_eq!(failed.status, NodeExecutionStatus::Running);
    assert!(!failed.can_retry(crate::domain::workflow::NodeProcessPresence::ConfirmedAbsent));
    assert!(fixture.sessions.continuations.lock().unwrap().is_empty());
    assert_eq!(failed.attempt, parent.attempt);
}

#[tokio::test]
async fn test_delegate_完了信号が先に届いてもartifact提出まで完了せず提出でchildを始める() {
    // Given
    let fixture = Fixture::new(0);
    let tree = fixture.start(&definition("")).await;
    let control = control(&fixture, &fixture.host);
    let parent = fixture.host.executions.lock().await[&tree].node_executions[0].clone();
    // When
    control
        .submit_output(SubmitOutputCommand {
            node_execution_id: parent.id.clone(),
            artifact: None,
        })
        .await
        .unwrap();
    stop(&control, &tree, &parent).await;
    // Then
    {
        let executions = fixture.host.executions.lock().await;
        assert_eq!(executions[&tree].node_executions.len(), 1);
        assert_eq!(
            executions[&tree].node_execution(&parent.id).unwrap().status,
            NodeExecutionStatus::Running
        );
    }
    submit(&control, &parent.id, serde_json::json!({"passed": false})).await;
    assert_eq!(
        fixture.host.executions.lock().await[&tree]
            .node_executions
            .last()
            .unwrap()
            .node_name,
        "check"
    );
}

#[tokio::test]
async fn test_delegate_sequenceとfanoutのchildを提出から起動して統合mapを注入し述語で完了する() {
    for kind in ["sequence", "fanout"] {
        // Given
        let fixture = Fixture::new(0);
        let nodes = format!("  main: {{artifact: result, session: {{provider: codex, facets: {{instruction: policy-confirmation}}}}, completion: {{delegate: {{child: checks, when: child.judge.passed, max_iterations: 2}}}}}}\n  checks: {{{kind}: {{children: [judge]}}}}\n  judge: {{artifact: result, session: {{provider: codex, facets: {{instruction: policy-confirmation}}}}}}\nschemas:\n  result: {{type: object, properties: {{passed: {{type: boolean}}}}, required: [passed]}}");
        let diagnosis = crate::adaptor::gateway::workflow::diagnostics::diagnose_workflow_source(
            &format!("name: composite-delegate\ndescription: test\nnodes:\n{nodes}"),
            None,
        );
        assert!(
            diagnosis.diagnostics.is_empty(),
            "{:?}",
            diagnosis.diagnostics
        );
        let tree = fixture.start(&nodes).await;
        let control = control(&fixture, &fixture.host);
        let parent = fixture.host.executions.lock().await[&tree].node_executions[0].clone();
        for (round, passed) in [(1, false), (2, true)] {
            // When
            submit(&control, &parent.id, serde_json::json!({"passed": false})).await;
            stop(&control, &tree, &parent).await;
            let (composite, judge) = {
                let executions = fixture.host.executions.lock().await;
                let execution = &executions[&tree];
                (
                    execution
                        .node_executions
                        .iter()
                        .rev()
                        .find(|n| n.node_name == "checks")
                        .unwrap()
                        .clone(),
                    execution.node_executions.last().unwrap().clone(),
                )
            };
            assert_eq!(composite.attempt, round);
            assert_eq!(
                composite.parent,
                Some(crate::domain::workflow::ExecutionParentRef::delegate_child(
                    &parent.id
                ))
            );
            assert_eq!(judge.node_name, "judge");
            assert_eq!(judge.parent.as_ref().unwrap().parent_id, composite.id);
            assert!(fixture
                .sessions
                .activated
                .lock()
                .unwrap()
                .contains(&judge.id));
            submit(&control, &judge.id, serde_json::json!({"passed": passed})).await;
            stop(&control, &tree, &judge).await;
            // Then
            let folded = workflow_fact_log::fold_tree_from(
                &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
                &tree,
            )
            .unwrap()
            .unwrap();
            let current = folded.aggregate.node_execution(&parent.id).unwrap();
            assert_eq!(
                current.artifact.as_ref().unwrap()["child"],
                serde_json::json!({"judge": {"passed": passed}})
            );
            assert_eq!(current.session_id, parent.session_id);
            assert_eq!(current.attempt, 1);
            assert_eq!(
                current.status,
                if passed {
                    NodeExecutionStatus::Succeeded
                } else {
                    NodeExecutionStatus::Running
                }
            );
            let continuations = fixture.sessions.continuations.lock().unwrap();
            assert_eq!(continuations.len(), 1);
            let injected: serde_json::Value =
                serde_json::from_str(continuations[0].1.split_once("\n\n").unwrap().1).unwrap();
            assert_eq!(
                injected["child"],
                serde_json::json!({"judge": {"passed": false}})
            );
        }
    }
}

#[tokio::test]
async fn test_delegate_未完了childを持つ再起動resumeは既存childだけを再開し親を待機させる() {
    // Given
    let fixture = Fixture::new(0);
    let tree = fixture.start(&definition("")).await;
    let initial_control = control(&fixture, &fixture.host);
    let parent = fixture.host.executions.lock().await[&tree].node_executions[0].clone();
    submit(
        &initial_control,
        &parent.id,
        serde_json::json!({"passed": false}),
    )
    .await;
    stop(&initial_control, &tree, &parent).await;
    let child = fixture.host.executions.lock().await[&tree]
        .node_executions
        .last()
        .unwrap()
        .clone();
    fixture.sessions.live_sessions.lock().unwrap().clear();
    fixture.sessions.live_sessions.lock().unwrap().clear();
    let restored = fixture.restarted_host();
    reconcile_startup(&restored, &fixture.app).await.unwrap();
    // When
    restored
        .resume_session_process(
            &fixture.app,
            &tree,
            &child.id,
            child.session_id.as_deref().unwrap(),
        )
        .await
        .unwrap();
    // Then
    {
        let executions = restored.executions.lock().await;
        let execution = &executions[&tree];
        assert_eq!(execution.node_executions.len(), 2);
        assert!(execution.delegate_waits_for_child(&parent.id));
        assert_eq!(
            execution.node_execution(&parent.id).unwrap().session_id,
            parent.session_id
        );
        assert_eq!(
            execution.node_execution(&child.id).unwrap().status,
            NodeExecutionStatus::Running
        );
        assert_eq!(execution.node_execution(&child.id).unwrap().attempt, 1);
    }
    assert!(fixture.sessions.continuations.lock().unwrap().is_empty());
    assert!(fixture
        .sessions
        .recovered
        .lock()
        .unwrap()
        .contains(&child.id));
    assert_eq!(fixture.sessions.prepared.lock().unwrap().len(), 2);
    let resumed_control = control(&fixture, &restored);
    submit(
        &resumed_control,
        &child.id,
        serde_json::json!({"passed": false}),
    )
    .await;
    stop(&resumed_control, &tree, &child).await;
    assert_eq!(fixture.sessions.continuations.lock().unwrap().len(), 1);
    let records = workflow_fact_log::read_tree_records(&fixture.store, &tree).unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|r| r.meta.node_name == "check" && matches!(r.fact, NodeFact::Started(_)))
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|r| matches!(r.fact, NodeFact::DelegateResultInjected(_)))
            .count(),
        1
    );
}

#[tokio::test]
async fn test_delegate_同じpendingを並行注入しても送信とcommitは一度になる() {
    // Given
    let fixture = Fixture::new(0);
    let tree = fixture.start(&definition("")).await;
    let control = control(&fixture, &fixture.host);
    let parent = fixture.host.executions.lock().await[&tree].node_executions[0].clone();
    submit(&control, &parent.id, serde_json::json!({"passed": false})).await;
    stop(&control, &tree, &parent).await;
    let child = fixture.host.executions.lock().await[&tree]
        .node_executions
        .last()
        .unwrap()
        .clone();
    submit(&control, &child.id, serde_json::json!({"passed": false})).await;
    workflow_fact_log::append_facts_for_events(
        &fixture.store,
        &[WorkflowEvent::NodeStopReceived {
            execution_id: tree.clone(),
            node_execution_id: child.id,
            timestamp: current_timestamp(),
        }],
    )
    .unwrap();
    let folded = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &tree,
    )
    .unwrap()
    .unwrap();
    let injection = folded
        .aggregate
        .pending_delegate_injection(&parent.id)
        .unwrap();
    fixture
        .host
        .executions
        .lock()
        .await
        .insert(tree.clone(), folded.aggregate);
    fixture
        .sessions
        .block_continuation
        .store(true, Ordering::SeqCst);
    // When
    let dependencies = fixture.app.clone();
    let first = fixture
        .host
        .inject_delegate_result(&dependencies, &tree, &injection);
    let second = async {
        fixture.sessions.continuation_entered.notified().await;
        let second = fixture
            .host
            .inject_delegate_result(&dependencies, &tree, &injection);
        tokio::pin!(second);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(25), &mut second)
                .await
                .is_err()
        );
        assert_eq!(fixture.sessions.recovered.lock().unwrap().len(), 1);
        fixture
            .sessions
            .block_continuation
            .store(false, Ordering::SeqCst);
        fixture.sessions.continuation_release.notify_one();
        second.await
    };
    let (first, second) = tokio::join!(first, second);
    // Then
    first.unwrap();
    second.unwrap();
    assert_eq!(fixture.sessions.continuations.lock().unwrap().len(), 1);
    let records = workflow_fact_log::read_tree_records(&fixture.store, &tree).unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|r| matches!(r.fact, NodeFact::DelegateResultInjected(_)))
            .count(),
        1
    );
}

#[tokio::test]
async fn test_delegate_隔離合成子の準備中に空のchildが完了したら準備後に注入する() {
    // Given
    let fixture = Fixture::new(0);
    let tree = fixture.start("  main: {artifact: result, session: {provider: codex, facets: {instruction: policy-confirmation}}, completion: {delegate: {child: checks, when: passed, max_iterations: 1}}}\n  checks: {worktree: isolated, fanout: {items: [], children: [{judge: {inputs: {item: items}}}]}}\n  judge: {input: [item], artifact: result, session: {provider: codex, facets: {instruction: policy-confirmation}}}\nschemas:\n  result: {type: object, properties: {passed: {type: boolean}}, required: [passed]}").await;
    let control = control(&fixture, &fixture.host);
    let parent = fixture.host.executions.lock().await[&tree].node_executions[0].clone();
    stop(&control, &tree, &parent).await;
    // When
    submit(&control, &parent.id, serde_json::json!({"passed": false})).await;
    // Then
    let executions = fixture.host.executions.lock().await;
    let execution = &executions[&tree];
    assert_eq!(execution.node_executions.len(), 2);
    assert_eq!(
        execution.node_executions[1].status,
        NodeExecutionStatus::Succeeded
    );
    assert_eq!(
        execution.node_executions[0].status,
        NodeExecutionStatus::Running
    );
    assert!(execution.pending_delegate_injections().is_empty());
    let instructions = fixture.sessions.continuations.lock().unwrap();
    assert_eq!(instructions.len(), 1);
    let value: serde_json::Value =
        serde_json::from_str(instructions[0].1.split_once("\n\n").unwrap().1).unwrap();
    assert!(value["child"]["worktree"].is_object());
    assert!(value["child"].get("judge").is_none());
    assert_eq!(fixture.sessions.prepared.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_delegate_組み立てが欠けた入口は送信せずエラーを返す() {
    // Given
    let fixture = Fixture::new(0);
    let mut host = fixture.host.clone();
    host.delegate_continuation = None;
    let injection = crate::domain::workflow::entities::workflow_execution::DelegateInjection {
        node_execution_id: "parent".into(),
        child_execution_id: "child".into(),
    };
    // When
    let error = host
        .inject_delegate_result(&fixture.app, "tree", &injection)
        .await
        .unwrap_err();
    // Then
    assert!(matches!(error, WorkflowRuntimeError::InvalidState(_)));
    assert!(fixture.sessions.continuations.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_delegate_child待ち中の再submitは状態拒否となり保存済み成果を保持する() {
    // Given
    let fixture = Fixture::new(0);
    let tree = fixture.start(&definition("")).await;
    let control = control(&fixture, &fixture.host);
    let parent = fixture.host.executions.lock().await[&tree].node_executions[0].clone();
    submit(&control, &parent.id, serde_json::json!({"passed": false})).await;
    let before = workflow_fact_log::read_tree_records(&fixture.store, &tree).unwrap();
    // When
    let error = control
        .submit_output(SubmitOutputCommand {
            node_execution_id: parent.id.clone(),
            artifact: Some(SubmitOutputArtifact {
                contract: "result".into(),
                value: serde_json::json!({"passed": true}),
            }),
        })
        .await
        .unwrap_err();
    // Then
    assert_eq!(
        error.to_string(),
        format!(
            "invalid_state: node execution '{}' cannot accept Artifact in its current state",
            parent.id
        )
    );
    assert_eq!(
        workflow_fact_log::read_tree_records(&fixture.store, &tree).unwrap(),
        before
    );
    let executions = fixture.host.executions.lock().await;
    let execution = &executions[&tree];
    assert_eq!(
        execution.node_execution(&parent.id).unwrap().status,
        NodeExecutionStatus::Running
    );
    assert_eq!(
        execution
            .node_execution(&parent.id)
            .unwrap()
            .artifact
            .as_ref()
            .unwrap()["passed"],
        false
    );
    assert!(execution.delegate_waits_for_child(&parent.id));
}

#[tokio::test]
async fn test_delegate_childの新attemptへのresumeは親を待機させ注入と次の発火を再生と一致させる() {
    // Given
    let fixture = Fixture::new(0);
    let tree = fixture.start(&definition("")).await;
    let control = control(&fixture, &fixture.host);
    let parent = fixture.host.executions.lock().await[&tree].node_executions[0].clone();
    submit(&control, &parent.id, serde_json::json!({"passed": false})).await;
    stop(&control, &tree, &parent).await;
    let first = fixture.host.executions.lock().await[&tree]
        .node_executions
        .last()
        .unwrap()
        .clone();
    fixture
        .host
        .settle_runtime_failure_for_node(
            &fixture.app,
            &tree,
            &first.id,
            &WorkflowRuntimeError::AgentSession("child provider failed".into()),
        )
        .await
        .unwrap();
    fixture.sessions.live_sessions.lock().unwrap().clear();
    // When
    control
        .resume_session_node(
            crate::usecase::workflow::command::ResumeSessionNodeCommand {
                execution_id: tree.clone(),
                node_execution_id: first.id.clone(),
            },
        )
        .await
        .unwrap();
    let retry = fixture.host.executions.lock().await[&tree]
        .node_executions
        .last()
        .unwrap()
        .clone();
    // Then
    assert_eq!(retry.attempt, 2);
    assert_ne!(retry.id, first.id);
    for child in [&first, &retry] {
        assert_eq!(
            child.parent,
            Some(crate::domain::workflow::ExecutionParentRef::delegate_child(
                &parent.id
            ))
        );
    }
    let folded = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &tree,
    )
    .unwrap()
    .unwrap();
    assert!(folded.aggregate.delegate_waits_for_child(&parent.id));
    assert_eq!(
        folded.aggregate.node_execution(&parent.id).unwrap().status,
        NodeExecutionStatus::Running
    );
    assert_eq!(
        folded.aggregate.node_execution(&retry.id).unwrap().status,
        NodeExecutionStatus::Running
    );
    // When
    submit(&control, &retry.id, serde_json::json!({"passed": false})).await;
    stop(&control, &tree, &retry).await;
    // Then
    let records = workflow_fact_log::read_tree_records(&fixture.store, &tree).unwrap();
    assert!(records
        .iter()
        .any(|record| record.fact == NodeFact::DelegateResultInjected(retry.id.clone())));
    assert_eq!(fixture.sessions.continuations.lock().unwrap().len(), 1);
    assert_eq!(
        fixture.sessions.continuations.lock().unwrap()[0].0,
        parent.session_id.clone().unwrap()
    );
    // When: Retry は delegate の発火上限を消費しない
    let mut replayed = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &tree,
    )
    .unwrap()
    .unwrap()
    .aggregate;
    replayed.record_node_completion_signal(
        &parent.id,
        crate::domain::workflow::NodeCompletionSignal::Submit,
        current_timestamp(),
    );
    assert_eq!(
        replayed.apply_submitted_output(
            "main".into(),
            &parent.id,
            1,
            parent.session_id.clone(),
            "result".into(),
            serde_json::json!({"passed": false}),
            None,
            current_timestamp()
        ),
        TransitionOutcome::Applied
    );
    let advance = replayed
        .apply_node_completion_handshake(
            &parent.id,
            &mut || "next-child".into(),
            current_timestamp(),
        )
        .unwrap();
    assert!(matches!(advance.advance, Some(crate::domain::workflow::entities::workflow_execution::ExecutionAdvanceDecision::StartNodes(_))));
    assert_eq!(replayed.node_execution("next-child").unwrap().attempt, 3);
    submit(&control, &parent.id, serde_json::json!({"passed": false})).await;
    let second = fixture.host.executions.lock().await[&tree]
        .node_executions
        .last()
        .unwrap()
        .clone();
    // Then
    assert_eq!(second.attempt, 3);
    assert_ne!(second.id, retry.id);
    let folded = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &tree,
    )
    .unwrap()
    .unwrap();
    let executions = fixture.host.executions.lock().await;
    let live = &executions[&tree];
    assert!(live.delegate_waits_for_child(&parent.id));
    assert!(folded.aggregate.delegate_waits_for_child(&parent.id));
    assert_replayed_nodes(live, &folded.aggregate);
    drop(executions);
    // When
    stop(&control, &tree, &parent).await;
    submit(&control, &second.id, serde_json::json!({"passed": false})).await;
    stop(&control, &tree, &second).await;
    submit(&control, &parent.id, serde_json::json!({"passed": false})).await;
    stop(&control, &tree, &parent).await;
    // Then
    let folded = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &tree,
    )
    .unwrap()
    .unwrap();
    assert!(!fixture.host.executions.lock().await.contains_key(&tree));
    assert_eq!(
        folded.aggregate.node_execution(&parent.id).unwrap().status,
        NodeExecutionStatus::Succeeded
    );
    assert_eq!(fixture.sessions.continuations.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn test_delegate_artifactを省略したisolated_session_childのworktree成果を注入する() {
    // Given
    let fixture = Fixture::new(0);
    let nodes = definition("worktree: isolated, ")
        .replace("when: child.passed", "when: passed")
        .replace(
            "check: {worktree: isolated, artifact: result,",
            "check: {worktree: isolated,",
        );
    let tree = fixture.start(&nodes).await;
    let control = control(&fixture, &fixture.host);
    let parent = fixture.host.executions.lock().await[&tree].node_executions[0].clone();
    submit(&control, &parent.id, serde_json::json!({"passed": false})).await;
    stop(&control, &tree, &parent).await;
    let child = fixture.host.executions.lock().await[&tree]
        .node_executions
        .last()
        .unwrap()
        .clone();
    // When
    control
        .submit_output(SubmitOutputCommand {
            node_execution_id: child.id.clone(),
            artifact: None,
        })
        .await
        .unwrap();
    stop(&control, &tree, &child).await;
    // Then
    let worktree = child.worktree.as_ref().unwrap();
    let expected =
        serde_json::json!({"worktree": {"branch": worktree.branch, "path": worktree.path}});
    let folded = workflow_fact_log::fold_tree_from(
        &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
        &tree,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        folded
            .aggregate
            .node_execution(&parent.id)
            .unwrap()
            .artifact
            .as_ref()
            .unwrap()["child"],
        expected
    );
    let continuations = fixture.sessions.continuations.lock().unwrap();
    assert_eq!(continuations.len(), 1);
    let injected: serde_json::Value =
        serde_json::from_str(continuations[0].1.split_once("\n\n").unwrap().1).unwrap();
    assert_eq!(injected["child"], expected);
}

#[tokio::test]
async fn test_delegate_非空inputsを保存した実行は再起動resume後も最新の供給元を解決する() {
    // Given
    let fixture = Fixture::new(0);
    let source = r#"
name: bindings
description: test
nodes:
  main:
    sequence:
      children:
        - worker: {inputs: {spec: request}}
  worker:
    input: [spec]
    session: {provider: codex, facets: {instruction: policy-confirmation}}
    artifact: result
    completion:
      delegate:
        child: check
        inputs: {spec: spec, task: worker.task, previous: worker.child.passed, requested: request}
        when: child.passed
        max_iterations: 2
  check:
    input: [spec, task, previous, requested]
    session: {provider: codex, facets: {instruction: policy-confirmation}}
    artifact: result
schemas:
  result: {type: object, properties: {passed: {type: boolean}, task: {type: string}}, required: [passed, task]}
"#;
    let definition: WorkflowDefinition = serde_saphyr::from_str(source).unwrap();
    let expected_inputs = definition
        .node_by_name("worker")
        .unwrap()
        .completion
        .delegate
        .as_ref()
        .unwrap()
        .inputs
        .clone();
    let tree = fixture
        .host
        .start_resolved_workflow(
            &fixture.app,
            definition,
            "/repo-worktrees/development".into(),
            Some("specification".into()),
            ExecutionOrigin::Cli,
        )
        .await
        .unwrap();
    let initial_control = control(&fixture, &fixture.host);
    let parent = fixture.host.executions.lock().await[&tree]
        .node_executions
        .iter()
        .find(|node| node.node_name == "worker")
        .unwrap()
        .clone();
    submit(
        &initial_control,
        &parent.id,
        serde_json::json!({"passed": false, "task": "first"}),
    )
    .await;
    stop(&initial_control, &tree, &parent).await;
    let child = fixture.host.executions.lock().await[&tree]
        .node_executions
        .last()
        .unwrap()
        .clone();
    submit(
        &initial_control,
        &child.id,
        serde_json::json!({"passed": false, "task": "reviewed"}),
    )
    .await;
    workflow_fact_log::append_facts_for_events(
        &fixture.store,
        &[WorkflowEvent::NodeStopReceived {
            execution_id: tree.clone(),
            node_execution_id: child.id.clone(),
            timestamp: current_timestamp(),
        }],
    )
    .unwrap();
    let records = workflow_fact_log::read_tree_records(&fixture.store, &tree).unwrap();
    let root = fact_codec::decode(
        fact_codec::event_type(&records[0].fact),
        &fact_codec::encode_detail(&records[0].fact).unwrap(),
    )
    .unwrap();
    let NodeFact::Started(started) = root else {
        panic!("root must start the tree")
    };
    assert_eq!(
        started
            .root
            .unwrap()
            .definition
            .as_ref()
            .unwrap()
            .node_by_name("worker")
            .unwrap()
            .completion
            .delegate
            .as_ref()
            .unwrap()
            .inputs,
        expected_inputs
    );
    fixture.sessions.live_sessions.lock().unwrap().clear();
    let restored = fixture.restarted_host();
    reconcile_startup(&restored, &fixture.app).await.unwrap();
    // When
    restored
        .resume_session_process(
            &fixture.app,
            &tree,
            &parent.id,
            parent.session_id.as_deref().unwrap(),
        )
        .await
        .unwrap();
    submit(
        &control(&fixture, &restored),
        &parent.id,
        serde_json::json!({"passed": false, "task": "second"}),
    )
    .await;
    // Then
    let executions = restored.executions.lock().await;
    let execution = &executions[&tree];
    let child = execution.node_executions.last().unwrap();
    assert_eq!(child.attempt, 2);
    assert_eq!(
        execution
            .workflow
            .as_ref()
            .unwrap()
            .node_by_name("worker")
            .unwrap()
            .completion
            .delegate
            .as_ref()
            .unwrap()
            .inputs,
        expected_inputs
    );
    let bindings: BTreeMap<_, _> = execution
        .leaf_start_for(&child.id)
        .unwrap()
        .bindings
        .into_iter()
        .collect();
    assert_eq!(
        bindings,
        BTreeMap::from([
            ("spec".into(), serde_json::json!("specification")),
            ("task".into(), serde_json::json!("second")),
            ("previous".into(), serde_json::json!(false)),
            ("requested".into(), serde_json::json!("specification")),
        ])
    );
    assert_eq!(fixture.sessions.continuations.lock().unwrap().len(), 1);
}

fn assert_replayed_nodes(live: &DomainExecutionTree, replayed: &DomainExecutionTree) {
    let mut persisted_nodes = live.node_executions.clone();
    for node in &mut persisted_nodes {
        node.started_at = (node.started_at * 1000.0) as i64 as f64 / 1000.0;
        node.completed_at = node
            .completed_at
            .map(|time| (time * 1000.0) as i64 as f64 / 1000.0);
    }
    assert_eq!(persisted_nodes, replayed.node_executions);
}

#[tokio::test]
async fn test_delegate_false_childが親stopより先に完了しても再開後のstopで一度だけ注入する() {
    for interrupted in [false, true] {
        // Given
        let fixture = Fixture::new(0);
        let tree = fixture.start(&definition("")).await;
        let initial_control = control(&fixture, &fixture.host);
        let parent = fixture.host.executions.lock().await[&tree].node_executions[0].clone();
        submit(
            &initial_control,
            &parent.id,
            serde_json::json!({"passed": false}),
        )
        .await;
        let child = fixture.host.executions.lock().await[&tree]
            .node_executions
            .last()
            .unwrap()
            .clone();
        submit(
            &initial_control,
            &child.id,
            serde_json::json!({"passed": false}),
        )
        .await;
        stop(&initial_control, &tree, &child).await;
        assert!(fixture.sessions.continuations.lock().unwrap().is_empty());
        assert!(fixture.host.executions.lock().await[&tree]
            .pending_delegate_injections()
            .is_empty());
        let restored = if interrupted {
            fixture.sessions.live_sessions.lock().unwrap().clear();
            let restored = fixture.restarted_host();
            reconcile_startup(&restored, &fixture.app).await.unwrap();
            restored
                .resume_session_process(
                    &fixture.app,
                    &tree,
                    &parent.id,
                    parent.session_id.as_deref().unwrap(),
                )
                .await
                .unwrap();
            restored
        } else {
            fixture.host.clone()
        };
        assert!(restored.executions.lock().await[&tree]
            .pending_delegate_injections()
            .is_empty());
        assert!(fixture
            .sessions
            .continuations
            .lock()
            .unwrap()
            .iter()
            .all(|(_, text)| !text.contains("\"child\"")));
        // When
        let resumed_control = control(&fixture, &restored);
        stop(&resumed_control, &tree, &parent).await;
        stop(&resumed_control, &tree, &parent).await;
        // Then
        let records = workflow_fact_log::read_tree_records(&fixture.store, &tree).unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|record| record.meta.node_name == "check"
                    && matches!(record.fact, NodeFact::Started(_)))
                .count(),
            1
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| record.fact == NodeFact::DelegateResultInjected(child.id.clone()))
                .count(),
            1
        );
        let executions = restored.executions.lock().await;
        let execution = &executions[&tree];
        assert_eq!(
            execution.node_execution(&child.id).unwrap().status,
            NodeExecutionStatus::Succeeded
        );
        let current_parent = execution.node_execution(&parent.id).unwrap();
        assert_eq!(current_parent.attempt, parent.attempt);
        assert_eq!(current_parent.session_id, parent.session_id);
        assert_eq!(
            current_parent.artifact.as_ref().unwrap()["child"]["passed"],
            false
        );
        let continuations = fixture.sessions.continuations.lock().unwrap();
        let delivered = continuations
            .iter()
            .filter(|(_, text)| text.contains("\"child\""))
            .collect::<Vec<_>>();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].0, parent.session_id.clone().unwrap());
        let value: serde_json::Value =
            serde_json::from_str(delivered[0].1.split_once("\n\n").unwrap().1).unwrap();
        assert_eq!(value["child"]["passed"], false);
    }
}

#[tokio::test]
async fn test_delegate_新attemptのresumeでも未注入結果を送り再生後も注入済みになる() {
    for lost_worktree in [false, true] {
        // Given
        let (fixture, root) = Fixture::with_repository();
        let tree = fixture.start_at(&definition(""), &root).await;
        let control = control(&fixture, &fixture.host);
        let parent = fixture.host.executions.lock().await[&tree].node_executions[0].clone();
        submit(&control, &parent.id, serde_json::json!({"passed": false})).await;
        stop(&control, &tree, &parent).await;
        let child = fixture.host.executions.lock().await[&tree]
            .node_executions
            .last()
            .unwrap()
            .clone();
        submit(&control, &child.id, serde_json::json!({"passed": false})).await;
        fixture
            .sessions
            .continuation_fails
            .store(true, Ordering::SeqCst);
        stop(&control, &tree, &child).await;
        fixture.sessions.live_sessions.lock().unwrap().clear();
        if lost_worktree {
            std::fs::remove_dir_all(&parent.worktree.as_ref().unwrap().path).unwrap();
        } else {
            fixture
                .sessions
                .conversation_missing
                .store(true, Ordering::SeqCst);
        }
        fixture
            .sessions
            .continuation_fails
            .store(false, Ordering::SeqCst);
        // When
        control
            .resume_session_node(
                crate::usecase::workflow::command::ResumeSessionNodeCommand {
                    execution_id: tree.clone(),
                    node_execution_id: parent.id.clone(),
                },
            )
            .await
            .unwrap();
        // Then
        let live = fixture.host.executions.lock().await[&tree].clone();
        let next = live.node_executions.last().unwrap();
        assert_ne!(next.id, parent.id);
        assert_eq!(next.attempt, 2);
        assert_eq!(next.status, NodeExecutionStatus::Running);
        assert!(live.pending_delegate_injections().is_empty());
        let deliveries = fixture.sessions.continuations.lock().unwrap();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(Some(&deliveries[0].0), next.session_id.as_ref());
        let artifact: serde_json::Value =
            serde_json::from_str(deliveries[0].1.split_once("\n\n").unwrap().1).unwrap();
        assert_eq!(artifact["child"]["passed"], false);
        drop(deliveries);
        assert_eq!(
            fixture.sessions.initial_instructions.lock().unwrap().len(),
            3
        );
        let folded = workflow_fact_log::fold_tree_from(
            &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
            &tree,
        )
        .unwrap()
        .unwrap();
        let replayed = folded.aggregate.node_execution(&next.id).unwrap();
        assert_eq!(replayed.status, next.status);
        assert_eq!(replayed.session_id, next.session_id);
        assert_eq!(replayed.attempt, next.attempt);
        assert_eq!(replayed.worktree, next.worktree);
        assert_eq!(replayed.artifact, next.artifact);
        assert_eq!(replayed.completion_signals, next.completion_signals);
        assert!(folded.aggregate.pending_delegate_injections().is_empty());
        let records = workflow_fact_log::read_tree_records(&fixture.store, &tree).unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|record| record.meta.node_name == "check"
                    && matches!(record.fact, NodeFact::Started(_)))
                .count(),
            1
        );
        assert_eq!(records.iter().filter(|record| record.meta.node_execution_id == next.id && matches!(&record.fact, NodeFact::DelegateResultInjected(id) if id == &child.id)).count(), 1);
    }
}
