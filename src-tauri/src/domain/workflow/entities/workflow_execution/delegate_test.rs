use super::*;
use crate::domain::workflow::{
    CommandSpec, NodeCompletion, NodeKind, Predicate, SessionDelegate, SessionSpec,
};
use serde_json::json;

fn fixture(
    when: &str,
    max_iterations: u32,
    approval: bool,
) -> (WorkflowExecution, impl FnMut() -> String, String) {
    let parent = NodeDefinition {
        name: "main".into(),
        kind: NodeKind::Session(SessionSpec::default()),
        artifact: Some("result".into()),
        completion: NodeCompletion {
            require: approval.then_some(crate::domain::workflow::CompletionRequirement::Approval),
            delegate: Some(SessionDelegate {
                child: "verify".into(),
                inputs: Vec::new(),
                when: Predicate::Ref(when.into()),
                max_iterations,
            }),
        },
        ..Default::default()
    };
    let child = NodeDefinition {
        name: "verify".into(),
        kind: NodeKind::Command(CommandSpec {
            command: "check".into(),
            env: Default::default(),
        }),
        ..Default::default()
    };
    let mut execution = WorkflowExecution::restore_runtime(WorkflowExecutionRestore {
        id: "tree".into(),
        workflow: WorkflowDefinition {
            name: "delegate".into(),
            entry: "main".into(),
            nodes: vec![parent, child],
            ..Default::default()
        },
        ..Default::default()
    });
    let mut n = 0;
    let mut ids = move || {
        n += 1;
        format!("node-{n}")
    };
    execution.start_root(&mut ids, 1.0).unwrap();
    let parent = execution.node_executions()[0].id.clone();
    execution.attach_node_session(&parent, "agent-session".into(), 1.0);
    (execution, ids, parent)
}

fn submit(
    execution: &mut WorkflowExecution,
    parent: &str,
    value: serde_json::Value,
    ids: &mut dyn FnMut() -> String,
) -> AppliedNodeCompletionHandshake {
    assert_eq!(
        execution.record_node_completion_signal(parent, NodeCompletionSignal::Submit, 2.0),
        TransitionOutcome::Applied
    );
    assert_eq!(
        execution.apply_submitted_output(
            execution.node_execution(parent).unwrap().node_name.clone(),
            parent,
            1,
            Some("agent-session".into()),
            "result".into(),
            value,
            None,
            2.0
        ),
        TransitionOutcome::Applied
    );
    execution
        .apply_node_completion_handshake(parent, ids, 2.0)
        .unwrap()
}

fn stop(
    execution: &mut WorkflowExecution,
    parent: &str,
    ids: &mut dyn FnMut() -> String,
) -> AppliedNodeCompletionHandshake {
    execution.record_node_completion_signal(parent, NodeCompletionSignal::Stop, 3.0);
    execution
        .apply_node_completion_handshake(parent, ids, 3.0)
        .unwrap()
}

fn child_id(execution: &WorkflowExecution, parent: &str) -> String {
    execution
        .node_executions
        .iter()
        .rev()
        .find(|node| {
            node.parent
                .as_ref()
                .is_some_and(|reference| reference.parent_id == parent)
        })
        .unwrap()
        .id
        .clone()
}

fn finish_child(
    execution: &mut WorkflowExecution,
    parent: &str,
    value: serde_json::Value,
    ids: &mut dyn FnMut() -> String,
) -> AppliedAdvance {
    let id = child_id(execution, parent);
    assert_eq!(
        execution.record_pending_result(&id, None, Some(value), None, None, 4.0),
        TransitionOutcome::Applied
    );
    execution.complete_leaf_and_advance(&id, ids, 4.0).unwrap()
}

