use super::*;
use crate::domain::workflow::{AbortRequestedFact, TreeRootFact};

fn execution() -> ExecutionTree {
    ExecutionTree::restore_without_definition(
        "tree",
        &TreeRootFact {
            repository_root: None,
            workspace_identity: "/repo".into(),
            worktree_path: "/repo".into(),
            created_from: ExecutionOrigin::Cli,
            request: String::new(),
            workflow_name: "old".into(),
            definition: None,
            launched_as: ExecutionTreeLaunch::Workflow,
        },
        1.0,
    )
}

#[test]
fn test_終端復元_abort時も成功済みfanoutだけ再試行前を除いたusageを合算する() {
    // Given
    for succeeded in [false, true] {
        let mut execution = execution();
        execution
            .begin_node_attempt(
                "fan".into(),
                NodeKindName::Fanout,
                1,
                None,
                "fan".into(),
                1.0,
            )
            .unwrap();
        for (id, attempt, usage) in [("old", 1, 100), ("new", 2, 3)] {
            execution
                .begin_node_attempt(
                    "work".into(),
                    NodeKindName::Session,
                    attempt,
                    Some(ExecutionParentRef::fanout_child("fan", None, 0)),
                    id.into(),
                    2.0,
                )
                .unwrap();
            execution.record_pending_result(
                id,
                None,
                None,
                None,
                Some(TokenUsage {
                    input_tokens: usage,
                    output_tokens: usage * 2,
                }),
                3.0,
            );
            if id == "old" {
                execution.request_node_retry(id, 4.0);
            } else {
                execution.record_node_completion_signal(id, NodeCompletionSignal::Submit, 4.0);
                execution.record_node_completion_signal(id, NodeCompletionSignal::Stop, 5.0);
                execution.complete_node_execution(
                    id,
                    None,
                    Some(TokenUsage {
                        input_tokens: usage,
                        output_tokens: usage * 2,
                    }),
                    5.0,
                );
            }
        }
        if succeeded {
            execution.complete_node_execution("fan", None, None, 6.0);
        }

        // When
        execution.replay_terminal_fact(&NodeFact::AbortRequested(Default::default()), 7.0);

        // Then
        assert_eq!(
            execution.node_execution("old").unwrap().status,
            RuntimeNodeExecutionStatus::Aborted
        );
        assert_eq!(
            execution.node_execution("fan").unwrap().token_usage,
            succeeded.then_some(TokenUsage {
                input_tokens: 3,
                output_tokens: 6,
            })
        );
    }
}

#[test]
fn test_起動時abort_未完了で定義が読めないときだけ理由付きで遷移する() {
    // Given
    let mut execution = execution();
    // When / Then
    assert!(execution.abort_unavailable_definition(None, 2.0).is_none());
    assert!(execution.is_active());
    let reason = "Workflow definition is unavailable: completion";
    assert_eq!(
        execution.abort_unavailable_definition(Some(reason.into()), 3.0),
        Some(NodeFact::AbortRequested(AbortRequestedFact {
            reason: Some(reason.into())
        }))
    );
    assert_eq!(execution.state(), &RuntimeExecutionState::Aborted);
    assert_eq!(execution.error_reason.as_deref(), Some(reason));
    assert_eq!(execution.updated_at, 3.0);
    assert!(execution
        .abort_unavailable_definition(Some("again".into()), 4.0)
        .is_none());
    assert_eq!(execution.updated_at, 3.0);
}

#[test]
fn test_起動時abort_完了とabortの既存終端を変更しない() {
    // Given
    for fact in [
        NodeFact::ExecutionCompleted,
        NodeFact::AbortRequested(AbortRequestedFact {
            reason: Some("human abort".into()),
        }),
    ] {
        let mut execution = execution();
        execution.replay_terminal_fact(&fact, 2.0);
        let before = execution.clone();
        // When
        assert!(execution
            .abort_unavailable_definition(Some("unreadable".into()), 3.0)
            .is_none());
        // Then
        assert_eq!(execution, before);
        assert!(execution.workflow.is_none());
    }
}

#[test]
fn test_起動時前進失敗_理由付きabortは実行中だけに一度適用する() {
    // Given
    let mut tree = execution();
    // When
    let fact = tree
        .abort_with_reason("advance failed".into(), 3.0)
        .unwrap();
    // Then
    assert_eq!(
        fact,
        NodeFact::AbortRequested(AbortRequestedFact {
            reason: Some("advance failed".into())
        })
    );
    assert_eq!(tree.state(), &RuntimeExecutionState::Aborted);
    assert!(tree.abort_with_reason("again".into(), 4.0).is_none());
    let mut completed = execution();
    completed.replay_terminal_fact(&NodeFact::ExecutionCompleted, 2.0);
    assert!(completed.abort_with_reason("late".into(), 3.0).is_none());
}
