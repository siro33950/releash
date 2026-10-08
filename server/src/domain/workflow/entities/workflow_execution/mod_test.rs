use super::test_helpers::*;
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
        let restore = ExecutionTreeRestore {
            state: state.clone(),
            ..Default::default()
        };

        // When
        let execution = ExecutionTree::restore_runtime(restore);

        // Then
        assert_eq!(execution.state(), &state);
    }
}

#[test]

fn test_単独session_node完了事実はworkflowの木で拒否される() {
    // Given
    let mut tree = execution(
        "name: session\ndescription: test\nnodes:\n  main: {session: {provider: codex}}\n",
    );
    tree.begin_node_attempt(
        "main".into(),
        NodeKindName::Session,
        1,
        None,
        "root".into(),
        1.0,
    )
    .unwrap();
    let root_before = tree.node_execution("root").unwrap().clone();

    // When
    assert_eq!(
        tree.complete_standalone_session_node("root", 2.0),
        TransitionOutcome::NotApplicable
    );
    // Then
    assert_eq!(tree.node_execution("root").unwrap(), &root_before);
}

#[test]

fn test_単独session_node完了事実は子nodeで拒否される() {
    // Given
    let mut tree = execution(
        "name: session\ndescription: test\nnodes:\n  main: {session: {provider: codex}}\n  child: {session: {provider: codex}}\n",
    );
    tree.begin_node_attempt(
        "main".into(),
        NodeKindName::Session,
        1,
        None,
        "root".into(),
        1.0,
    )
    .unwrap();
    tree.begin_node_attempt(
        "child".into(),
        NodeKindName::Session,
        1,
        Some(ExecutionParentRef::delegate_child("root")),
        "child".into(),
        1.0,
    )
    .unwrap();
    tree.runtime.launched_as = ExecutionTreeLaunch::Session;
    let child_before = tree.node_execution("child").unwrap().clone();
    // When
    assert_eq!(
        tree.complete_standalone_session_node("child", 2.0),
        TransitionOutcome::NotApplicable
    );
    // Then
    assert_eq!(tree.node_execution("child").unwrap(), &child_before);
}

#[test]

fn test_単独session_node完了事実は存在しないnodeで拒否される() {
    // Given
    let mut tree = execution(
        "name: session\ndescription: test\nnodes:\n  main: {session: {provider: codex}}\n",
    );
    tree.begin_node_attempt(
        "main".into(),
        NodeKindName::Session,
        1,
        None,
        "root".into(),
        1.0,
    )
    .unwrap();
    tree.runtime.launched_as = ExecutionTreeLaunch::Session;
    let nodes_before = tree.node_executions().to_vec();
    // When
    assert_eq!(
        tree.complete_standalone_session_node("missing", 2.0),
        TransitionOutcome::NotApplicable
    );
    // Then
    assert_eq!(tree.node_executions(), nodes_before);
}

#[test]

fn test_単独session_node完了事実はrootを一度だけ完了する() {
    // Given
    let mut tree = execution(
        "name: session\ndescription: test\nnodes:\n  main: {session: {provider: codex}}\n",
    );
    tree.begin_node_attempt(
        "main".into(),
        NodeKindName::Session,
        1,
        None,
        "root".into(),
        1.0,
    )
    .unwrap();
    tree.runtime.launched_as = ExecutionTreeLaunch::Session;
    // When
    let outcome = tree.complete_standalone_session_node("root", 2.0);
    // Then
    assert_eq!(outcome, TransitionOutcome::Applied);
    assert_eq!(
        tree.node_execution("root").unwrap().status,
        RuntimeNodeExecutionStatus::Succeeded
    );
    assert_eq!(tree.node_execution("root").unwrap().completed_at, Some(2.0));

    // When
    let repeated = tree.complete_standalone_session_node("root", 3.0);
    // Then
    assert_eq!(repeated, TransitionOutcome::AlreadyApplied);
    assert_eq!(
        tree.node_execution("root").unwrap().status,
        RuntimeNodeExecutionStatus::Succeeded
    );
    assert_eq!(tree.node_execution("root").unwrap().completed_at, Some(2.0));
}

#[test]

fn test_provider停止_完了済みsessionではstopを受理しnodeの完了シグナルは適用しない() {
    // Given
    let mut tree = execution(
        "name: session\ndescription: test\nnodes:\n  main: {session: {provider: codex}}\n",
    );
    tree.begin_node_attempt(
        "main".into(),
        NodeKindName::Session,
        1,
        None,
        "root".into(),
        1.0,
    )
    .unwrap();
    tree.runtime.launched_as = ExecutionTreeLaunch::Session;
    tree.attach_node_session("root", "agent".into(), 2.0);
    assert_eq!(
        tree.complete_standalone_session_node("root", 3.0),
        TransitionOutcome::Applied
    );
    let node_before = tree.node_execution("root").unwrap().clone();
    // When
    let outcome = tree.record_provider_stop("root", "agent", 4.0);
    // Then
    assert_eq!(
        outcome,
        Ok(ProviderStopAccepted {
            node_signal: TransitionOutcome::NotApplicable
        })
    );
    assert_eq!(tree.node_execution("root").unwrap(), &node_before);
}

#[test]

fn test_provider停止_中断済みsessionではstopを受理しnodeの完了シグナルは適用しない() {
    // Given
    let mut tree = execution(
        "name: session\ndescription: test\nnodes:\n  main: {session: {provider: codex}}\n",
    );
    tree.begin_node_attempt(
        "main".into(),
        NodeKindName::Session,
        1,
        None,
        "root".into(),
        1.0,
    )
    .unwrap();
    tree.attach_node_session("root", "agent".into(), 2.0);
    assert_eq!(
        tree.abort_node_execution("root", 3.0),
        TransitionOutcome::Applied
    );
    let node_before = tree.node_execution("root").unwrap().clone();

    // When
    let outcome = tree.record_provider_stop("root", "agent", 4.0);

    // Then
    assert_eq!(
        outcome,
        Ok(ProviderStopAccepted {
            node_signal: TransitionOutcome::NotApplicable
        })
    );
    assert_eq!(tree.node_execution("root").unwrap(), &node_before);
}