#[test]
fn test_delegate_親の述語が真ならchildを起こさず二信号で完了する() {
    // Given
    let (mut execution, mut ids, parent) = fixture("done", 2, false);
    // When
    let submitted = submit(&mut execution, &parent, json!({"done": true}), &mut ids);
    // Then
    assert!(submitted.advance.is_none());
    assert_eq!(execution.node_executions.len(), 1);
    assert_eq!(
        execution.node_execution(&parent).unwrap().artifact,
        Some(json!({"done": true, "child": null}))
    );
    stop(&mut execution, &parent, &mut ids);
    assert_eq!(
        execution.node_execution(&parent).unwrap().status,
        RuntimeNodeExecutionStatus::Succeeded
    );
}

#[test]
fn test_delegate_提出で発火し同じsessionで再提出し上限後は最後の結果を保持する() {
    // Given
    let (mut execution, mut ids, parent) = fixture("child.passed", 2, false);
    // When
    for attempt in 1..=2 {
        let submitted = submit(&mut execution, &parent, json!({"round": attempt}), &mut ids);
        assert!(matches!(
            submitted.advance,
            Some(ExecutionAdvanceDecision::StartNodes(_))
        ));
        let child = execution
            .node_execution(&child_id(&execution, &parent))
            .unwrap();
        assert_eq!(child.attempt, attempt);
        assert_eq!(
            child.parent,
            Some(ExecutionParentRef::delegate_child(&parent))
        );
        assert_eq!(
            execution
                .node_execution(&parent)
                .unwrap()
                .artifact
                .as_ref()
                .unwrap()["child"],
            json!(null)
        );
        stop(&mut execution, &parent, &mut ids);
        finish_child(
            &mut execution,
            &parent,
            json!({"passed": false, "round": attempt}),
            &mut ids,
        );
        let injection = execution.pending_delegate_injection(&parent).unwrap();
        assert_eq!(
            execution.record_delegate_injected(&injection, 5.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.record_delegate_injected(&injection, 5.0),
            TransitionOutcome::NotApplicable
        );
        let node = execution.node_execution(&parent).unwrap();
        assert_eq!(node.status, RuntimeNodeExecutionStatus::Running);
        assert_eq!(node.attempt, 1);
        assert_eq!(node.session_id.as_deref(), Some("agent-session"));
    }
    submit(&mut execution, &parent, json!({"round": 3}), &mut ids);
    stop(&mut execution, &parent, &mut ids);
    // Then
    assert_eq!(execution.node_executions.len(), 3);
    let node = execution.node_execution(&parent).unwrap();
    assert_eq!(node.status, RuntimeNodeExecutionStatus::Succeeded);
    assert_eq!(
        node.artifact.as_ref().unwrap()["child"],
        json!({"passed": false, "round": 2})
    );
}

#[test]
fn test_delegate_上限後の提出は最後のchild結果でsequenceのgive_upへ進む() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str(
        r#"
name: delegate-routing
description: test
schemas:
  result: {type: object, properties: {round: {type: integer}}, required: [round]}
  verdict: {type: object, properties: {passed: {type: boolean}, round: {type: integer}}, required: [passed, round]}
nodes:
  main:
    sequence:
      children:
        - implement:
            rules:
              - when: {on: child.passed, then: done}
                next: give_up
        - done: {rules: []}
        - give_up
  implement:
    session: {provider: codex}
    artifact: result
    completion:
      delegate: {child: verify, when: child.passed, max_iterations: 2}
  verify: {command: check, artifact: verdict}
  done: {command: done}
  give_up: {command: give_up}
"#,
    )
    .unwrap();
    let mut execution = WorkflowExecution::restore_runtime(WorkflowExecutionRestore {
        id: "tree".into(),
        workflow,
        ..Default::default()
    });
    let mut index = 0;
    let mut ids = || {
        index += 1;
        format!("node-{index}")
    };
    execution.start_root(&mut ids, 1.0).unwrap();
    let parent = execution.node_executions.last().unwrap().id.clone();
    execution.attach_node_session(&parent, "agent-session".into(), 1.0);
    for round in 1..=2 {
        submit(&mut execution, &parent, json!({"round": round}), &mut ids);
        stop(&mut execution, &parent, &mut ids);
        finish_child(
            &mut execution,
            &parent,
            json!({"passed": false, "round": round}),
            &mut ids,
        );
        let injection = execution.pending_delegate_injection(&parent).unwrap();
        execution.record_delegate_injected(&injection, 5.0);
    }

    // When
    let submitted = submit(&mut execution, &parent, json!({"round": 3}), &mut ids);
    let stopped = stop(&mut execution, &parent, &mut ids);

    // Then
    assert!(submitted.advance.is_none());
    let Some(ExecutionAdvanceDecision::StartNodes(starts)) = stopped.advance else {
        panic!("上限後は Sequence の後続 Node が起動する");
    };
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0].node_name(), "give_up");
    assert_eq!(
        execution
            .node_executions
            .iter()
            .filter(|node| node.node_name == "verify")
            .count(),
        2
    );
    assert!(!execution
        .node_executions
        .iter()
        .any(|node| node.node_name == "done"));
    let parent = execution.node_execution(&parent).unwrap();
    assert_eq!(parent.status, RuntimeNodeExecutionStatus::Succeeded);
    assert_eq!(
        parent.artifact.as_ref().unwrap()["child"],
        json!({"passed": false, "round": 2})
    );
    assert_eq!(
        execution.node_executions.last().unwrap().status,
        RuntimeNodeExecutionStatus::Running
    );
}

