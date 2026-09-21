use super::*;
use serde_json::{json, Value};

#[test]
fn test_workflow状態復元_三状態をそのまま復元する() {
    for state in [
        RuntimeExecutionState::Running,
        RuntimeExecutionState::Completed,
        RuntimeExecutionState::Aborted,
    ] {
        // Given
        let restore = WorkflowExecutionRestore {
            state: state.clone(),
            ..Default::default()
        };

        // When
        let execution = WorkflowExecution::restore_runtime(restore);

        // Then
        assert_eq!(execution.state(), &state);
    }
}

fn execution(yaml: &str) -> WorkflowExecution {
    WorkflowExecution::restore_runtime(WorkflowExecutionRestore {
        id: "execution".to_string(),
        workflow: serde_saphyr::from_str(yaml).unwrap(),
        ..Default::default()
    })
}

fn id_source() -> impl FnMut() -> String {
    let mut counter = 0;
    move || {
        counter += 1;
        format!("node-{counter}")
    }
}

fn finish_leaf(
    execution: &mut WorkflowExecution,
    leaf: &LeafStart,
    artifact: Option<Value>,
    new_id: &mut dyn FnMut() -> String,
) -> AppliedAdvance {
    assert_eq!(
        execution.record_pending_result(
            &leaf.node_execution_id,
            Some("leaf result".to_string()),
            artifact,
            Some("leaf-contract".to_string()),
            Some(TokenUsage::default()),
            2.0,
        ),
        TransitionOutcome::Applied
    );
    if leaf.kind == LeafKind::Session {
        for signal in [NodeCompletionSignal::Submit, NodeCompletionSignal::Stop] {
            assert_eq!(
                execution.record_node_completion_signal(&leaf.node_execution_id, signal, 3.0),
                TransitionOutcome::Applied
            );
        }
        let applied = execution
            .apply_node_completion_handshake(&leaf.node_execution_id, new_id, 3.0)
            .unwrap();
        return AppliedAdvance {
            decision: applied.advance.expect("session must advance"),
            events: applied.events,
        };
    }
    execution
        .complete_leaf_and_advance(&leaf.node_execution_id, new_id, 3.0)
        .unwrap()
}

fn next_leaf(decision: ExecutionAdvanceDecision) -> LeafStart {
    let ExecutionAdvanceDecision::StartNodes(mut leaves) = decision else {
        panic!("expected a leaf start, got {decision:?}");
    };
    assert_eq!(leaves.len(), 1);
    match leaves.remove(0) {
        NodeStart::Leaf(leaf) => leaf,
        _ => panic!("expected leaf start"),
    }
}

#[test]
fn test_sequenceの成果_通って成果を産出した子だけをmapに統合する() {
    // Given
    let mut execution = execution(
        r#"
name: merged-artifact
description: test
nodes:
  main:
    sequence:
      children: [part, report]
  part:
    sequence:
      entry: a
      children:
        - z:
            rules:
              - when: {on: visit_skipped, then: skipped}
                next: silent
        - a:
            rules: [{next: z}]
        - skipped
        - silent
  z: {session: {provider: codex}}
  a: {session: {provider: codex}}
  skipped: {session: {provider: codex}}
  silent: {session: {provider: codex}}
  report: {session: {provider: codex}}
"#,
    );
    let mut new_id = id_source();
    let leaf = next_leaf(execution.start_root(&mut new_id, 1.0).unwrap().decision);

    // When
    assert_eq!(leaf.node_name, "a");
    let leaf = next_leaf(
        finish_leaf(
            &mut execution,
            &leaf,
            Some(json!({"value": 42})),
            &mut new_id,
        )
        .decision,
    );
    assert_eq!(leaf.node_name, "z");
    let leaf = next_leaf(
        finish_leaf(
            &mut execution,
            &leaf,
            Some(json!({"visit_skipped": false})),
            &mut new_id,
        )
        .decision,
    );
    assert_eq!(leaf.node_name, "silent");
    let completed = finish_leaf(&mut execution, &leaf, None, &mut new_id);

    // Then
    assert_eq!(next_leaf(completed.decision).node_name, "report");
    let part = execution
        .node_executions()
        .iter()
        .find(|node| node.node_name == "part")
        .unwrap();
    let expected = json!({"z": {"visit_skipped": false}, "a": {"value": 42}});
    assert_eq!(part.artifact, Some(expected.clone()));
    assert_eq!(
        part.artifact
            .as_ref()
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["z", "a"])
    );
    assert_eq!(part.result_summary, None);
    assert_eq!(part.token_usage, None);
    assert_eq!(part.status, RuntimeNodeExecutionStatus::Succeeded);
    let artifact = execution.flattened_artifacts().remove("part").unwrap();
    assert_eq!(artifact.contract, None);
    assert_eq!(artifact.result, None);
    assert_eq!(artifact.token_usage, None);
    assert!(completed.events.iter().any(|event| matches!(event,
        WorkflowEvent::ArtifactProduced { node_name, contract: None, value, .. }
            if node_name == "part" && value == &expected
    )));
}

