use super::*;
use serde_json::{json, Value};

fn execution(nodes: &str) -> WorkflowExecution {
    WorkflowExecution::restore_runtime(WorkflowExecutionRestore {
        id: "execution".into(),
        workflow: serde_saphyr::from_str(&format!(
            "name: isolated\ndescription: test\nnodes:\n{nodes}"
        ))
        .unwrap(),
        worktree_path: "/repo-root-worktrees/development".into(),
        repository_root: Some("/repo-root".into()),
        ..Default::default()
    })
}

fn ids() -> impl FnMut() -> String {
    let mut count = 0;
    move || {
        count += 1;
        format!("node-{count}")
    }
}

fn leaves(advance: AppliedAdvance) -> Vec<NodeStart> {
    match advance.decision {
        ExecutionAdvanceDecision::StartNodes(leaves) => leaves,
        other => panic!("expected starts: {other:?}"),
    }
}

fn complete(
    execution: &mut WorkflowExecution,
    start: &NodeStart,
    artifact: Option<Value>,
    ids: &mut dyn FnMut() -> String,
) -> AppliedAdvance {
    let leaf = expect_leaf(start);
    if artifact.is_some() {
        assert_eq!(
            execution.record_pending_result(
                &leaf.node_execution_id,
                None,
                artifact,
                None,
                None,
                2.0
            ),
            TransitionOutcome::Applied
        );
    }
    if leaf.kind == LeafKind::Session {
        for signal in [NodeCompletionSignal::Submit, NodeCompletionSignal::Stop] {
            assert_eq!(
                execution.record_node_completion_signal(&leaf.node_execution_id, signal, 3.0),
                TransitionOutcome::Applied
            );
        }
        let completed = execution
            .apply_node_completion_handshake(&leaf.node_execution_id, ids, 3.0)
            .unwrap();
        AppliedAdvance {
            decision: completed.advance.unwrap(),
            events: completed.events,
        }
    } else {
        execution
            .complete_leaf_and_advance(&leaf.node_execution_id, ids, 3.0)
            .unwrap()
    }
}

#[test]
fn test_隔離実行_sequence準備完了後だけchildを始めsharedと省略は親を継承する() {
    // Given
    let mut execution = execution("  main: {worktree: isolated, sequence: {children: [first, second]}}\n  first: {session: {provider: codex}}\n  second: {worktree: shared, command: 'true'}");
    let mut ids = ids();

    // When
    let root = leaves(execution.start_root(&mut ids, 1.0).unwrap()).remove(0);
    let path = execution
        .execution_worktree_path(root.node_execution_id())
        .unwrap()
        .to_string();

    // Then
    assert_eq!(execution.node_executions.len(), 1);
    assert_eq!(
        execution.parent_worktree_path(root.node_execution_id()),
        Some("/repo-root-worktrees/development")
    );
    assert!(path.starts_with("/repo-root-worktrees/.releash-isolated/"));
    let first = leaves(
        execution
            .start_prepared_composite(root.node_execution_id(), &mut ids, 1.0)
            .unwrap(),
    )
    .remove(0);
    assert_eq!(
        execution.execution_worktree_path(first.node_execution_id()),
        Some(path.as_str())
    );
    assert!(execution
        .node_execution(first.node_execution_id())
        .unwrap()
        .worktree
        .is_none());
    let second = leaves(complete(&mut execution, &first, None, &mut ids)).remove(0);
    assert_eq!(
        execution.execution_worktree_path(second.node_execution_id()),
        Some(path.as_str())
    );
    complete(&mut execution, &second, Some(json!({"ok": true})), &mut ids);
    let artifact = execution
        .node_execution(root.node_execution_id())
        .unwrap()
        .artifact
        .as_ref()
        .unwrap();
    assert_eq!(artifact["worktree"]["path"], path);
    assert_eq!(artifact["second"]["ok"], true);
    assert!(artifact.get("first").is_none());
}