#[test]
fn test_delegate_childの成功はstopを待ち承認条件と併記できる() {
    // Given
    let (mut execution, mut ids, parent) = fixture("child.passed", 2, true);
    submit(&mut execution, &parent, json!({}), &mut ids);
    // When
    finish_child(&mut execution, &parent, json!({"passed": true}), &mut ids);
    // Then
    assert_eq!(
        execution.node_execution(&parent).unwrap().status,
        RuntimeNodeExecutionStatus::Running
    );
    assert!(execution.pending_delegate_injection(&parent).is_none());
    stop(&mut execution, &parent, &mut ids);
    assert_eq!(
        execution.node_execution(&parent).unwrap().status,
        RuntimeNodeExecutionStatus::WaitingApproval
    );
    assert_eq!(
        execution
            .node_execution(&parent)
            .unwrap()
            .artifact
            .as_ref()
            .unwrap()["child"],
        json!({"passed": true})
    );
    execution.apply_approval(&parent, &mut ids, 5.0).unwrap();
    assert_eq!(
        execution.node_execution(&parent).unwrap().status,
        RuntimeNodeExecutionStatus::Succeeded
    );
}

#[test]
fn test_delegate_注入前の中断は結果を再利用し注入後の中断は再注入しない() {
    // Given
    let (mut execution, mut ids, parent) = fixture("child.passed", 2, false);
    submit(&mut execution, &parent, json!({}), &mut ids);
    stop(&mut execution, &parent, &mut ids);
    finish_child(&mut execution, &parent, json!({"passed": false}), &mut ids);
    // When
    execution.pause_node_execution(&parent, 5.0);
    assert!(execution.pending_delegate_injection(&parent).is_none());
    execution.resume_node_execution(&parent, 6.0);
    let injection = execution.pending_delegate_injection(&parent).unwrap();
    execution.record_delegate_injected(&injection, 7.0);
    execution.pause_node_execution(&parent, 8.0);
    execution.resume_node_execution(&parent, 9.0);
    // Then
    assert!(execution.pending_delegate_injection(&parent).is_none());
    assert_eq!(execution.node_executions.len(), 2);
    assert_eq!(
        execution
            .node_execution(&child_id(&execution, &parent))
            .unwrap()
            .status,
        RuntimeNodeExecutionStatus::Succeeded
    );
}