fn finish_leaf(
    execution: &mut ExecutionTree,
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

fn test_node起動_起動済みのleafと終了したleafは再起動しない() {
    for kind in ["session: {provider: codex}", "command: true"] {
        // Given
        let mut execution = execution(&format!(
            "name: launch\ndescription: test\nnodes:\n  main: {{{kind}}}"
        ));
        let leaf = next_leaf(
            execution
                .start_root(&mut id_source(), 1.0)
                .unwrap()
                .decision,
        );
        assert!(execution
            .node_execution(&leaf.node_execution_id)
            .unwrap()
            .can_start_process());
        assert!(execution.can_prepare_node_worktree(&leaf.node_execution_id));

        // When
        match leaf.kind {
            LeafKind::Session => {
                execution.attach_node_session(&leaf.node_execution_id, "agent".into(), 2.0);
            }
            LeafKind::Command => {
                execution.record_node_display_command(&leaf.node_execution_id, "true".into(), 2.0);
            }
        }

        // Then
        assert!(!execution
            .node_execution(&leaf.node_execution_id)
            .unwrap()
            .can_start_process());
        assert!(!execution.can_prepare_node_worktree(&leaf.node_execution_id));
        let mut node = execution
            .node_execution(&leaf.node_execution_id)
            .unwrap()
            .clone();
        node.session_id = None;
        node.display_command = None;
        for status in [
            RuntimeNodeExecutionStatus::WaitingApproval,
            RuntimeNodeExecutionStatus::Succeeded,
            RuntimeNodeExecutionStatus::Aborted,
        ] {
            node.status = status;
            assert!(!node.can_start_process());
        }
        node.status = RuntimeNodeExecutionStatus::Running;
        for kind in [NodeKindName::Sequence, NodeKindName::Fanout] {
            node.kind = kind;
            assert!(!node.can_start_process());
        }
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
        crate::domain::workflow::services::validation::validate(
            execution.workflow_definition().unwrap(),
        )
        .unwrap();
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

fn fanout_execution(children: &str, items: &str) -> ExecutionTree {
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
    execution: &mut ExecutionTree,
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
    crate::domain::workflow::services::validation::validate(
        execution.workflow_definition().unwrap(),
    )
    .unwrap();
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
        crate::domain::workflow::services::validation::validate(
            execution.workflow_definition().unwrap(),
        )
        .unwrap();
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
            crate::domain::workflow::services::validation::validate(
                execution.workflow_definition().unwrap(),
            )
            .unwrap();
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
                    execution
                        .workflow
                        .as_ref()
                        .unwrap()
                        .node_by_name("main")
                        .unwrap(),
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

fn test_実行木archive遷移_未終了を拒否し終了状態を変えずarchiveとrestoreを冪等に受理する() {
    use crate::domain::workflow::{ArchiveRequestedFact, NodeFact};
    // Given
    let mut execution = ExecutionTree::restore(RuntimeExecutionState::Running);
    // When / Then
    assert!(execution.archive(1.0, "manual").is_err());
    assert!(execution.restore_archive().is_none());
    execution.replay_aborted_at(2.0, None);
    let before = execution.state().clone();
    assert_eq!(
        execution.archive(3.125, "worktree_removed").unwrap(),
        Some(NodeFact::ArchiveRequested(ArchiveRequestedFact {
            archived_at: 3.125,
            reason: "worktree_removed".into()
        }))
    );
    assert!(execution.archive(4.0, "manual").unwrap().is_none());
    assert_eq!(
        execution.restore_archive(),
        Some(NodeFact::RestoreRequested)
    );
    assert!(execution.restore_archive().is_none());
    assert_eq!(execution.state(), &before);
    assert!(execution.archive(5.0, "manual").unwrap().is_some());
}

#[test]

fn test_command反映判定_実行木とnodeとattemptが一致するrunningだけを受理する() {
    // Given
    let mut tree = execution("name: wf\ndescription: test\nnodes:\n  main:\n    command: true\n");
    let leaf = next_leaf(tree.start_root(&mut id_source(), 1.0).unwrap().decision);
    let id = &leaf.node_execution_id;
    // When / Then
    assert!(tree.validate_command_attempt(id, "main", 1).is_ok());
    assert!(tree.validate_command_attempt("missing", "main", 1).is_err());
    assert!(tree.validate_command_attempt(id, "other", 1).is_err());
    assert!(tree.validate_command_attempt(id, "main", 2).is_err());
    tree.mark_node_waiting_approval(id, 2.0);
    assert_eq!(
        tree.validate_command_attempt(id, "main", 1),
        Err("command NodeExecution is no longer running")
    );
    tree.transition_aborted();
    assert_eq!(
        tree.validate_command_attempt(id, "main", 1),
        Err("execution tree is terminal")
    );
}

fn session_tree() -> (ExecutionTree, LeafStart) {
    let mut tree = execution(
        "name: wf\ndescription: test\nnodes:\n  main:\n    session:\n      provider: claude\n",
    );
    let leaf = next_leaf(tree.start_root(&mut id_source(), 1.0).unwrap().decision);
    assert_eq!(
        tree.attach_node_session(&leaf.node_execution_id, "session-1".to_string(), 1.5),
        TransitionOutcome::Applied
    );
    (tree, leaf)
}

#[test]

fn test_session再開_動いているnodeは会話が残れば再開し無ければ新しいattemptで起動し直す() {
    use crate::domain::workflow::NodeProcessPresence as P;
    // Given
    let (tree, leaf) = session_tree();
    let id = &leaf.node_execution_id;
    // When / Then
    assert_eq!(
        tree.session_resume_action(id, P::ConfirmedAbsent, true, true),
        Ok(SessionResumeAction::ResumeConversation {
            session_id: "session-1".to_string(),
            injection: None,
        })
    );
    for (conversation, worktree) in [(false, true), (true, false), (false, false)] {
        assert_eq!(
            tree.session_resume_action(id, P::ConfirmedAbsent, conversation, worktree),
            Ok(SessionResumeAction::RestartAttempt)
        );
    }
}

#[test]

fn test_session再開_終わったnodeも会話が残れば再開し無ければ起動し直せず拒否する() {
    use crate::domain::workflow::NodeProcessPresence as P;
    // Given
    let (mut tree, leaf) = session_tree();
    let id = leaf.node_execution_id.clone();
    finish_leaf(&mut tree, &leaf, None, &mut id_source());
    assert_eq!(
        tree.node_execution(&id).unwrap().status,
        RuntimeNodeExecutionStatus::Succeeded
    );
    let (mut aborted, aborted_leaf) = session_tree();
    aborted.transition_aborted();
    for (tree, id) in [
        (&tree, id.as_str()),
        (&aborted, &aborted_leaf.node_execution_id),
    ] {
        // When / Then
        assert_eq!(
            tree.session_resume_action(id, P::ConfirmedAbsent, true, true),
            Ok(SessionResumeAction::ResumeConversation {
                session_id: "session-1".to_string(),
                injection: None,
            })
        );
        assert_eq!(
            tree.session_resume_action(id, P::ConfirmedAbsent, false, true),
            Err(SessionResumeRejection::Unrecoverable)
        );
    }
}

#[test]

fn test_session再開_プロセスが居るか不明なnodeとsession以外と存在しないnodeは拒否する() {
    use crate::domain::workflow::NodeProcessPresence as P;
    // Given
    let (tree, leaf) = session_tree();
    let mut command =
        execution("name: wf\ndescription: test\nnodes:\n  main:\n    command: 'true'\n");
    let command_leaf = next_leaf(command.start_root(&mut id_source(), 1.0).unwrap().decision);
    // When / Then
    for presence in [P::Live, P::Unknown] {
        assert_eq!(
            tree.session_resume_action(&leaf.node_execution_id, presence, true, true),
            Err(SessionResumeRejection::ProcessPresent)
        );
    }
    assert_eq!(
        command.session_resume_action(
            &command_leaf.node_execution_id,
            P::ConfirmedAbsent,
            true,
            true
        ),
        Err(SessionResumeRejection::NotSession)
    );
    assert_eq!(
        tree.session_resume_action("missing", P::ConfirmedAbsent, true, true),
        Err(SessionResumeRejection::NodeExecutionNotFound)
    );
}
pub(crate) mod tests {
    use super::*;

    fn aggregate(state: RuntimeExecutionState) -> ExecutionTree {
        ExecutionTree::restore(state)
    }

    fn states() -> [(ExecutionStateSet, RuntimeExecutionState); 3] {
        [
            (ExecutionStateSet::Active, RuntimeExecutionState::Running),
            (ExecutionStateSet::Finished, RuntimeExecutionState::Aborted),
            (
                ExecutionStateSet::Finished,
                RuntimeExecutionState::Completed,
            ),
        ]
    }

    #[test]

    fn test_fanout_items実行時解決_多段の終端配列で展開する() {
        // Given
        let parent_scope = ScopeRuntime {
            node_execution_id: "main-execution".to_string(),
            node_name: "main".to_string(),
            parent_scope_id: None,
            parameters: Vec::new(),
            kind: ScopeRuntimeKind::Sequence(SequenceScopeRuntime {
                artifacts: HashMap::from([(
                    "producer".to_string(),
                    RuntimeArtifact {
                        node_name: "producer".to_string(),
                        attempt: 1,
                        session_id: None,
                        result: None,
                        artifact: Some(serde_json::json!({
                            "payload": {"groups": {"items": [1, 2, 3]}}
                        })),
                        contract: Some("result".to_string()),
                        token_usage: None,
                        completed_at: 1.0,
                    },
                )]),
                ..Default::default()
            }),
        };
        let fanout_scope = ScopeRuntime {
            node_execution_id: "fan-execution".to_string(),
            node_name: "fan".to_string(),
            parent_scope_id: Some("main-execution".to_string()),
            parameters: Vec::new(),
            kind: ScopeRuntimeKind::Fanout(FanoutScopeRuntime::default()),
        };
        let execution = ExecutionTree::restore_runtime(ExecutionTreeRestore {
            scopes: vec![parent_scope, fanout_scope],
            ..Default::default()
        });
        let spec = crate::domain::workflow::value_objects::FanoutSpec {
            children: Vec::new(),
            items: Some(crate::domain::workflow::ItemsSource::ArtifactField {
                node: "producer".to_string(),
                field_path: crate::domain::workflow::FieldPath::new(["payload", "groups", "items"]),
            }),
        };

        // When
        let items = execution
            .resolve_fanout_items_in_scope(execution.scope("fan-execution").unwrap(), &spec)
            .unwrap();

        // Then
        assert_eq!(
            items,
            Some(vec![
                serde_json::json!(1),
                serde_json::json!(2),
                serde_json::json!(3)
            ])
        );
    }

    #[test]

    fn state_sets_are_exhaustive() {
        for (expected, state) in states() {
            assert_eq!(aggregate(state).state_set(), expected);
        }
        assert_eq!(
            aggregate(RuntimeExecutionState::Aborted).state_set(),
            ExecutionStateSet::Finished
        );
    }

    #[test]

    fn test_workflow状態遷移_実行中だけ完了とabortへ遷移する() {
        // Given / When / Then
        for (state, complete, abort) in [
            (
                RuntimeExecutionState::Running,
                TransitionOutcome::Applied,
                TransitionOutcome::Applied,
            ),
            (
                RuntimeExecutionState::Completed,
                TransitionOutcome::AlreadyApplied,
                TransitionOutcome::NotApplicable,
            ),
            (
                RuntimeExecutionState::Aborted,
                TransitionOutcome::NotApplicable,
                TransitionOutcome::NotApplicable,
            ),
        ] {
            let mut completed = aggregate(state.clone());
            assert_eq!(completed.complete(), complete);
            let expected = if state == RuntimeExecutionState::Aborted {
                &state
            } else {
                &RuntimeExecutionState::Completed
            };
            assert_eq!(completed.state(), expected);
            let mut aborted = aggregate(state.clone());
            assert_eq!(aborted.abort(), abort);
            let expected = if state == RuntimeExecutionState::Completed {
                &state
            } else {
                &RuntimeExecutionState::Aborted
            };
            assert_eq!(aborted.state(), expected);
        }
    }

    fn restored_execution(state: RuntimeExecutionState) -> ExecutionTree {
        ExecutionTree::restore_runtime(ExecutionTreeRestore {
            id: "execution-1".to_string(),
            workflow: WorkflowDefinition {
                name: "workflow".to_string(),
                nodes: vec![crate::domain::workflow::NodeDefinition {
                    name: "implement".to_string(),
                    ..Default::default()
                }],
                entry: "implement".to_string(),
                ..Default::default()
            },
            state,
            ..ExecutionTreeRestore::default()
        })
    }

    #[test]

    fn newly_terminal_sessions_activeから終端への初回遷移だけを導出する() {
        for active in [
            RuntimeNodeExecutionStatus::Running,
            RuntimeNodeExecutionStatus::WaitingApproval,
        ] {
            for terminal in [
                RuntimeNodeExecutionStatus::Succeeded,
                RuntimeNodeExecutionStatus::Aborted,
            ] {
                let mut before = restored_execution(RuntimeExecutionState::Running);
                before
                    .begin_node_attempt(
                        "implement".to_string(),
                        NodeKindName::Session,
                        1,
                        None,
                        "node-execution-1".to_string(),
                        10.0,
                    )
                    .unwrap();
                before.attach_node_session("node-execution-1", "agent-session-1".to_string(), 11.0);
                before.node_executions[0].status = active;
                let mut after = before.clone();
                after.node_executions[0].status = terminal;

                assert_eq!(
                    after.newly_terminal_sessions_since(&before),
                    vec![NewlyTerminalSession {
                        node_execution_id: "node-execution-1".to_string(),
                        agent_session_id: "agent-session-1".to_string(),
                    }],
                    "{active:?} -> {terminal:?}"
                );
            }
        }
    }

    #[test]

    fn newly_terminal_sessions_active維持と既終端と非sessionと参照なしを除外する() {
        let mut before = restored_execution(RuntimeExecutionState::Running);
        for (id, kind, session_id, status) in [
            (
                "running",
                NodeKindName::Session,
                Some("running-session"),
                RuntimeNodeExecutionStatus::Running,
            ),
            (
                "waiting-approval",
                NodeKindName::Session,
                Some("waiting-approval-session"),
                RuntimeNodeExecutionStatus::WaitingApproval,
            ),
            (
                "terminal",
                NodeKindName::Session,
                Some("terminal-session"),
                RuntimeNodeExecutionStatus::Succeeded,
            ),
            (
                "command",
                NodeKindName::Command,
                Some("command-session"),
                RuntimeNodeExecutionStatus::Running,
            ),
            (
                "unattached",
                NodeKindName::Session,
                None,
                RuntimeNodeExecutionStatus::Running,
            ),
        ] {
            before
                .begin_node_attempt("implement".to_string(), kind, 1, None, id.to_string(), 10.0)
                .unwrap();
            if let Some(session_id) = session_id {
                before.attach_node_session(id, session_id.to_string(), 11.0);
            }
            before
                .node_executions
                .iter_mut()
                .find(|node| node.id == id)
                .unwrap()
                .status = status;
        }
        let mut after = before.clone();
        for id in ["terminal", "command", "unattached"] {
            after
                .node_executions
                .iter_mut()
                .find(|node| node.id == id)
                .unwrap()
                .status = RuntimeNodeExecutionStatus::Aborted;
        }

        assert!(after.newly_terminal_sessions_since(&before).is_empty());
        after.id = "different-execution".to_string();
        assert!(after.newly_terminal_sessions_since(&before).is_empty());
    }

    #[test]

    fn node_submit_target_is_derived_from_node_execution_identity() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "node-execution-1".to_string(),
                10.0,
            )
            .unwrap();

        let target = execution.admit_node_submit("node-execution-1").unwrap();
        assert_eq!(target.node_name, "implement");
        assert_eq!(target.attempt, 1);
        assert_eq!(
            execution.admit_node_submit("missing"),
            Err(NodeSubmitRejection::NodeExecutionNotFound)
        );
    }

    #[test]

    fn aggregate_owns_node_attempt_session_and_terminal_fact() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        let node_execution_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "node-execution-1".to_string(),
                10.0,
            )
            .unwrap();

        assert_eq!(
            execution.decide_node_completion_handshake(&node_execution_id),
            NodeCompletionHandshakeDecision::AwaitingSignal
        );
        assert_eq!(
            execution.attach_node_session(&node_execution_id, "session-1".to_string(), 11.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.record_node_completion_signal(
                &node_execution_id,
                NodeCompletionSignal::Submit,
                11.5,
            ),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.record_node_completion_signal(
                &node_execution_id,
                NodeCompletionSignal::Stop,
                11.75,
            ),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.complete_node_execution(
                &node_execution_id,
                Some(serde_json::json!({"ok": true})),
                Some(TokenUsage {
                    input_tokens: 2,
                    output_tokens: 3,
                }),
                12.0,
            ),
            TransitionOutcome::Applied
        );

        let node = &execution.node_executions()[0];
        assert_eq!(node.status, RuntimeNodeExecutionStatus::Succeeded);
        assert_eq!(node.session_id.as_deref(), Some("session-1"));
        assert_eq!(
            node.artifact.as_ref(),
            Some(&serde_json::json!({"ok": true}))
        );
        assert_eq!(
            execution.complete_node_execution(&node_execution_id, None, None, 13.0),
            TransitionOutcome::AlreadyApplied
        );
    }

    #[test]

    fn provider_stop_admission_belongs_to_the_workflow_aggregate() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        let node_execution_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "node-execution-1".to_string(),
                10.0,
            )
            .unwrap();
        execution.attach_node_session(&node_execution_id, "session-1".to_string(), 11.0);

        assert_eq!(
            execution.record_provider_stop(&node_execution_id, "session-2", 12.0),
            Err(ProviderStopRejection::SessionDoesNotOwnAttempt)
        );
        assert_eq!(
            execution.record_provider_stop(&node_execution_id, "session-1", 13.0),
            Ok(ProviderStopAccepted {
                node_signal: TransitionOutcome::Applied
            })
        );
        assert_eq!(
            execution.record_provider_stop(&node_execution_id, "session-1", 14.0),
            Ok(ProviderStopAccepted {
                node_signal: TransitionOutcome::AlreadyApplied
            })
        );
    }

    #[test]

    fn routing_failure_surfaces_an_error_without_workflow_terminal_transition() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution.runtime.workflow.as_mut().unwrap().nodes.push(
            crate::domain::workflow::NodeDefinition {
                name: "main".to_string(),
                kind: crate::domain::workflow::NodeKind::Sequence(
                    crate::domain::workflow::SequenceSpec {
                        entry: None,
                        children: vec![crate::domain::workflow::ChildEntry {
                            name: "implement".to_string(),
                            inputs: Vec::new(),
                            rules: Some(vec![crate::domain::workflow::Rule::Next(
                                "missing-node".to_string(),
                            )]),
                        }],
                    },
                ),
                ..Default::default()
            },
        );
        execution.runtime.workflow.as_mut().unwrap().entry = "main".to_string();
        execution
            .replay_node_started("main-1", "main", NodeKindName::Sequence, 1, None, 9.0)
            .unwrap();
        execution
            .replay_node_started(
                "node-execution-1",
                "implement",
                NodeKindName::Session,
                1,
                Some(ExecutionParentRef::sequence_child("main-1")),
                10.0,
            )
            .unwrap();
        execution.record_node_completion_signal(
            "node-execution-1",
            NodeCompletionSignal::Submit,
            10.5,
        );
        execution.record_node_completion_signal(
            "node-execution-1",
            NodeCompletionSignal::Stop,
            10.75,
        );

        let mut new_id = || "next-node".to_string();
        let result =
            execution.apply_node_completion_handshake("node-execution-1", &mut new_id, 11.0);

        assert!(result.is_err());
        assert_ne!(execution.state(), &RuntimeExecutionState::Completed);
    }

    #[test]

    fn agent_node_attempt_cannot_complete_before_submit_and_stop() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        let node_execution_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "node-execution-1".to_string(),
                10.0,
            )
            .unwrap();

        assert_eq!(
            execution.complete_node_execution(&node_execution_id, None, None, 11.0),
            TransitionOutcome::NotApplicable
        );
        assert_eq!(
            execution.node_executions()[0].status,
            RuntimeNodeExecutionStatus::Running
        );
        assert_eq!(
            execution.record_node_completion_signal(
                &node_execution_id,
                NodeCompletionSignal::Stop,
                12.0,
            ),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.node_executions()[0].completion_signals,
            NodeCompletionSignalState::StopReceived
        );
        assert_eq!(
            execution.decide_node_completion_handshake(&node_execution_id),
            NodeCompletionHandshakeDecision::AwaitingSignal
        );
        assert_eq!(
            execution.complete_node_execution(&node_execution_id, None, None, 13.0),
            TransitionOutcome::NotApplicable
        );
        assert_eq!(
            execution.record_node_completion_signal(
                &node_execution_id,
                NodeCompletionSignal::Stop,
                14.0,
            ),
            TransitionOutcome::AlreadyApplied
        );
        assert_eq!(
            execution.record_node_completion_signal(
                &node_execution_id,
                NodeCompletionSignal::Submit,
                15.0,
            ),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.node_executions()[0].completion_signals,
            NodeCompletionSignalState::Ready
        );
        assert_eq!(
            execution.decide_node_completion_handshake(&node_execution_id),
            NodeCompletionHandshakeDecision::CompleteAuto
        );
        assert_eq!(
            execution.complete_node_execution(&node_execution_id, None, None, 16.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.decide_node_completion_handshake(&node_execution_id),
            NodeCompletionHandshakeDecision::AlreadySettled
        );
    }

    #[test]

    fn test_fanout親_completion承認はauto子の完了経路でも承認待ちになる() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution.runtime.workflow.as_mut().unwrap().nodes = vec![
            crate::domain::workflow::NodeDefinition {
                name: "fanout".to_string(),
                kind: crate::domain::workflow::NodeKind::Fanout(
                    crate::domain::workflow::FanoutSpec {
                        children: vec![crate::domain::workflow::ChildEntry::reference("worker")],
                        items: None,
                    },
                ),
                completion: crate::domain::workflow::NodeCompletion::require_approval(),
                ..Default::default()
            },
            crate::domain::workflow::NodeDefinition {
                name: "worker".to_string(),
                ..Default::default()
            },
        ];
        execution.runtime.workflow.as_mut().unwrap().entry = "fanout".to_string();
        execution
            .replay_node_started(
                "parent-execution-1",
                "fanout",
                NodeKindName::Fanout,
                1,
                None,
                10.0,
            )
            .unwrap();
        execution
            .replay_node_started(
                "child-execution-1",
                "worker",
                NodeKindName::Session,
                1,
                Some(ExecutionParentRef::fanout_child(
                    "parent-execution-1",
                    None,
                    0,
                )),
                10.0,
            )
            .unwrap();
        execution.record_node_completion_signal(
            "child-execution-1",
            NodeCompletionSignal::Submit,
            11.0,
        );
        execution.record_node_completion_signal(
            "child-execution-1",
            NodeCompletionSignal::Stop,
            12.0,
        );

        let mut new_id = || "node-execution-next".to_string();
        let result = execution
            .apply_node_completion_handshake("child-execution-1", &mut new_id, 13.0)
            .unwrap();

        assert_eq!(
            result.advance,
            Some(ExecutionAdvanceDecision::Persist),
            "承認まで次 node へ進まない"
        );
        assert!(
            result.events.iter().any(|event| matches!(
                event,
                WorkflowEvent::ApprovalRequested { node_execution_id, node_name, .. }
                    if node_execution_id == "parent-execution-1" && node_name == "fanout"
            )),
            "親の ApprovalRequested が発行される: {:?}",
            result.events
        );
        let parent = execution
            .node_executions()
            .iter()
            .find(|node| node.id == "parent-execution-1")
            .unwrap();
        assert_eq!(
            parent.status,
            RuntimeNodeExecutionStatus::WaitingApproval,
            "親は承認待ちで完了しない"
        );
        assert!(
            execution.scope("parent-execution-1").is_some(),
            "承認時の artifact 集約のため fanout スコープは保持される"
        );
    }

    #[test]

    fn completion_handshake_applies_the_domain_transition_and_uses_the_supplied_next_id() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution.runtime.workflow.as_mut().unwrap().nodes.push(
            crate::domain::workflow::NodeDefinition {
                name: "verify".to_string(),
                kind: crate::domain::workflow::NodeKind::Command(
                    crate::domain::workflow::CommandSpec {
                        command: "true".to_string(),
                        env: Default::default(),
                    },
                ),
                ..Default::default()
            },
        );
        execution.runtime.workflow.as_mut().unwrap().nodes.push(
            crate::domain::workflow::NodeDefinition {
                name: "main".to_string(),
                kind: crate::domain::workflow::NodeKind::Sequence(
                    crate::domain::workflow::SequenceSpec {
                        entry: None,
                        children: vec![
                            crate::domain::workflow::ChildEntry::reference("implement"),
                            crate::domain::workflow::ChildEntry::reference("verify"),
                        ],
                    },
                ),
                ..Default::default()
            },
        );
        execution.runtime.workflow.as_mut().unwrap().entry = "main".to_string();
        execution
            .replay_node_started("main-1", "main", NodeKindName::Sequence, 1, None, 9.0)
            .unwrap();
        execution
            .replay_node_started(
                "node-execution-1",
                "implement",
                NodeKindName::Session,
                1,
                Some(ExecutionParentRef::sequence_child("main-1")),
                10.0,
            )
            .unwrap();
        execution.record_node_completion_signal(
            "node-execution-1",
            NodeCompletionSignal::Submit,
            11.0,
        );
        execution.record_node_completion_signal(
            "node-execution-1",
            NodeCompletionSignal::Stop,
            12.0,
        );

        let mut new_id = || "node-execution-2".to_string();
        let result = execution
            .apply_node_completion_handshake("node-execution-1", &mut new_id, 13.0)
            .unwrap();

        assert_eq!(
            result.advance,
            Some(ExecutionAdvanceDecision::StartNodes(vec![NodeStart::Leaf(
                LeafStart {
                    node_execution_id: "node-execution-2".to_string(),
                    node_name: "verify".to_string(),
                    kind: LeafKind::Command,
                    bindings: Vec::new(),
                    item: None,
                }
            )]))
        );
        assert_eq!(
            execution.node_executions().last().unwrap().id,
            "node-execution-2"
        );
        assert!(result.events.iter().any(|event| matches!(
            event,
            WorkflowEvent::NodeStarted { node_execution_id, node_name, .. }
                if node_execution_id == "node-execution-2" && node_name == "verify"
        )));
    }

    #[test]

    fn approval_target_requires_an_exact_attempt_when_fanout_names_are_ambiguous() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution.runtime.workflow.as_mut().unwrap().nodes[0].completion =
            crate::domain::workflow::NodeCompletion::require_approval();
        for (id, child_index) in [("child-1", 0), ("child-2", 1)] {
            execution
                .begin_node_attempt(
                    "implement".to_string(),
                    NodeKindName::Session,
                    1,
                    Some(ExecutionParentRef::fanout_child(
                        "parent-execution-1",
                        None,
                        child_index,
                    )),
                    id.to_string(),
                    10.0,
                )
                .unwrap();
            assert_eq!(
                execution.mark_node_waiting_approval(id, 11.0),
                TransitionOutcome::Applied
            );
        }

        assert!(matches!(
            execution.resolve_approval_attempt_target("implement", None),
            Err(crate::domain::workflow::WorkflowError::InvalidState(_))
        ));
        let target = execution
            .resolve_approval_attempt_target("implement", Some("child-2"))
            .unwrap();
        assert_eq!(target.node_execution_id, "child-2");
        assert_eq!(target.parent.unwrap().fanout_slot().unwrap().child_index, 1);
    }

    #[test]

    fn new_attempt_isolates_previous_completion_signals_and_preserves_its_history() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        let previous_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "node-execution-1".to_string(),
                10.0,
            )
            .unwrap();
        assert_eq!(
            execution.attach_node_session(&previous_id, "session-1".to_string(), 11.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.record_node_completion_signal(
                &previous_id,
                NodeCompletionSignal::Submit,
                12.0,
            ),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.apply_submitted_output(
                "implement".to_string(),
                &previous_id,
                1,
                Some("session-1".to_string()),
                "result".to_string(),
                serde_json::json!({"attempt": 1}),
                Some("first".to_string()),
                13.0,
            ),
            TransitionOutcome::Applied
        );

        execution.restart_node_attempt_at(&previous_id, "node-execution-2".to_string(), 20.0);

        assert_eq!(execution.node_executions().len(), 2);
        let previous = &execution.node_executions()[0];
        assert_eq!(previous.id, previous_id);
        assert_eq!(previous.status, RuntimeNodeExecutionStatus::Aborted);
        assert_eq!(
            previous.completion_signals,
            NodeCompletionSignalState::SubmitReceived
        );
        assert_eq!(previous.session_id.as_deref(), Some("session-1"));
        assert_eq!(
            previous.artifact.as_ref(),
            Some(&serde_json::json!({"attempt": 1}))
        );

        let current = &execution.node_executions()[1];
        assert_eq!(current.id, "node-execution-2");
        assert_eq!(current.attempt, 2);
        assert_eq!(current.status, RuntimeNodeExecutionStatus::Running);
        assert_eq!(
            execution
                .runtime
                .retry_predecessors
                .get("node-execution-2")
                .map(String::as_str),
            Some(previous_id.as_str())
        );
        assert_eq!(
            current.completion_signals,
            NodeCompletionSignalState::Pending
        );
        assert!(current.session_id.is_none());
        assert!(current.artifact.is_none());

        assert_eq!(
            execution
                .record_node_completion_signal(&previous_id, NodeCompletionSignal::Stop, 21.0,),
            TransitionOutcome::NotApplicable
        );
        assert_eq!(
            execution.node_executions()[1].completion_signals,
            NodeCompletionSignalState::Pending
        );
    }

    #[test]

    fn test_workflow_execution_session起動木もresume用の新attemptを作れる() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution.launched_as = ExecutionTreeLaunch::Session;
        let node_execution_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "session-root".to_string(),
                10.0,
            )
            .unwrap();
        execution.record_node_completion_signal(
            &node_execution_id,
            NodeCompletionSignal::Stop,
            11.0,
        );

        let restarted = execution.restart_node_attempt_at(
            &node_execution_id,
            "retry-attempt".to_string(),
            12.0,
        );

        let restarted = restarted.unwrap();
        assert_eq!(restarted.attempt.attempt, 2);
        assert_eq!(
            execution.node_execution(&node_execution_id).unwrap().status,
            RuntimeNodeExecutionStatus::Aborted
        );
        assert_eq!(
            restarted.attempt.status,
            RuntimeNodeExecutionStatus::Running
        );
    }

    #[test]

    fn node_attempt_abort_and_approval_transitions_are_closed() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        let first_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "node-execution-1".to_string(),
                10.0,
            )
            .unwrap();
        assert_eq!(
            execution.mark_node_waiting_approval(&first_id, 11.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.mark_node_waiting_approval(&first_id, 11.5),
            TransitionOutcome::AlreadyApplied
        );
        assert_eq!(
            execution.mark_node_running(&first_id, 12.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.abort_node_execution(&first_id, 13.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.complete_node_execution(&first_id, None, None, 15.0),
            TransitionOutcome::NotApplicable
        );

        let second_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                2,
                None,
                "node-execution-2".to_string(),
                16.0,
            )
            .unwrap();
        assert_eq!(
            execution.abort_node_execution(&second_id, 17.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.abort_node_execution(&second_id, 18.0),
            TransitionOutcome::AlreadyApplied
        );
        assert_eq!(execution.node_executions.len(), 2);
    }

    #[test]

    fn fanout_child_completion_updates_slot_and_node_as_one_transition() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution.runtime.workflow.as_mut().unwrap().nodes = vec![
            crate::domain::workflow::NodeDefinition {
                name: "fanout".to_string(),
                kind: crate::domain::workflow::NodeKind::Fanout(
                    crate::domain::workflow::FanoutSpec {
                        children: vec![
                            crate::domain::workflow::ChildEntry::reference("implement"),
                            crate::domain::workflow::ChildEntry::reference("verify"),
                        ],
                        items: None,
                    },
                ),
                ..Default::default()
            },
            crate::domain::workflow::NodeDefinition {
                name: "implement".to_string(),
                ..Default::default()
            },
            crate::domain::workflow::NodeDefinition {
                name: "verify".to_string(),
                ..Default::default()
            },
        ];
        execution.runtime.workflow.as_mut().unwrap().entry = "fanout".to_string();
        execution
            .replay_node_started(
                "parent-execution-1",
                "fanout",
                NodeKindName::Fanout,
                1,
                None,
                10.0,
            )
            .unwrap();
        for (id, name, child_index) in [("child-1", "implement", 0), ("child-2", "verify", 1)] {
            execution
                .replay_node_started(
                    id,
                    name,
                    NodeKindName::Session,
                    1,
                    Some(ExecutionParentRef::fanout_child(
                        "parent-execution-1",
                        None,
                        child_index,
                    )),
                    10.5,
                )
                .unwrap();
        }
        execution.record_node_completion_signal("child-1", NodeCompletionSignal::Submit, 11.25);
        execution.record_node_completion_signal("child-1", NodeCompletionSignal::Stop, 11.5);
        execution.record_pending_result(
            "child-1",
            Some("done".to_string()),
            Some(serde_json::json!({"ok": true})),
            Some("result".to_string()),
            None,
            11.9,
        );

        let mut new_id = || "unused".to_string();
        let applied = execution
            .complete_leaf_and_advance("child-1", &mut new_id, 12.0)
            .unwrap();

        assert_eq!(applied.decision, ExecutionAdvanceDecision::Persist);
        let fanout = execution
            .scope("parent-execution-1")
            .unwrap()
            .fanout()
            .unwrap();
        assert_eq!(fanout.children[0].state, FanoutChildRuntimeState::Completed);
        assert_eq!(
            fanout.children[0].artifact,
            Some(serde_json::json!({"ok": true}))
        );
        assert_eq!(fanout.children[1].state, FanoutChildRuntimeState::Running);
        assert_eq!(
            execution
                .node_executions()
                .iter()
                .find(|node| node.id == "child-1")
                .unwrap()
                .status,
            RuntimeNodeExecutionStatus::Succeeded
        );
        assert_eq!(execution.state(), &RuntimeExecutionState::Running);
    }

    #[test]

    fn fanout_child_retry_replaces_only_the_current_logical_child_attempt() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution.runtime.workflow.as_mut().unwrap().nodes = vec![
            crate::domain::workflow::NodeDefinition {
                name: "fanout".to_string(),
                kind: crate::domain::workflow::NodeKind::Fanout(
                    crate::domain::workflow::FanoutSpec {
                        children: vec![crate::domain::workflow::ChildEntry::reference("implement")],
                        items: None,
                    },
                ),
                ..Default::default()
            },
            crate::domain::workflow::NodeDefinition {
                name: "implement".to_string(),
                ..Default::default()
            },
        ];
        execution.runtime.workflow.as_mut().unwrap().entry = "fanout".to_string();
        execution
            .replay_node_started(
                "parent-execution-1",
                "fanout",
                NodeKindName::Fanout,
                1,
                None,
                10.0,
            )
            .unwrap();
        execution
            .replay_node_started(
                "child-execution-1",
                "implement",
                NodeKindName::Session,
                1,
                Some(ExecutionParentRef::fanout_child(
                    "parent-execution-1",
                    Some(0),
                    0,
                )),
                10.5,
            )
            .unwrap();
        execution.record_node_completion_signal(
            "child-execution-1",
            NodeCompletionSignal::Stop,
            12.0,
        );

        let restarted = execution
            .restart_node_attempt_at("child-execution-1", "child-execution-2".to_string(), 13.0)
            .unwrap();

        assert!(restarted.fanout_child);
        assert_eq!(execution.node_executions().len(), 3);
        let old = execution
            .node_executions()
            .iter()
            .find(|node| node.id == "child-execution-1")
            .unwrap();
        assert_eq!(old.status, RuntimeNodeExecutionStatus::Aborted);
        assert_eq!(
            old.completion_signals,
            NodeCompletionSignalState::StopReceived
        );
        let current = execution
            .node_executions()
            .iter()
            .find(|node| node.id == "child-execution-2")
            .unwrap();
        assert_eq!(current.attempt, 2);
        assert_eq!(
            current.completion_signals,
            NodeCompletionSignalState::Pending
        );
        let fanout = execution
            .scope("parent-execution-1")
            .unwrap()
            .fanout()
            .unwrap();
        assert_eq!(fanout.children.len(), 1);
        assert_eq!(fanout.children[0].node_execution_id, "child-execution-2");
        assert_eq!(fanout.children[0].attempt, 2);
        assert_eq!(fanout.children[0].state, FanoutChildRuntimeState::Running);
    }

    // --- 実行木（#1463）: 合成子の再帰実行 -----------------------------------

    use crate::domain::workflow::{
        ChildEntry, CommandSpec, FanoutSpec, NodeCompletion, NodeKind, Rule, SequenceSpec,
    };

    fn tree_command_node(name: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Command(CommandSpec {
                command: format!("printf {name}"),
                env: Default::default(),
            }),
            ..Default::default()
        }
    }

    fn tree_sequence_node(name: &str, children: Vec<ChildEntry>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Sequence(SequenceSpec {
                entry: None,
                children,
            }),
            ..Default::default()
        }
    }

    fn tree_execution(nodes: Vec<NodeDefinition>) -> ExecutionTree {
        ExecutionTree::restore_runtime(ExecutionTreeRestore {
            id: "execution-1".to_string(),
            workflow: WorkflowDefinition {
                name: "tree".to_string(),
                entry: "main".to_string(),
                nodes,
                ..Default::default()
            },
            ..ExecutionTreeRestore::default()
        })
    }

    fn tree_id_source() -> impl FnMut() -> String {
        let mut counter = 0;
        move || {
            counter += 1;
            format!("id-{counter}")
        }
    }

    #[test]

    fn test_command_env_resumeは保存済み宣言と再構築bindingから値を再解決できる() {
        let mut command = tree_command_node("run");
        command.input.push(crate::domain::workflow::InputParam {
            name: "document".to_string(),
            contract: None,
        });
        let NodeKind::Command(command_spec) = &mut command.kind else {
            unreachable!();
        };
        command_spec.env = [(
            crate::domain::workflow::EnvironmentVariableName::new("DOC").unwrap(),
            crate::domain::workflow::InputParameterRef::new("document").unwrap(),
        )]
        .into_iter()
        .collect();
        let mut entry = ChildEntry::reference("run");
        entry.inputs.push((
            "document".to_string(),
            crate::domain::workflow::value_objects::InputSourceRef::new("request"),
        ));
        let mut execution = tree_execution(vec![tree_sequence_node("main", vec![entry]), command]);
        execution.request = Some("document body".to_string());
        let mut new_id = tree_id_source();
        let started = execution.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartNodes(leaves) = started.decision else {
            panic!("command leaf must start");
        };
        let original = leaves[0].node_execution_id();

        let restarted = execution
            .restart_node_attempt_at(original, "resumed-command".to_string(), 3.0)
            .unwrap();

        assert_eq!(restarted.leaf.bindings, expect_leaf(&leaves[0]).bindings);
        let command = execution
            .node_definition("run")
            .and_then(NodeDefinition::command_spec)
            .unwrap();
        assert_eq!(
            crate::domain::workflow::services::reference::resolve_command_environment(
                &command.env,
                &restarted.leaf.bindings,
            )
            .unwrap(),
            vec![("DOC".to_string(), "document body".to_string())]
        );
    }

    /// 起動済み leaf 群を先入れ先出しで完了させ続け、実行を終端まで進める。
    /// 完了させた leaf の (node_name, node_execution_id) を完了順で返す。
    fn drive_leaves_to_end(
        execution: &mut ExecutionTree,
        initial: Vec<NodeStart>,
        new_id: &mut dyn FnMut() -> String,
    ) -> Vec<(String, String)> {
        let mut queue: std::collections::VecDeque<NodeStart> = initial.into();
        let mut completed = Vec::new();
        let mut now = 10.0;
        while let Some(leaf) = queue.pop_front() {
            now += 1.0;
            let applied = execution
                .complete_leaf_and_advance(leaf.node_execution_id(), new_id, now)
                .unwrap();
            completed.push((
                leaf.node_name().to_string(),
                leaf.node_execution_id().to_string(),
            ));
            if let ExecutionAdvanceDecision::StartNodes(next) = applied.decision {
                queue.extend(next);
            }
        }
        completed
    }

    #[test]

    fn fanout_child_sequence_runs_recursively_and_completes_bottom_up() {
        let mut execution = tree_execution(vec![
            tree_sequence_node(
                "main",
                vec![ChildEntry::reference("fan"), ChildEntry::reference("after")],
            ),
            NodeDefinition {
                name: "fan".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: vec![ChildEntry::reference("part"), ChildEntry::reference("solo")],
                    items: None,
                }),
                ..Default::default()
            },
            tree_sequence_node(
                "part",
                vec![ChildEntry::reference("s1"), ChildEntry::reference("s2")],
            ),
            tree_command_node("s1"),
            tree_command_node("s2"),
            tree_command_node("solo"),
            tree_command_node("after"),
        ]);
        let mut new_id = tree_id_source();

        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        assert_eq!(
            started_names(&applied.events),
            ["main", "fan", "part", "s1", "solo"]
        );
        let ExecutionAdvanceDecision::StartNodes(leaves) = applied.decision else {
            panic!("nested start must yield leaves");
        };
        assert_eq!(
            leaves
                .iter()
                .map(|leaf| leaf.node_name())
                .collect::<Vec<_>>(),
            ["s1", "solo"]
        );

        // 親参照が実行木を成す: s1 → part（sequence の子）、part → fan（fanout の子）。
        let main_id = execution_id_of(&execution, "main");
        let fan_id = execution_id_of(&execution, "fan");
        let part_id = execution_id_of(&execution, "part");
        let s1 = execution
            .node_executions()
            .iter()
            .find(|node| node.node_name == "s1")
            .unwrap();
        assert_eq!(
            s1.parent,
            Some(ExecutionParentRef::sequence_child(&part_id))
        );
        let part = execution
            .node_executions()
            .iter()
            .find(|node| node.node_name == "part")
            .unwrap();
        let part_parent = part.parent.clone().unwrap();
        assert_eq!(part_parent.parent_id, fan_id);
        assert!(part_parent.fanout_slot().is_some());
        let fan = execution
            .node_executions()
            .iter()
            .find(|node| node.node_name == "fan")
            .unwrap();
        assert_eq!(
            fan.parent,
            Some(ExecutionParentRef::sequence_child(&main_id))
        );

        let completed = drive_leaves_to_end(&mut execution, leaves, &mut new_id);
        assert_eq!(
            completed
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["s1", "solo", "s2", "after"]
        );
        assert_eq!(*execution.state(), RuntimeExecutionState::Completed);
        for name in ["main", "fan", "part"] {
            assert_eq!(
                execution
                    .node_executions()
                    .iter()
                    .find(|node| node.node_name == name)
                    .unwrap()
                    .status,
                RuntimeNodeExecutionStatus::Succeeded,
                "composite instance '{name}' must complete bottom-up"
            );
        }
    }

    #[test]

    fn nested_approval_pauses_inside_the_tree_and_resumes_in_place() {
        let mut execution = tree_execution(vec![
            tree_sequence_node(
                "main",
                vec![
                    ChildEntry::reference("part"),
                    ChildEntry::reference("report"),
                ],
            ),
            NodeDefinition {
                completion: NodeCompletion::require_approval(),
                ..tree_sequence_node("part", vec![ChildEntry::reference("inner")])
            },
            tree_command_node("inner"),
            tree_command_node("report"),
        ]);
        let mut new_id = tree_id_source();

        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartNodes(leaves) = applied.decision else {
            panic!("start must yield the inner leaf");
        };
        let part_id = execution_id_of(&execution, "part");
        let applied = execution
            .complete_leaf_and_advance(leaves[0].node_execution_id(), &mut new_id, 2.0)
            .unwrap();

        // ネスト内で承認待ち停止: part は WaitingApproval、前進しない。
        assert!(applied.events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ApprovalRequested { node_execution_id, .. }
                if node_execution_id == &part_id
        )));
        assert_eq!(applied.decision, ExecutionAdvanceDecision::Persist);
        assert_eq!(
            execution
                .node_executions()
                .iter()
                .find(|node| node.id == part_id)
                .unwrap()
                .status,
            RuntimeNodeExecutionStatus::WaitingApproval
        );
        assert_eq!(execution.display_current_node(), Some("part".to_string()));

        // 承認でネスト位置から再開し、親 sequence が report へ前進する。
        let applied = execution
            .apply_approval(&part_id, &mut new_id, 3.0)
            .unwrap();
        let ExecutionAdvanceDecision::StartNodes(leaves) = applied.decision else {
            panic!("approval must resume the parent sequence");
        };
        assert_eq!(leaves[0].node_name(), "report");
        let applied = execution
            .complete_leaf_and_advance(leaves[0].node_execution_id(), &mut new_id, 4.0)
            .unwrap();
        assert!(applied
            .events
            .iter()
            .any(|event| matches!(event, WorkflowEvent::ExecutionCompleted { .. })));
        assert_eq!(*execution.state(), RuntimeExecutionState::Completed);
    }

    #[test]

    fn replay_restores_the_nested_position_for_resume() {
        let nodes = vec![
            tree_sequence_node(
                "main",
                vec![
                    ChildEntry::reference("part"),
                    ChildEntry::reference("report"),
                ],
            ),
            tree_sequence_node(
                "part",
                vec![
                    ChildEntry::reference("inner-a"),
                    ChildEntry::reference("inner-b"),
                ],
            ),
            tree_command_node("inner-a"),
            tree_command_node("inner-b"),
            tree_command_node("report"),
        ];
        let mut live = tree_execution(nodes.clone());
        let mut new_id = tree_id_source();
        let applied = live.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartNodes(leaves) = applied.decision else {
            panic!("start must yield the inner-a leaf");
        };
        let advanced = live
            .complete_leaf_and_advance(leaves[0].node_execution_id(), &mut new_id, 2.0)
            .unwrap();
        let mut events = applied.events;
        events.extend(advanced.events);

        // 事実列だけからスコープ木を再構築する（inner-b 実行中の位置）。
        let mut replayed = tree_execution(nodes);
        for event in &events {
            match event {
                WorkflowEvent::NodeStarted {
                    node_execution_id,
                    node_name,
                    kind,
                    attempt,
                    parent,
                    timestamp,
                    ..
                } => replayed
                    .replay_node_started(
                        node_execution_id,
                        node_name,
                        *kind,
                        *attempt,
                        parent.clone(),
                        *timestamp,
                    )
                    .unwrap(),
                WorkflowEvent::NodeCompleted {
                    node_execution_id,
                    timestamp,
                    ..
                } => replayed
                    .derive_session_settlement(node_execution_id, *timestamp)
                    .unwrap(),
                _ => {}
            }
        }

        let main_id = execution_id_of(&live, "main");
        let part_id = execution_id_of(&live, "part");
        let inner_b_id = execution_id_of(&live, "inner-b");
        assert!(replayed.scope(&main_id).is_some());
        assert!(replayed.scope(&part_id).is_some());
        assert_eq!(
            replayed
                .scope(&part_id)
                .and_then(ScopeRuntime::sequence)
                .and_then(|sequence| sequence.current_child.clone()),
            Some("inner-b".to_string())
        );
        assert_eq!(replayed.display_current_node(), Some("inner-b".to_string()));
        let leaf = replayed
            .leaf_start_for(&inner_b_id)
            .expect("the interrupted leaf must be restartable in place");
        assert_eq!(leaf.node_name, "inner-b");
        assert_eq!(
            replayed
                .node_executions()
                .iter()
                .find(|node| node.id == inner_b_id)
                .unwrap()
                .parent,
            Some(ExecutionParentRef::sequence_child(&part_id))
        );
    }

    #[test]

    fn parallel_fanout_lanes_keep_independent_loop_guard_counts() {
        // fan は同じ部品 sequence "part" を items 2 件で並走させる。part 内の
        // fix は loop_guard(2) で自己ループする。lane 0 が予算を使い切っても
        // lane 1 の fix は自分のスコープの予算で 2 回目に入れる。
        let mut execution = tree_execution(vec![
            tree_sequence_node("main", vec![ChildEntry::reference("fan")]),
            NodeDefinition {
                name: "fan".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: vec![ChildEntry::reference("part")],
                    items: Some(crate::domain::workflow::ItemsSource::Literal(vec![
                        serde_json::json!("a"),
                        serde_json::json!("b"),
                    ])),
                }),
                ..Default::default()
            },
            tree_sequence_node(
                "part",
                vec![
                    ChildEntry {
                        name: "fix".to_string(),
                        inputs: Vec::new(),
                        rules: Some(vec![
                            Rule::LoopGuard {
                                max_iterations: 2,
                                on_exhausted: "exit".to_string(),
                            },
                            Rule::Next("fix".to_string()),
                        ]),
                    },
                    ChildEntry::reference("exit"),
                ],
            ),
            tree_command_node("fix"),
            tree_command_node("exit"),
        ]);
        let mut new_id = tree_id_source();

        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartNodes(leaves) = applied.decision else {
            panic!("start must yield one fix leaf per lane");
        };
        assert_eq!(
            leaves
                .iter()
                .map(|leaf| leaf.node_name())
                .collect::<Vec<_>>(),
            ["fix", "fix"]
        );
        let lane_parts: Vec<String> = leaves
            .iter()
            .map(|leaf| {
                execution
                    .node_executions()
                    .iter()
                    .find(|node| node.id == leaf.node_execution_id())
                    .and_then(|node| node.parent.clone())
                    .expect("a lane fix must hang under its part instance")
                    .parent_id
            })
            .collect();
        assert_ne!(
            lane_parts[0], lane_parts[1],
            "each lane must run its own part instance"
        );

        // lane 0 が fix の予算 2 回を使い切り exit へ抜ける。
        let applied = execution
            .complete_leaf_and_advance(leaves[0].node_execution_id(), &mut new_id, 2.0)
            .unwrap();
        let ExecutionAdvanceDecision::StartNodes(lane0_second) = applied.decision else {
            panic!("lane 0 must revisit fix");
        };
        assert_eq!(lane0_second[0].node_name(), "fix");
        let applied = execution
            .complete_leaf_and_advance(lane0_second[0].node_execution_id(), &mut new_id, 3.0)
            .unwrap();
        let ExecutionAdvanceDecision::StartNodes(lane0_exit) = applied.decision else {
            panic!("lane 0 must exhaust into exit");
        };
        assert_eq!(lane0_exit[0].node_name(), "exit");

        // lane 1 の fix はカウント独立: lane 0 が 2 回消費済みでも 2 回目に入れる。
        let applied = execution
            .complete_leaf_and_advance(leaves[1].node_execution_id(), &mut new_id, 4.0)
            .unwrap();
        let ExecutionAdvanceDecision::StartNodes(lane1_second) = applied.decision else {
            panic!("lane 1 must revisit fix with its own budget");
        };
        assert_eq!(lane1_second[0].node_name(), "fix");
        assert_eq!(
            execution
                .node_executions()
                .iter()
                .find(|node| node.id == lane1_second[0].node_execution_id())
                .and_then(|node| node.parent.clone())
                .unwrap()
                .parent_id,
            lane_parts[1],
            "the second fix of lane 1 must stay in lane 1's part instance"
        );

        // 残りを流し切ると全体が完了する。
        let mut queue = vec![lane0_exit[0].clone(), lane1_second[0].clone()];
        let mut now = 5.0;
        while let Some(leaf) = queue.pop() {
            now += 1.0;
            let applied = execution
                .complete_leaf_and_advance(leaf.node_execution_id(), &mut new_id, now)
                .unwrap();
            if let ExecutionAdvanceDecision::StartNodes(next) = applied.decision {
                queue.extend(next);
            }
        }
        assert_eq!(*execution.state(), RuntimeExecutionState::Completed);
        assert_eq!(
            execution
                .node_executions()
                .iter()
                .filter(|node| node.node_name == "fix")
                .count(),
            4,
            "each lane must have run fix twice"
        );
    }

    #[test]

    fn part_sequence_input_parameters_feed_child_bindings() {
        // main は prepare の Artifact を part の input `target` に配線し、
        // part 内の worker は `target` を自分のパラメータ `data` として受け取る。
        let mut execution = tree_execution(vec![
            tree_sequence_node(
                "main",
                vec![
                    ChildEntry::reference("prepare"),
                    ChildEntry {
                        name: "part".to_string(),
                        inputs: vec![(
                            "target".to_string(),
                            crate::domain::workflow::value_objects::InputSourceRef::new("prepare"),
                        )],
                        rules: None,
                    },
                ],
            ),
            NodeDefinition {
                input: vec![crate::domain::workflow::InputParam {
                    name: "target".to_string(),
                    contract: None,
                }],
                ..tree_sequence_node(
                    "part",
                    vec![ChildEntry {
                        name: "worker".to_string(),
                        inputs: vec![(
                            "data".to_string(),
                            crate::domain::workflow::value_objects::InputSourceRef::new("target"),
                        )],
                        rules: None,
                    }],
                )
            },
            tree_command_node("prepare"),
            tree_command_node("worker"),
        ]);
        let mut new_id = tree_id_source();

        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartNodes(leaves) = applied.decision else {
            panic!("start must yield the prepare leaf");
        };
        let prepared_value = serde_json::json!({"path": "src/lib.rs"});
        assert_eq!(
            execution.record_pending_result(
                leaves[0].node_execution_id(),
                Some("done".to_string()),
                Some(prepared_value.clone()),
                None,
                None,
                2.0,
            ),
            TransitionOutcome::Applied
        );
        let applied = execution
            .complete_leaf_and_advance(leaves[0].node_execution_id(), &mut new_id, 3.0)
            .unwrap();

        // part スコープは input `target` を prepare の Artifact で束縛し、
        // worker の起動束縛は `target` から `data` を受け取る。
        let ExecutionAdvanceDecision::StartNodes(leaves) = applied.decision else {
            panic!("main must advance into part");
        };
        assert_eq!(leaves[0].node_name(), "worker");
        assert_eq!(
            expect_leaf(&leaves[0]).bindings,
            vec![("data".to_string(), prepared_value.clone())]
        );
        let part_id = execution_id_of(&execution, "part");
        assert_eq!(
            execution
                .scope(&part_id)
                .map(|scope| scope.parameters.clone()),
            Some(vec![("target".to_string(), prepared_value)])
        );
    }

    #[test]

    fn abort_records_every_active_lane_leaf_even_with_equal_name_and_attempt() {
        // fanout の並走 lane は同じ部品 sequence を走らせるため、同名 node が
        // 同一 attempt（スコープ採番）でアクティブになる。abort は全 lane の
        // leaf を記録する。
        let mut execution = tree_execution(vec![
            tree_sequence_node("main", vec![ChildEntry::reference("fan")]),
            NodeDefinition {
                name: "fan".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: vec![ChildEntry::reference("part")],
                    items: Some(crate::domain::workflow::ItemsSource::Literal(vec![
                        serde_json::json!("a"),
                        serde_json::json!("b"),
                    ])),
                }),
                ..Default::default()
            },
            tree_sequence_node("part", vec![ChildEntry::reference("fix")]),
            tree_command_node("fix"),
        ]);
        let mut new_id = tree_id_source();
        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartNodes(leaves) = applied.decision else {
            panic!("start must yield one fix leaf per lane");
        };
        assert_eq!(leaves.len(), 2);
        let fixes: Vec<_> = execution
            .node_executions()
            .iter()
            .filter(|node| node.node_name == "fix")
            .collect();
        assert_eq!(
            (fixes[0].attempt, fixes[1].attempt),
            (1, 1),
            "both lanes must carry the same scope-local attempt"
        );

        execution.record_aborted_history_for_active_leaves(2.0);

        let aborted: Vec<_> = execution
            .node_history
            .iter()
            .filter(|entry| {
                entry.node_name == "fix"
                    && entry.state == crate::domain::workflow::value_objects::NODE_STATUS_ABORTED
            })
            .collect();
        assert_eq!(
            aborted.len(),
            2,
            "every active lane leaf must get its own aborted entry"
        );
    }

    #[test]

    fn direct_fanout_child_lanes_each_start_at_attempt_one_and_retry_independently() {
        // items 2 件の直接 fanout 子（leaf）は lane ごとに attempt 1 で始まり、
        // 片方の retry だけがその lane の attempt 2 になる。
        let mut execution = tree_execution(vec![
            tree_sequence_node("main", vec![ChildEntry::reference("fan")]),
            NodeDefinition {
                name: "fan".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: vec![ChildEntry::reference("worker")],
                    items: Some(crate::domain::workflow::ItemsSource::Literal(vec![
                        serde_json::json!("a"),
                        serde_json::json!("b"),
                    ])),
                }),
                ..Default::default()
            },
            tree_command_node("worker"),
        ]);
        let mut new_id = tree_id_source();

        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartNodes(leaves) = applied.decision else {
            panic!("start must yield one worker leaf per lane");
        };
        let attempts: Vec<u32> = leaves
            .iter()
            .map(|leaf| {
                execution
                    .node_executions()
                    .iter()
                    .find(|node| node.id == leaf.node_execution_id())
                    .unwrap()
                    .attempt
            })
            .collect();
        assert_eq!(attempts, [1, 1], "each lane must start at attempt 1");

        // lane 0 を失敗させて retry すると、その lane だけ attempt 2 になる。
        let lane0 = leaves[0].node_execution_id().to_string();
        let restarted = execution
            .restart_node_attempt_at(&lane0, "retry-1".to_string(), 3.0)
            .expect("a failed lane leaf must be retryable");
        assert_eq!(restarted.attempt.attempt, 2);
        // lane 1 は attempt 1 のまま。
        assert_eq!(
            execution
                .node_executions()
                .iter()
                .find(|node| node.id == leaves[1].node_execution_id())
                .unwrap()
                .attempt,
            1
        );
    }

    #[test]

    fn revisited_part_sequence_gets_a_fresh_loop_guard_budget() {
        // main は part を loop_guard(2) で再訪し、part 内部の fix も
        // loop_guard(2) で自己ループする。カウントの範囲はスコープなので、
        // part の再訪ごとに内部カウントはフレッシュになる。
        let mut execution = tree_execution(vec![
            tree_sequence_node(
                "main",
                vec![
                    ChildEntry {
                        name: "part".to_string(),
                        inputs: Vec::new(),
                        rules: Some(vec![
                            Rule::LoopGuard {
                                max_iterations: 2,
                                on_exhausted: "finish".to_string(),
                            },
                            Rule::Next("part".to_string()),
                        ]),
                    },
                    ChildEntry::reference("finish"),
                ],
            ),
            tree_sequence_node(
                "part",
                vec![
                    ChildEntry {
                        name: "fix".to_string(),
                        inputs: Vec::new(),
                        rules: Some(vec![
                            Rule::LoopGuard {
                                max_iterations: 2,
                                on_exhausted: "exit".to_string(),
                            },
                            Rule::Next("fix".to_string()),
                        ]),
                    },
                    ChildEntry::reference("exit"),
                ],
            ),
            tree_command_node("fix"),
            tree_command_node("exit"),
            tree_command_node("finish"),
        ]);
        let mut new_id = tree_id_source();

        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartNodes(leaves) = applied.decision else {
            panic!("start must yield the first fix leaf");
        };
        let completed = drive_leaves_to_end(&mut execution, leaves, &mut new_id);

        // part 2 訪問 × 内部 fix 2 回ずつ。1 回目の消費が持ち越されるなら
        // 2 回目の fix は 1 回で exhausted になり、この列は崩れる。
        assert_eq!(
            completed
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["fix", "fix", "exit", "fix", "fix", "exit", "finish"]
        );
        assert_eq!(*execution.state(), RuntimeExecutionState::Completed);
        assert_eq!(
            execution
                .node_executions()
                .iter()
                .filter(|node| node.node_name == "part")
                .count(),
            2,
            "part must have one execution instance per visit"
        );
    }
}