#[test]
fn test_sequenceの成果_成果を持たない子だけなら空mapで完了する() {
    // Given
    let mut execution = execution(
        r#"
name: empty-artifact
description: test
nodes:
  main: {sequence: {children: [silent]}}
  silent: {session: {provider: codex}}
"#,
    );
    let mut new_id = id_source();
    let leaf = next_leaf(execution.start_root(&mut new_id, 1.0).unwrap().decision);

    // When
    let completed = finish_leaf(&mut execution, &leaf, None, &mut new_id);

    // Then
    assert_eq!(*execution.state(), RuntimeExecutionState::Completed);
    assert_eq!(execution.node_executions()[0].artifact, Some(json!({})));
    assert!(!completed
        .events
        .iter()
        .any(|event| matches!(event, WorkflowEvent::NodeFailed { .. })));
}

#[test]
fn test_sequenceの成果_後方辺で再訪した子は最後の成果だけを残す() {
    // Given
    let mut execution = execution(
        r#"
name: loop-artifact
description: test
nodes:
  main:
    sequence:
      children:
        - repeat:
            rules:
              - loop_guard: {max_iterations: 2, on_exhausted: silent}
              - next: repeat
        - silent
  repeat: {session: {provider: codex}}
  silent: {session: {provider: codex}}
"#,
    );
    let mut new_id = id_source();
    let leaf = next_leaf(execution.start_root(&mut new_id, 1.0).unwrap().decision);

    // When
    let leaf = next_leaf(
        finish_leaf(
            &mut execution,
            &leaf,
            Some(json!({"attempt": 1})),
            &mut new_id,
        )
        .decision,
    );
    assert_eq!(leaf.node_name, "repeat");
    let leaf = next_leaf(
        finish_leaf(
            &mut execution,
            &leaf,
            Some(json!({"attempt": 2})),
            &mut new_id,
        )
        .decision,
    );
    assert_eq!(leaf.node_name, "silent");
    finish_leaf(&mut execution, &leaf, None, &mut new_id);

    // Then
    assert_eq!(*execution.state(), RuntimeExecutionState::Completed);
    assert_eq!(
        execution.node_executions()[0].artifact,
        Some(json!({"repeat": {"attempt": 2}}))
    );
}