#[test]
fn test_delegate_inputsは親の最新提出と前回childとrequestから起動ごとに解決する() {
    use crate::domain::workflow::value_objects::{InputParam, InputSourceRef};
    // Given
    let (mut execution, mut ids, parent) = fixture("child.passed", 3, false);
    execution.runtime.request = Some("user request".into());
    let node = execution
        .runtime
        .workflow
        .nodes
        .iter_mut()
        .find(|node| node.name == "main")
        .unwrap();
    node.completion.delegate.as_mut().unwrap().inputs = [
        ("current", "main.round"),
        ("previous", "main.child"),
        ("goal", "request"),
    ]
    .into_iter()
    .map(|(name, source)| (name.into(), InputSourceRef::new(source)))
    .collect();
    execution
        .runtime
        .workflow
        .nodes
        .iter_mut()
        .find(|node| node.name == "verify")
        .unwrap()
        .input = ["current", "previous", "goal"]
        .into_iter()
        .map(|name| InputParam {
            name: name.into(),
            contract: None,
        })
        .collect();
    for round in 1..=2 {
        // When
        submit(&mut execution, &parent, json!({"round": round}), &mut ids);
        let id = child_id(&execution, &parent);
        let child = execution.leaf_start_for(&id).unwrap();
        let values: HashMap<_, _> = child.bindings.into_iter().collect();
        // Then
        assert_eq!(values["current"], round);
        assert_eq!(values["goal"], "user request");
        assert_eq!(
            values["previous"],
            if round == 1 {
                json!(null)
            } else {
                json!({"passed": false, "round": 1})
            }
        );
        stop(&mut execution, &parent, &mut ids);
        finish_child(
            &mut execution,
            &parent,
            json!({"passed": false, "round": round}),
            &mut ids,
        );
        let injection = execution.pending_delegate_injection(&parent).unwrap();
        execution.record_delegate_injected(&injection, 5.0);
    }
}

#[test]
fn test_delegate_承認だけではchild待ちと未成立の続行を完了できない() {
    // Given
    let (mut execution, mut ids, parent) = fixture("child.passed", 2, true);
    submit(&mut execution, &parent, json!({}), &mut ids);
    stop(&mut execution, &parent, &mut ids);
    // When / Then
    assert!(execution.apply_approval(&parent, &mut ids, 4.0).is_err());
    assert_eq!(
        execution.node_execution(&parent).unwrap().status,
        RuntimeNodeExecutionStatus::Running
    );
    finish_child(&mut execution, &parent, json!({"passed": false}), &mut ids);
    assert!(execution.apply_approval(&parent, &mut ids, 5.0).is_err());
    let injection = execution.pending_delegate_injection(&parent).unwrap();
    execution.record_delegate_injected(&injection, 6.0);
    assert!(execution.apply_approval(&parent, &mut ids, 7.0).is_err());
    assert_eq!(
        execution.node_execution(&parent).unwrap().status,
        RuntimeNodeExecutionStatus::Running
    );
    assert_eq!(execution.node_executions.len(), 2);
}

fn execution_from_source(source: &str) -> (WorkflowExecution, impl FnMut() -> String) {
    let workflow: WorkflowDefinition = serde_saphyr::from_str(source).unwrap();
    let errors = crate::domain::workflow::services::validation::validate_all(&workflow);
    assert!(errors.is_empty(), "{errors:?}");
    let mut execution = WorkflowExecution::restore_runtime(WorkflowExecutionRestore {
        id: "tree".into(),
        workflow,
        request: Some("specification".into()),
        ..Default::default()
    });
    let mut index = 0;
    let mut ids = move || {
        index += 1;
        format!("node-{index}")
    };
    execution.start_root(&mut ids, 1.0).unwrap();
    (execution, ids)
}

