use super::*;
use serde_json::{json, Value};

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
    if leaf.kind == NodeKindName::Session {
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
    let ExecutionAdvanceDecision::StartLeaves(mut leaves) = decision else {
        panic!("expected a leaf start, got {decision:?}");
    };
    assert_eq!(leaves.len(), 1);
    leaves.remove(0)
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
        let ExecutionAdvanceDecision::StartLeaves(leaves) = completed.decision else {
            panic!("fanout must expand");
        };
        assert_eq!(leaves.len(), 2);
        for (leaf, item) in leaves.iter().zip(["first", "second"]) {
            assert_eq!(leaf.node_name, "worker");
            assert_eq!(leaf.bindings, vec![("item".to_string(), json!(item))]);
            assert_eq!(leaf.item, Some(json!(item)));
        }
    }
}