#[test]
fn test_sequenceの多段参照_配線と辺とfanout展開へ統合mapの値を渡す() {
    // Given
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/adaptor/gateway/workflow/fixtures/valid/sequence-merged-references.yml"
    ));
    for (has_open_threads, status) in [(false, "READY"), (true, "HOLD"), (true, "READY")] {
        let mut execution = execution(source);
        crate::domain::workflow::services::validation::validate(&execution.workflow).unwrap();
        let mut new_id = id_source();
        let leaf = next_leaf(execution.start_root(&mut new_id, 1.0).unwrap().decision);
        let scan = json!({"ok": true, "has_open_threads": has_open_threads, "status": status, "tasks": ["first", "second"]});

        // When
        let leaf =
            next_leaf(finish_leaf(&mut execution, &leaf, Some(scan.clone()), &mut new_id).decision);

        // Then
        if !has_open_threads {
            assert_eq!(leaf.node_name, "finished");
            continue;
        }
        assert_eq!(leaf.node_name, "consume");
        assert_eq!(
            leaf.bindings,
            vec![
                (
                    "all".to_string(),
                    json!({"check_full_review_threads": scan.clone()})
                ),
                ("scan".to_string(), scan.clone()),
                ("flag".to_string(), json!(true)),
            ]
        );
        let leaf = next_leaf(finish_leaf(&mut execution, &leaf, None, &mut new_id).decision);
        assert_eq!(leaf.node_name, "classify");
        let completed = finish_leaf(&mut execution, &leaf, Some(scan), &mut new_id);
        if status == "HOLD" {
            assert_eq!(next_leaf(completed.decision).node_name, "finished");
            continue;
        }
        let ExecutionAdvanceDecision::StartNodes(leaves) = completed.decision else {
            panic!("fanout must expand");
        };
        assert_eq!(leaves.len(), 2);
        for (leaf, item) in leaves.iter().zip(["first", "second"]) {
            assert_eq!(leaf.node_name(), "worker");
            assert_eq!(
                expect_leaf(leaf).bindings,
                vec![("item".to_string(), json!(item))]
            );
            assert_eq!(expect_leaf(leaf).item, Some(json!(item)));
        }
    }
}

fn fanout_execution(children: &str, items: &str) -> WorkflowExecution {
    execution(&format!(
        r#"
name: fanout-map
description: test
nodes:
  main:
    fanout:
      children: {children}
      {items}
  a: {{command: 'echo a'}}
  b: {{command: 'echo b'}}
"#
    ))
}

fn start_fanout(
    execution: &mut WorkflowExecution,
    new_id: &mut dyn FnMut() -> String,
) -> Vec<LeafStart> {
    let ExecutionAdvanceDecision::StartNodes(leaves) =
        execution.start_root(new_id, 1.0).unwrap().decision
    else {
        panic!("expected fanout leaves");
    };
    leaves
        .into_iter()
        .map(|start| match start {
            NodeStart::Leaf(leaf) => leaf,
            _ => panic!("expected leaf start"),
        })
        .collect()
}

#[test]
fn test_fanoutの成果_itemsの有無と複数childrenでキーが決まり空なら空mapになる() {
    // Given
    for (children, items, expected) in [
        ("[a, b]", "", json!({"a": {"slot": 0}, "b": {"slot": 1}})),
        (
            "[a]",
            "items: [x, y]",
            json!({"0": {"slot": 0}, "1": {"slot": 1}}),
        ),
        (
            "[a, b]",
            "items: [x, y]",
            json!({"0": {"slot": 0}, "1": {"slot": 1}, "2": {"slot": 2}, "3": {"slot": 3}}),
        ),
        ("[a]", "items: []", json!({})),
    ] {
        let mut execution = fanout_execution(children, items);
        let mut new_id = id_source();
        let started = execution.start_root(&mut new_id, 1.0).unwrap();
        let mut events = started.events;

        // When
        if let ExecutionAdvanceDecision::StartNodes(leaves) = started.decision {
            for (index, leaf) in leaves.iter().enumerate().rev() {
                execution.record_pending_result(
                    leaf.node_execution_id(),
                    None,
                    Some(json!({"slot": index})),
                    None,
                    Some(TokenUsage {
                        input_tokens: 2,
                        output_tokens: 3,
                    }),
                    2.0,
                );
                events.extend(
                    execution
                        .complete_leaf_and_advance(leaf.node_execution_id(), &mut new_id, 3.0)
                        .unwrap()
                        .events,
                );
            }
        }

        // Then
        let slot_count = expected.as_object().unwrap().len() as u64;
        assert_eq!(*execution.state(), RuntimeExecutionState::Completed);
        assert_eq!(
            execution.node_executions()[0].artifact,
            Some(expected.clone())
        );
        assert!(events.iter().any(|event| matches!(event,
            WorkflowEvent::ArtifactProduced { node_name, contract: None, value, .. }
                if node_name == "main" && value == &expected
        )));
        assert!(events.iter().any(|event| matches!(event,
            WorkflowEvent::NodeCompleted { node_name, result_summary: Some(summary), token_usage: Some(usage), .. }
                if node_name == "main" && summary == "complete"
                    && usage == &TokenUsage { input_tokens: slot_count * 2, output_tokens: slot_count * 3 }
        )));
    }
}

