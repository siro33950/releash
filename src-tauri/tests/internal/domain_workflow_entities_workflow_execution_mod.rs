use releash_lib::test_support::integration::fixtures::fixtures_domain_workflow_entities_workflow_execution_mod_execution_id_of as execution_id_of;
use releash_lib::test_support::integration::fixtures::fixtures_domain_workflow_entities_workflow_execution_mod_id_source as id_source;
use releash_lib::test_support::integration::fixtures::fixtures_domain_workflow_entities_workflow_execution_mod_started_names as started_names;
use releash_lib::test_support::integration::workflow::workflow_execution_ExecutionTree as ExecutionTree;
use releash_lib::test_support::integration::workflow::AppliedNodeCompletionHandshake;
use releash_lib::test_support::integration::workflow::ExecutionAdvanceDecision;
use releash_lib::test_support::integration::workflow::ExecutionParentRef;
use releash_lib::test_support::integration::workflow::ExecutionTreeRestore;
use releash_lib::test_support::integration::workflow::NodeCompletionSignal;
use releash_lib::test_support::integration::workflow::NodeExecutionStatus as RuntimeNodeExecutionStatus;
use releash_lib::test_support::integration::workflow::NodeKindName;
use releash_lib::test_support::integration::workflow::TransitionOutcome;
use releash_lib::test_support::integration::workflow::WorkflowDefinition;

#[test]
pub fn test_正本サンプル_fanout内の隔離sessionがdelegateを発火して成果をmergeへ渡す() {
    // Given
    let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../workflows/examples/full-cycle-development.yml");
    let source = std::fs::read_to_string(source_path).unwrap();
    let workflow: WorkflowDefinition = serde_saphyr::from_str(&source).unwrap();
    let mut execution = ExecutionTree::restore_runtime(ExecutionTreeRestore {
        id: "canonical-example-execution".to_string(),
        repository_root: Some("/repo".into()),
        worktree_path: "/repo".into(),
        workflow,
        ..ExecutionTreeRestore::default()
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

pub(crate) fn settle_session_leaf(
    execution: &mut ExecutionTree,
    node_execution_id: &str,
    new_id: &mut dyn FnMut() -> String,
    timestamp: f64,
) -> AppliedNodeCompletionHandshake {
    assert_eq!(
        execution.record_node_completion_signal(
            node_execution_id,
            NodeCompletionSignal::Submit,
            timestamp,
        ),
        TransitionOutcome::Applied
    );
    assert_eq!(
        execution.record_node_completion_signal(
            node_execution_id,
            NodeCompletionSignal::Stop,
            timestamp,
        ),
        TransitionOutcome::Applied
    );
    execution
        .apply_node_completion_handshake(node_execution_id, new_id, timestamp)
        .unwrap()
}