#[test]
fn test_delegate_合成子から親に配線したinputを初回と再発火のchildへ渡す() {
    // Given
    let (mut execution, mut ids) = execution_from_source(
        r#"
name: delegate-input
description: test
schemas:
  result: {type: object, properties: {done: {type: boolean}}, required: [done]}
nodes:
  main:
    sequence:
      children:
        - implement: {inputs: {spec: request}}
  implement:
    session: {provider: codex, facets: {instruction: implement}}
    input: [spec]
    artifact: result
    completion:
      delegate: {child: verify, inputs: {spec: spec}, when: child.ok, max_iterations: 2}
  verify: {command: check, input: [spec]}
"#,
    );
    let parent = execution.node_executions.last().unwrap().id.clone();
    execution.attach_node_session(&parent, "agent-session".into(), 1.0);
    assert_eq!(
        execution.leaf_start_for(&parent).unwrap().bindings,
        vec![("spec".into(), json!("specification"))]
    );
    for _ in 1..=2 {
        // When
        submit(&mut execution, &parent, json!({"done": false}), &mut ids);
        let child = child_id(&execution, &parent);
        // Then
        assert_eq!(
            execution.leaf_start_for(&child).unwrap().bindings,
            vec![("spec".into(), json!("specification"))]
        );
        stop(&mut execution, &parent, &mut ids);
        finish_child(&mut execution, &parent, json!({"ok": false}), &mut ids);
        let injection = execution.pending_delegate_injection(&parent).unwrap();
        execution.record_delegate_injected(&injection, 5.0);
    }
}

#[test]
fn test_delegate_下流配線は最後のchild結果を受け辺もその結果で分岐する() {
    // Given
    let (mut execution, mut ids) = execution_from_source(
        r#"
name: delegate-downstream
description: test
schemas:
  result: {type: object, properties: {done: {type: boolean}}, required: [done]}
nodes:
  main:
    sequence:
      children:
        - implement:
            rules:
              - when: {on: child.ok, then: done}
                next: failed
        - done:
            inputs: {verdict: implement.child.ok}
            rules: []
        - failed
  implement:
    session: {provider: codex, facets: {instruction: implement}}
    artifact: result
    completion:
      delegate: {child: verify, when: child.ok, max_iterations: 2}
  verify: {command: check}
  done: {command: done, input: [verdict]}
  failed: {command: failed}
"#,
    );
    let parent = execution.node_executions.last().unwrap().id.clone();
    execution.attach_node_session(&parent, "agent-session".into(), 1.0);
    for passed in [false, true] {
        // When
        submit(&mut execution, &parent, json!({"done": false}), &mut ids);
        stop(&mut execution, &parent, &mut ids);
        finish_child(&mut execution, &parent, json!({"ok": passed}), &mut ids);
        if !passed {
            let injection = execution.pending_delegate_injection(&parent).unwrap();
            execution.record_delegate_injected(&injection, 5.0);
        }
    }
    // Then
    let downstream = execution.node_executions.last().unwrap();
    assert_eq!(downstream.node_name, "done");
    assert_eq!(
        execution.leaf_start_for(&downstream.id).unwrap().bindings,
        vec![("verdict".into(), json!(true))]
    );
    assert!(!execution
        .node_executions
        .iter()
        .any(|node| node.node_name == "failed"));
    assert_eq!(
        execution
            .node_execution(&parent)
            .unwrap()
            .artifact
            .as_ref()
            .unwrap()["child"]["ok"],
        true
    );
}

#[test]
fn test_delegate_child待ち中の再提出は消失と異なる状態拒否になり成果を変更しない() {
    // Given
    let (mut execution, mut ids, parent) = fixture("child.passed", 2, false);
    submit(&mut execution, &parent, json!({"done": false}), &mut ids);
    let before = execution.node_execution(&parent).unwrap().artifact.clone();
    // When
    let outcome = execution.apply_submitted_output(
        "main".into(),
        &parent,
        1,
        None,
        "result".into(),
        json!({"done": true}),
        None,
        3.0,
    );
    let missing = execution.apply_submitted_output(
        "main".into(),
        "missing",
        1,
        None,
        "result".into(),
        json!({}),
        None,
        3.0,
    );
    // Then
    assert_eq!(
        outcome,
        TransitionOutcome::Rejected(TransitionRejection::ArtifactNotAccepted)
    );
    assert_eq!(missing, TransitionOutcome::NotApplicable);
    assert_eq!(execution.node_execution(&parent).unwrap().artifact, before);
    assert!(execution.delegate_waits_for_child(&parent));
    assert_eq!(execution.node_executions.len(), 2);
}