#[test]
fn test_fanoutの成果_artifact未宣言の完了slotをnullで残す() {
    // Given
    let mut execution = execution(
        r#"
name: null-slots
description: test
nodes:
  main: {fanout: {children: [silent, failed]}}
  silent: {session: {provider: codex}}
  failed: {command: 'exit 1'}
"#,
    );
    let mut new_id = id_source();
    let leaves = start_fanout(&mut execution, &mut new_id);
    finish_leaf(&mut execution, &leaves[0], None, &mut new_id);
    finish_leaf(
        &mut execution,
        &leaves[1],
        Some(json!({"ok": false})),
        &mut new_id,
    );

    // Then
    assert_eq!(
        execution.node_executions()[0].artifact,
        Some(json!({"silent": null, "failed": {"ok": false}}))
    );
}

#[test]
fn test_fanoutの成果_replayのpush順が異なっても展開座標から同じキーを作る() {
    // Given
    let mut execution = fanout_execution("[a, b]", "items: [x, y]");
    execution
        .replay_node_started("main-id", "main", NodeKindName::Fanout, 1, None, 1.0)
        .unwrap();
    for (index, name, item_index, child_index) in [
        (3, "b", 1, 1),
        (0, "a", 0, 0),
        (2, "a", 1, 0),
        (1, "b", 0, 1),
    ] {
        execution
            .replay_node_started(
                &format!("slot-{index}"),
                name,
                NodeKindName::Command,
                1,
                Some(ExecutionParentRef::fanout_child(
                    "main-id",
                    Some(item_index),
                    child_index,
                )),
                2.0,
            )
            .unwrap();
    }
    let mut new_id = id_source();

    // When
    for index in [0, 1, 2, 3] {
        let leaf = execution.leaf_start_for(&format!("slot-{index}")).unwrap();
        finish_leaf(
            &mut execution,
            &leaf,
            Some(json!({"slot": index})),
            &mut new_id,
        );
    }

    // Then
    assert_eq!(
        execution.node_execution("main-id").unwrap().artifact,
        Some(json!({"0": {"slot": 0}, "1": {"slot": 1}, "2": {"slot": 2}, "3": {"slot": 3}}))
    );
}

#[test]
fn test_fanoutの成果_解決不能な展開座標は集約エラーになる() {
    // Given
    let mut execution = fanout_execution("[a]", "");
    let mut new_id = id_source();
    let leaves = start_fanout(&mut execution, &mut new_id);
    let parent = execution
        .runtime
        .node_executions
        .iter_mut()
        .find(|node| node.id == leaves[0].node_execution_id)
        .unwrap()
        .parent
        .as_mut()
        .unwrap();
    *parent = crate::domain::workflow::ExecutionParentRef::fanout_child(
        parent.parent_id.clone(),
        parent.fanout_slot().unwrap().item_index,
        1,
    );
    let scope_id = execution.node_executions()[0].id.clone();

    // When
    let result = execution.complete_scope(&scope_id, false, &mut AdvanceEffects::Derive, 2.0);

    // Then
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("has no valid slot coordinates"));
}