#[test]
fn test_隔離実行_fanout本体は単一環境を共有し隔離childはslotごとの環境を持つ() {
    // Given
    for isolated_children in [false, true] {
        let child_mode = if isolated_children {
            "isolated"
        } else {
            "shared"
        };
        let mut execution = execution(&format!("  main: {{worktree: isolated, fanout: {{children: [one, two]}}}}\n  one: {{worktree: {child_mode}, session: {{provider: codex}}}}\n  two: {{worktree: {child_mode}, session: {{provider: codex}}}}"));
        let mut ids = ids();
        let root = leaves(execution.start_root(&mut ids, 1.0).unwrap()).remove(0);
        let root_path = execution
            .execution_worktree_path(root.node_execution_id())
            .unwrap()
            .to_string();

        // When
        let slots = leaves(
            execution
                .start_prepared_composite(root.node_execution_id(), &mut ids, 1.0)
                .unwrap(),
        );

        // Then
        assert_eq!(slots.len(), 2);
        let paths = slots
            .iter()
            .map(|slot| {
                execution
                    .execution_worktree_path(slot.node_execution_id())
                    .unwrap()
                    .to_string()
            })
            .collect::<Vec<_>>();
        if isolated_children {
            assert_ne!(paths[0], paths[1]);
        } else {
            assert_eq!(paths, vec![root_path.clone(); 2]);
        }
        for slot in &slots {
            assert_eq!(
                execution.parent_worktree_path(slot.node_execution_id()),
                Some(root_path.as_str())
            );
            complete(&mut execution, slot, None, &mut ids);
        }
        let artifact = execution
            .node_execution(root.node_execution_id())
            .unwrap()
            .artifact
            .as_ref()
            .unwrap();
        assert_eq!(artifact["worktree"]["path"], root_path);
        for slot in slots {
            let value = &artifact[slot.node_name()];
            if isolated_children {
                assert!(value["worktree"]["path"].is_string());
            } else {
                assert!(value.is_null());
            }
        }
    }
}

#[test]
fn test_隔離実行_隔離sessionの成果を配線し後続はrootで実行する() {
    // Given
    let mut execution = execution("  main:\n    sequence:\n      children:\n        - part\n        - report: {inputs: {path: part.work.worktree.path, branch: part.work.worktree.branch}}\n  part: {sequence: {children: [work]}}\n  work: {worktree: isolated, session: {provider: codex}}\n  report: {input: [path, branch], command: 'true', env: {PATH_VALUE: path, BRANCH: branch}}");
    let mut ids = ids();
    let work = leaves(execution.start_root(&mut ids, 1.0).unwrap()).remove(0);
    let worktree = execution
        .node_execution(work.node_execution_id())
        .unwrap()
        .worktree
        .clone()
        .unwrap();

    // When
    let report = leaves(complete(&mut execution, &work, None, &mut ids)).remove(0);

    // Then
    assert_eq!(
        expect_leaf(&report).bindings,
        vec![
            ("path".into(), json!(worktree.path)),
            ("branch".into(), json!(worktree.branch))
        ]
    );
    let command = execution
        .workflow
        .node_by_name("report")
        .unwrap()
        .command_spec()
        .unwrap();
    let environment = workflow_reference::resolve_command_environment(
        &command.env,
        &expect_leaf(&report).bindings,
    )
    .unwrap();
    assert!(environment.contains(&("BRANCH".into(), worktree.branch.clone())));
    assert_eq!(
        execution.execution_worktree_path(report.node_execution_id()),
        Some("/repo-root-worktrees/development")
    );
    assert_eq!(
        execution
            .node_execution(work.node_execution_id())
            .unwrap()
            .artifact,
        Some(json!({"worktree": {"path": worktree.path, "branch": worktree.branch}}))
    );
}