#[test]
fn test_delegate_falseのchildがstop前に完了してもstop後に一度だけ注入可能になる() {
    for interrupted in [false, true] {
        // Given
        let (mut execution, mut ids, parent) = fixture("child.passed", 2, false);
        submit(&mut execution, &parent, json!({}), &mut ids);
        let child = child_id(&execution, &parent);
        finish_child(&mut execution, &parent, json!({"passed": false}), &mut ids);
        assert!(execution.pending_delegate_injection(&parent).is_none());
        assert_eq!(
            execution
                .node_execution(&parent)
                .unwrap()
                .completion_signals,
            NodeCompletionSignalState::SubmitReceived
        );
        // When
        if interrupted {
            execution.pause_node_execution(&parent, 5.0);
            execution.resume_node_execution(&parent, 6.0);
            assert!(execution.pending_delegate_injection(&parent).is_none());
        }
        stop(&mut execution, &parent, &mut ids);
        let injection = execution.pending_delegate_injection(&parent).unwrap();
        // Then
        assert_eq!(injection.child_execution_id, child);
        assert_eq!(execution.node_executions.len(), 2);
        assert_eq!(
            execution.node_execution(&child).unwrap().status,
            RuntimeNodeExecutionStatus::Succeeded
        );
        let node = execution.node_execution(&parent).unwrap();
        assert_eq!(
            node.artifact.as_ref().unwrap()["child"],
            json!({"passed": false})
        );
        assert_eq!(node.session_id.as_deref(), Some("agent-session"));
        assert_eq!(node.attempt, 1);
        assert_eq!(
            execution.record_delegate_injected(&injection, 7.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.record_delegate_injected(&injection, 8.0),
            TransitionOutcome::NotApplicable
        );
        stop(&mut execution, &parent, &mut ids);
        assert!(execution.pending_delegate_injections().is_empty());
    }
}

#[test]
fn test_delegate_提出の受理と再生はchild合成と述語と上限で同じ遷移になる() {
    for (done, exhausted) in [(false, false), (true, false), (false, true)] {
        // Given
        let (mut live, mut ids, parent) = fixture("done", 1, false);
        if exhausted {
            submit(&mut live, &parent, json!({"done": false}), &mut ids);
            stop(&mut live, &parent, &mut ids);
            finish_child(&mut live, &parent, json!({"passed": false}), &mut ids);
            let injection = live.pending_delegate_injection(&parent).unwrap();
            live.record_delegate_injected(&injection, 5.0);
        }
        let mut replayed = live.clone();
        // When
        for execution in [&mut live, &mut replayed] {
            execution.record_node_completion_signal(&parent, NodeCompletionSignal::Submit, 6.0);
        }
        let submitted = json!({"done": done});
        assert_eq!(
            live.apply_submitted_output(
                "main".into(),
                &parent,
                1,
                Some("agent-session".into()),
                "result".into(),
                submitted.clone(),
                None,
                6.0
            ),
            TransitionOutcome::Applied
        );
        assert_eq!(
            replayed.replay_artifact_produced(
                &parent,
                "main",
                Some("result".into()),
                submitted,
                6.0
            ),
            TransitionOutcome::Applied
        );
        // Then
        assert_eq!(live.node_executions, replayed.node_executions);
        assert_eq!(live.delegates, replayed.delegates);
        assert_eq!(
            live.derive_pending_advances(),
            replayed.derive_pending_advances()
        );
        assert_eq!(
            live.derive_pending_advances().len(),
            usize::from(!done && !exhausted)
        );
        assert_eq!(
            live.node_execution(&parent)
                .unwrap()
                .artifact
                .as_ref()
                .unwrap()["child"],
            if exhausted {
                json!({"passed": false})
            } else {
                json!(null)
            }
        );
    }
}