#[test]
fn test_fanoutの多段参照_名前と添字とsequence経由で入力束縛とitems展開へ値を渡す() {
    // Given
    let mut execution = execution(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/adaptor/gateway/workflow/fixtures/valid/fanout-map-references.yml"
    )));
    crate::domain::workflow::services::validation::validate(&execution.workflow).unwrap();
    let mut new_id = id_source();
    let leaf = next_leaf(execution.start_root(&mut new_id, 1.0).unwrap().decision);
    let named = json!({"passed": true, "tasks": ["first", "second"]});
    let indexed = json!({"passed": false, "tasks": []});
    let nested = json!({"passed": true, "tasks": ["third", "fourth"]});

    // When
    let completed = finish_leaf(&mut execution, &leaf, Some(named.clone()), &mut new_id);
    let ExecutionAdvanceDecision::StartNodes(leaves) = completed.decision else {
        panic!("expected items expansion");
    };

    // Then
    assert_eq!(leaves.len(), 2);
    for (leaf, item) in leaves.iter().zip(["first", "second"]) {
        assert_eq!(
            expect_leaf(leaf).bindings,
            vec![("item".to_string(), json!(item))]
        );
    }
    finish_leaf(
        &mut execution,
        expect_leaf(&leaves[0]),
        Some(indexed.clone()),
        &mut new_id,
    );
    let leaf = next_leaf(
        finish_leaf(
            &mut execution,
            expect_leaf(&leaves[1]),
            Some(indexed),
            &mut new_id,
        )
        .decision,
    );
    assert_eq!(leaf.node_name, "nested_a");
    let leaf = next_leaf(finish_leaf(&mut execution, &leaf, Some(nested), &mut new_id).decision);
    assert_eq!(leaf.node_name, "consume");
    assert_eq!(
        leaf.bindings,
        vec![
            ("all".to_string(), json!({"a": named.clone()})),
            ("slot".to_string(), named),
            ("named".to_string(), json!(true)),
            ("indexed".to_string(), json!(false)),
            ("nested".to_string(), json!(true)),
        ]
    );
    let completed = finish_leaf(&mut execution, &leaf, None, &mut new_id);
    let ExecutionAdvanceDecision::StartNodes(leaves) = completed.decision else {
        panic!("expected nested items expansion");
    };
    assert_eq!(leaves.len(), 2);
    for (leaf, item) in leaves.iter().zip(["third", "fourth"]) {
        assert_eq!(leaf.node_name(), "worker");
        assert_eq!(
            expect_leaf(leaf).bindings,
            vec![("item".to_string(), json!(item))]
        );
    }
}

#[test]
fn test_fanoutの辺_確定したmapのwhenとswitchとsequence経由で次のleafを起動する() {
    // Given
    for (passed, verdict, nested_passed, target) in [
        (false, "READY", true, "finished"),
        (true, "HOLD", true, "finished"),
        (true, "READY", false, "finished"),
        (true, "READY", true, "ready"),
    ] {
        let mut execution = execution(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/adaptor/gateway/workflow/fixtures/valid/fanout-map-routing.yml"
        )));
        crate::domain::workflow::services::validation::validate(&execution.workflow).unwrap();
        let mut new_id = id_source();
        let mut leaf = next_leaf(execution.start_root(&mut new_id, 1.0).unwrap().decision);

        // When
        assert_eq!(leaf.node_name, "a");
        leaf = next_leaf(
            finish_leaf(
                &mut execution,
                &leaf,
                Some(json!({"passed": passed, "verdict": verdict})),
                &mut new_id,
            )
            .decision,
        );
        if passed {
            assert_eq!(leaf.node_name, "indexed_worker");
            leaf = next_leaf(
                finish_leaf(
                    &mut execution,
                    &leaf,
                    Some(json!({"passed": true, "verdict": "READY"})),
                    &mut new_id,
                )
                .decision,
            );
            assert_eq!(leaf.node_name, "classify");
            leaf = next_leaf(
                finish_leaf(
                    &mut execution,
                    &leaf,
                    Some(json!({"passed": true, "verdict": verdict})),
                    &mut new_id,
                )
                .decision,
            );
            if verdict == "READY" {
                assert_eq!(leaf.node_name, "nested_a");
                leaf = next_leaf(
                    finish_leaf(
                        &mut execution,
                        &leaf,
                        Some(json!({"passed": nested_passed, "verdict": verdict})),
                        &mut new_id,
                    )
                    .decision,
                );
            }
        }

        // Then
        assert_eq!(leaf.node_name, target);
    }
}