#[test]
fn test_隔離実行_自動retryは異なる識別子とattemptでworktreeを導出する() {
    // Given
    let mut execution = execution("  main: {sequence: {children: [{work: {on_failure: {retry: 1}}}]}}\n  work: {worktree: isolated, session: {provider: codex}}");
    let mut ids = ids();
    let first = leaves(execution.start_root(&mut ids, 1.0).unwrap()).remove(0);
    let previous = execution
        .node_execution(first.node_execution_id())
        .unwrap()
        .worktree
        .clone()
        .unwrap();

    // When
    assert_eq!(
        execution.fail_leaf_execution(
            first.node_execution_id(),
            "creation failed".into(),
            NodeExecutionFailureKind::InfrastructureCrash,
            FailureDisposition::Terminal,
            2.0
        ),
        TransitionOutcome::Applied
    );
    let retry = execution
        .apply_on_failure_treatment(first.node_execution_id(), &mut ids, 2.0)
        .unwrap()
        .unwrap()
        .starts
        .remove(0);

    // Then
    let node = execution.node_execution(retry.node_execution_id()).unwrap();
    assert_eq!(node.attempt, 2);
    assert_ne!(node.id, first.node_execution_id());
    assert_ne!(node.worktree.as_ref().unwrap().path, previous.path);
    assert!(node.worktree.as_ref().unwrap().branch.ends_with("-a2"));
    assert_eq!(
        execution
            .node_execution(first.node_execution_id())
            .unwrap()
            .worktree
            .as_ref(),
        Some(&previous)
    );
}

#[test]
fn test_隔離実行_items展開された同じsequenceのslotを独立させる() {
    // Given
    let mut execution = execution("  main: {fanout: {items: [x, y], children: [group]}}\n  group: {worktree: isolated, sequence: {children: [work]}}\n  work: {session: {provider: codex}}");
    let mut ids = ids();

    // When
    let groups = leaves(execution.start_root(&mut ids, 1.0).unwrap());

    // Then
    assert_eq!(groups.len(), 2);
    let paths = groups
        .iter()
        .map(|group| {
            execution
                .execution_worktree_path(group.node_execution_id())
                .unwrap()
                .to_string()
        })
        .collect::<Vec<_>>();
    assert_ne!(paths[0], paths[1]);
    for (index, group) in groups.iter().enumerate() {
        let work = leaves(
            execution
                .start_prepared_composite(group.node_execution_id(), &mut ids, 2.0)
                .unwrap(),
        )
        .remove(0);
        assert_eq!(
            execution.execution_worktree_path(work.node_execution_id()),
            Some(paths[index].as_str())
        );
        complete(&mut execution, &work, None, &mut ids);
    }
    let artifact = execution.node_executions[0].artifact.as_ref().unwrap();
    assert_eq!(artifact["0"]["worktree"]["path"], paths[0]);
    assert_eq!(artifact["1"]["worktree"]["path"], paths[1]);
}

#[test]
fn test_隔離実行_contractの成果にworktreeを追加して保持する() {
    // Given
    for kind in ["command: 'true'", "session: {provider: codex}"] {
        let mut execution = execution(&format!("  main: {{worktree: isolated, {kind}}}"));
        let mut ids = ids();
        let leaf = leaves(execution.start_root(&mut ids, 1.0).unwrap()).remove(0);
        let expected = execution
            .node_execution(leaf.node_execution_id())
            .unwrap()
            .worktree
            .clone()
            .unwrap();

        // When
        complete(
            &mut execution,
            &leaf,
            Some(json!({"summary": "done", "ok": true})),
            &mut ids,
        );

        // Then
        assert_eq!(
            execution
                .node_execution(leaf.node_execution_id())
                .unwrap()
                .artifact,
            Some(
                json!({"summary": "done", "ok": true, "worktree": {"path": expected.path, "branch": expected.branch}})
            )
        );
    }
}

#[test]
fn test_隔離実行_repository_rootがない開始記録を拒否する() {
    // Given
    let mut execution = execution("  main: {worktree: isolated, command: 'true'}");
    execution.runtime.repository_root = None;

    // When / Then
    assert!(execution.start_root(&mut ids(), 1.0).is_err());
    assert_eq!(
        execution.begin_node_attempt(
            "main".into(),
            NodeKindName::Command,
            1,
            None,
            "node".into(),
            1.0
        ),
        Err(TransitionRejection::MissingRepositoryRoot)
    );
    assert!(execution.node_executions.is_empty());
}

