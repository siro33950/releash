pub(crate) mod tests {
    use super::super::*;

    fn sequence_scope() -> SequenceScopeRuntime {
        SequenceScopeRuntime::default()
    }

    #[test]
    fn record_child_start_counts_from_one_and_increases_monotonically() {
        let mut scope = sequence_scope();

        assert_eq!(scope.record_child_start("fix"), 1);
        assert_eq!(scope.record_child_start("fix"), 2);
        assert_eq!(scope.record_child_start("exit"), 1);
        assert_eq!(scope.record_child_start("fix"), 3);
    }

    #[test]
    fn raise_child_count_to_never_lowers_the_count() {
        let mut scope = sequence_scope();
        scope.raise_child_count_to("fix", 3);
        assert_eq!(scope.record_child_start("fix"), 4);

        scope.raise_child_count_to("fix", 2);
        assert_eq!(scope.record_child_start("fix"), 5);
    }
}

use super::super::{ExecutionTreeRestore, RuntimeNodeExecutionStatus as S};
use super::*;
use crate::domain::workflow::{
    ChildEntry, ExecutionParentRef, NodeKind, NodeKindName, SequenceSpec,
};

#[test]
fn test_合成子の成果_再試行前の子を除き開始順と最新成果を保つ() {
    // Given
    let mut aggregate = ExecutionTree::restore_runtime(ExecutionTreeRestore::default());
    for (id, name, status, value) in [
        ("old", "a", S::Aborted, "old"),
        ("other", "b", S::Succeeded, "other"),
        ("new", "a", S::Succeeded, "new"),
    ] {
        aggregate
            .runtime
            .node_executions
            .push(RuntimeNodeExecution {
                worktree: None,
                id: id.into(),
                execution_id: "tree".into(),
                node_name: name.into(),
                kind: NodeKindName::Session,
                attempt: 1,
                status,
                session_id: None,
                display_command: None,
                shell: None,
                cwd: None,
                artifact: Some(serde_json::json!(value)),
                result_summary: None,
                token_usage: None,
                parent: Some(ExecutionParentRef::sequence_child("scope")),
                completion_signals: Default::default(),
                started_at: 0.0,
                completed_at: None,
            });
    }
    aggregate
        .runtime
        .retry_predecessors
        .insert("new".into(), "old".into());
    let node = NodeDefinition {
        name: "scope".into(),
        kind: NodeKind::Sequence(SequenceSpec {
            entry: None,
            children: vec![ChildEntry::reference("a"), ChildEntry::reference("b")],
        }),
        ..Default::default()
    };
    // When / Then
    assert_eq!(
        aggregate
            .scope_children("scope")
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>(),
        ["other", "new"]
    );
    assert_eq!(
        aggregate
            .finish_composite("scope", &node, S::Running, false, None)
            .unwrap(),
        Some((
            S::Succeeded,
            Some(serde_json::json!({"a":"new", "b":"other"}))
        ))
    );
}