#[test]
fn test_fanout集約node_commandとsessionが同じslot集合のmapを型なしinputで受ける() {
    // Given
    for source in [
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/adaptor/gateway/workflow/fixtures/valid/fanout-command-reducer.yml"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/adaptor/gateway/workflow/fixtures/valid/fanout-session-reducer.yml"
        )),
    ] {
        for all_lgtm in [true, false] {
            let mut execution = execution(source);
            crate::domain::workflow::services::validation::validate(&execution.workflow).unwrap();
            let mut new_id = id_source();
            let leaves = start_fanout(&mut execution, &mut new_id);
            let review_a = json!({"lgtm": true});
            let review_b = json!({"lgtm": all_lgtm});

            // When
            finish_leaf(
                &mut execution,
                &leaves[0],
                Some(review_a.clone()),
                &mut new_id,
            );
            let judge = next_leaf(
                finish_leaf(
                    &mut execution,
                    &leaves[1],
                    Some(review_b.clone()),
                    &mut new_id,
                )
                .decision,
            );

            // Then
            assert_eq!(judge.node_name, "judge");
            assert_eq!(
                judge.bindings,
                vec![(
                    "reviews".to_string(),
                    json!({"review-a": review_a, "review-b": review_b})
                )]
            );
            let judgment = if judge.kind == LeafKind::Command {
                json!({"ok": true, "all_lgtm": all_lgtm})
            } else {
                json!({"verdict": if all_lgtm {"READY"} else {"NEEDS_FIX"}})
            };
            let target = next_leaf(
                finish_leaf(&mut execution, &judge, Some(judgment), &mut new_id).decision,
            );
            assert_eq!(target.node_name, if all_lgtm { "ready" } else { "fix" });
        }
    }
}

#[test]
fn test_承認対象検証_承認要求未宣言ならrequire形式で不足を示す() {
    // Given
    let mut execution = execution(
        r#"
name: missing-approval-requirement
description: test
nodes:
  main: {session: {provider: codex}}
"#,
    );
    let mut new_id = id_source();
    let leaf = next_leaf(execution.start_root(&mut new_id, 1.0).unwrap().decision);
    assert_eq!(
        execution.mark_node_waiting_approval(&leaf.node_execution_id, 2.0),
        TransitionOutcome::Applied
    );

    // When
    let error = execution
        .resolve_approval_attempt_target("main", Some(&leaf.node_execution_id))
        .unwrap_err();

    // Then
    assert_eq!(
        error,
        crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(
            "node does not declare completion.require: approval".to_string()
        )
    );
}