#[test]
fn test_隔離実行_正本のfix_allはfix_and_verifyをslotごとに隔離し修正と検証が継承する() {
    // Given
    let workflow = serde_saphyr::from_str(include_str!(
        "../../../../../../workflows/examples/full-cycle-development.yml"
    ))
    .unwrap();
    let mut execution = WorkflowExecution::restore_runtime(WorkflowExecutionRestore {
        id: "canonical-fix-execution".into(),
        workflow,
        worktree_path: "/repo".into(),
        repository_root: Some("/repo".into()),
        ..Default::default()
    });
    assert!(execution
        .workflow
        .node_by_name("fix_and_verify")
        .unwrap()
        .is_isolated());
    execution
        .replay_node_started("main", "main", NodeKindName::Sequence, 1, None, 1.0)
        .unwrap();
    execution
        .replay_node_started(
            "fix-round",
            "fix_round",
            NodeKindName::Sequence,
            1,
            Some(ExecutionParentRef::sequence_child("main")),
            2.0,
        )
        .unwrap();
    execution
        .replay_node_started(
            "plan",
            "create_fix_plan",
            NodeKindName::Session,
            1,
            Some(ExecutionParentRef::sequence_child("fix-round")),
            3.0,
        )
        .unwrap();
    let plan = execution.leaf_start_for("plan").unwrap();
    let tasks = (1..=2)
        .map(|index| {
            json!({
                "task_id": format!("task-{index}"),
                "thread_id": format!("thread-{index}"),
                "target_files": [],
                "implementation_steps": [],
                "acceptance_criteria": [],
                "non_goals": [],
                "source_policy": "policy"
            })
        })
        .collect::<Vec<_>>();
    let mut ids = ids();

    // When
    let slots = leaves(complete(
        &mut execution,
        &NodeStart::Leaf(plan),
        Some(json!({"tasks": tasks, "summary": "two fixes"})),
        &mut ids,
    ));

    // Then
    assert_eq!(slots.len(), 2);
    let worktrees = slots
        .iter()
        .map(|slot| {
            assert_eq!(slot.node_name(), "fix_and_verify");
            let node = execution.node_execution(slot.node_execution_id()).unwrap();
            let parent = node.parent.as_ref().unwrap();
            assert_eq!(
                execution
                    .node_execution(&parent.parent_id)
                    .unwrap()
                    .node_name,
                "fix_all"
            );
            assert!(parent.fanout_slot.is_some());
            assert_eq!(execution.parent_worktree_path(&node.id), Some("/repo"));
            node.worktree.clone().unwrap()
        })
        .collect::<Vec<_>>();
    assert_ne!(worktrees[0].path, worktrees[1].path);
    assert_ne!(worktrees[0].branch, worktrees[1].branch);
    for (slot, worktree) in slots.iter().zip(worktrees) {
        let fixes = leaves(
            execution
                .start_prepared_composite(slot.node_execution_id(), &mut ids, 4.0)
                .unwrap(),
        );
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].node_name(), "fix_task");
        assert_eq!(
            execution.execution_worktree_path(fixes[0].node_execution_id()),
            Some(worktree.path.as_str())
        );
        let verifies = leaves(complete(&mut execution, &fixes[0], None, &mut ids));
        assert_eq!(verifies.len(), 1);
        assert_eq!(verifies[0].node_name(), "verify_fix");
        assert_eq!(
            execution.execution_worktree_path(verifies[0].node_execution_id()),
            Some(worktree.path.as_str())
        );
    }
}

#[test]
fn test_起動契約_隔離合成子は準備要求だけを返し葉runtimeとして起動できない() {
    for kind in ["sequence", "fanout"] {
        // Given
        let mut execution = execution(&format!("  main: {{worktree: isolated, {kind}: {{children: [work]}}}}\n  work: {{command: 'true'}}"));
        let mut ids = ids();
        // When
        let starts = leaves(execution.start_root(&mut ids, 1.0).unwrap());
        // Then
        let [NodeStart::PrepareComposite(composite)] = starts.as_slice() else {
            panic!("composite must require preparation");
        };
        assert!(execution
            .leaf_start_for(&composite.node_execution_id)
            .is_err());
        let prepared = execution
            .start_prepared_composite(&composite.node_execution_id, &mut ids, 2.0)
            .unwrap();
        let children = leaves(prepared);
        let [NodeStart::Leaf(leaf)] = children.as_slice() else {
            panic!("command must have a leaf runtime");
        };
        assert_eq!(leaf.kind, LeafKind::Command);
        assert_eq!(leaf.node_name, "work");
    }
}