#[test]
fn test_completion要求_全node種別で本来の完了条件後に承認を待ち省略時は自動完了する() {
    // Given
    for kind in [
        "session: {provider: claude}",
        "command: 'true'",
        "fanout: {children: [{leaf: {command: 'true'}}]}",
        "sequence: {children: [{leaf: {command: 'true'}}]}",
    ] {
        for require_approval in [false, true] {
            let completion = if require_approval {
                "\n    completion: {require: approval}"
            } else {
                ""
            };
            let source = format!(
                "name: completion\ndescription: test\nnodes:\n  main:\n    {kind}{completion}\n"
            );
            let mut execution = execution(&source);
            let mut new_id = id_source();
            // When
            let leaf = next_leaf(execution.start_root(&mut new_id, 1.0).unwrap().decision);
            let completed_events = if leaf.kind == LeafKind::Session {
                execution.record_node_completion_signal(
                    &leaf.node_execution_id,
                    NodeCompletionSignal::Submit,
                    2.0,
                );
                assert_eq!(
                    execution.decide_node_completion_handshake(&leaf.node_execution_id),
                    NodeCompletionHandshakeDecision::AwaitingSignal
                );
                assert_eq!(
                    execution.node_executions()[0].status,
                    RuntimeNodeExecutionStatus::Running
                );
                execution.record_node_completion_signal(
                    &leaf.node_execution_id,
                    NodeCompletionSignal::Stop,
                    3.0,
                );
                let expected = if require_approval {
                    NodeCompletionHandshakeDecision::RequestApproval
                } else {
                    NodeCompletionHandshakeDecision::CompleteAuto
                };
                assert_eq!(
                    execution.decide_node_completion_handshake(&leaf.node_execution_id),
                    expected
                );
                let applied = execution
                    .apply_node_completion_handshake(&leaf.node_execution_id, &mut new_id, 3.0)
                    .unwrap();
                assert_eq!(applied.advance.is_none(), require_approval);
                applied.events
            } else if leaf.node_name == "main" {
                let disposition = workflow_transition::decide_completion_disposition(
                    execution.workflow.node_by_name("main").unwrap(),
                );
                if require_approval {
                    assert_eq!(
                        disposition,
                        workflow_transition::CompletionDisposition::RequestApproval
                    );
                    assert_eq!(
                        execution.record_pending_result(
                            &leaf.node_execution_id,
                            Some("process exited".into()),
                            Some(json!({"ok": true})),
                            None,
                            None,
                            3.0
                        ),
                        TransitionOutcome::Applied
                    );
                    assert_eq!(
                        execution.mark_node_waiting_approval(&leaf.node_execution_id, 3.0),
                        TransitionOutcome::Applied
                    );
                    Vec::new()
                } else {
                    assert_eq!(
                        disposition,
                        workflow_transition::CompletionDisposition::Complete
                    );
                    finish_leaf(&mut execution, &leaf, None, &mut new_id).events
                }
            } else {
                finish_leaf(&mut execution, &leaf, None, &mut new_id).events
            };
            let root = execution
                .node_executions()
                .iter()
                .find(|node| node.node_name == "main")
                .unwrap();
            let root_id = root.id.clone();
            // Then
            if require_approval {
                assert_eq!(
                    root.status,
                    RuntimeNodeExecutionStatus::WaitingApproval,
                    "{kind}"
                );
                if root.kind != NodeKindName::Command {
                    assert!(completed_events.iter().any(|event| matches!(event,
                        WorkflowEvent::ApprovalRequested { node_execution_id, .. } if node_execution_id == &root_id
                    )));
                }
                assert_ne!(*execution.state(), RuntimeExecutionState::Completed);
                execution
                    .apply_approval(&root_id, &mut new_id, 4.0)
                    .unwrap();
            } else {
                assert!(!completed_events
                    .iter()
                    .any(|event| matches!(event, WorkflowEvent::ApprovalRequested { .. })));
            }
            assert_eq!(
                *execution.state(),
                RuntimeExecutionState::Completed,
                "{kind}"
            );
            assert!(execution
                .node_executions()
                .iter()
                .all(|node| node.status == RuntimeNodeExecutionStatus::Succeeded));
        }
    }
}

#[test]
fn test_正本サンプル_fanout内の隔離sessionがdelegateを発火して成果をmergeへ渡す() {
    // Given
    use super::tests::{execution_id_of, settle_session_leaf, started_names};
    let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../workflows/examples/full-cycle-development.yml");
    let source = std::fs::read_to_string(source_path).unwrap();
    let workflow: WorkflowDefinition = serde_saphyr::from_str(&source).unwrap();
    let mut execution = WorkflowExecution::restore_runtime(WorkflowExecutionRestore {
        id: "canonical-example-execution".to_string(),
        repository_root: Some("/repo".into()),
        worktree_path: "/repo".into(),
        workflow,
        ..WorkflowExecutionRestore::default()
    });

    execution
        .replay_node_started("main", "main", NodeKindName::Sequence, 1, None, 1.0)
        .unwrap();
    execution
        .replay_node_started(
            "implementation",
            "implementation",
            NodeKindName::Sequence,
            1,
            Some(ExecutionParentRef::sequence_child("main")),
            2.0,
        )
        .unwrap();
    execution
        .replay_node_started(
            "create-detailed-design",
            "create_detailed_design",
            NodeKindName::Session,
            1,
            Some(ExecutionParentRef::sequence_child("implementation")),
            3.0,
        )
        .unwrap();

    let tasks = serde_json::json!({
        "tasks": [
            {
                "task_id": "task-1",
                "requirements": [],
                "depends_on": [],
                "parallel": true,
                "files": [],
                "outputs": [],
                "verify": []
            },
            {
                "task_id": "task-2",
                "requirements": [],
                "depends_on": [],
                "parallel": true,
                "files": [],
                "outputs": [],
                "verify": []
            }
        ]
    });
    assert_eq!(
        execution.record_pending_result(
            "create-detailed-design",
            Some("created two tasks".to_string()),
            Some(tasks),
            Some("implement-tasks".to_string()),
            None,
            4.0,
        ),
        TransitionOutcome::Applied
    );

    let mut new_id = id_source();
    // When
    let applied = settle_session_leaf(&mut execution, "create-detailed-design", &mut new_id, 5.0);
    assert_eq!(
        started_names(&applied.events),
        ["implement_all", "implement_task", "implement_task",]
    );
    let Some(ExecutionAdvanceDecision::StartNodes(implement_leaves)) = applied.advance else {
        panic!("canonical example must start one implementation leaf per task");
    };
    assert_eq!(implement_leaves.len(), 2);
    assert_ne!(
        execution.execution_worktree_path(implement_leaves[0].node_execution_id()),
        execution.execution_worktree_path(implement_leaves[1].node_execution_id())
    );

    let mut verify_leaves = Vec::new();
    for (index, leaf) in implement_leaves.iter().enumerate() {
        let id = leaf.node_execution_id();
        execution.apply_submitted_output(
            "implement_task".into(),
            id,
            1,
            None,
            "implement-task-result".into(),
            serde_json::json!({"task_id": format!("task-{}", index + 1), "summary": "implemented"}),
            None,
            6.0 + index as f64,
        );
        let applied = settle_session_leaf(&mut execution, id, &mut new_id, 6.0 + index as f64);
        let Some(ExecutionAdvanceDecision::StartNodes(leaves)) = applied.advance else {
            panic!("implement_task must advance from implement_task to verify_task");
        };
        assert_eq!(
            leaves
                .iter()
                .map(|leaf| leaf.node_name())
                .collect::<Vec<_>>(),
            ["verify_task"]
        );
        verify_leaves.extend(leaves);
    }

    let mut final_started = Vec::new();
    for (index, leaf) in verify_leaves.iter().enumerate() {
        assert_eq!(
            execution.record_pending_result(
                leaf.node_execution_id(),
                Some("verified".to_string()),
                Some(serde_json::json!({
                    "task_id": format!("task-{}", index + 1),
                    "complete": true,
                    "reason": "ok"
                })),
                Some("implement-task-check-result".to_string()),
                None,
                8.0 + index as f64,
            ),
            TransitionOutcome::Applied
        );
        let applied = settle_session_leaf(
            &mut execution,
            leaf.node_execution_id(),
            &mut new_id,
            10.0 + index as f64,
        );
        final_started.extend(started_names(&applied.events));
    }

    // Then
    assert_eq!(final_started, ["merge_implementations"]);
    let worktrees = execution
        .node_executions
        .iter()
        .filter(|node| node.node_name == "implement_task")
        .map(|node| node.worktree.as_ref().unwrap())
        .collect::<Vec<_>>();
    let expected_results = serde_json::json!({
        "0": {"task_id": "task-1", "summary": "implemented", "child": {"task_id": "task-1", "complete": true, "reason": "ok"}, "worktree": {"branch": worktrees[0].branch, "path": worktrees[0].path}},
        "1": {"task_id": "task-2", "summary": "implemented", "child": {"task_id": "task-2", "complete": true, "reason": "ok"}, "worktree": {"branch": worktrees[1].branch, "path": worktrees[1].path}}
    });
    let merge_id = execution_id_of(&execution, "merge_implementations");
    let merge = execution.leaf_start_for(&merge_id).unwrap();
    assert_eq!(
        merge
            .bindings
            .iter()
            .find(|(name, _)| name == "results")
            .map(|(_, value)| value),
        Some(&expected_results)
    );
    assert_eq!(
        execution
            .node_executions()
            .iter()
            .filter(|node| node.node_name == "implement_task")
            .map(|node| node.status)
            .collect::<Vec<_>>(),
        [
            RuntimeNodeExecutionStatus::Succeeded,
            RuntimeNodeExecutionStatus::Succeeded,
        ]
    );
    assert_eq!(
        execution
            .node_executions()
            .iter()
            .find(|node| node.node_name == "implement_all")
            .expect("canonical fanout must have started")
            .status,
        RuntimeNodeExecutionStatus::Succeeded
    );
}
