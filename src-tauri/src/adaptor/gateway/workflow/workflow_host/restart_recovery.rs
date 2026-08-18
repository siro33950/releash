//! Canonical event-log hydration for active Workflow restart reconciliation.
//!
//! 事実ログを集約へ replay した実行木（`ActiveRestartProjection.aggregate`）を
//! そのまま live registry へ戻す。復旧専用の再構築経路は持たない。

use super::resume_projection::ActiveRestartProjection;
use super::*;

fn invalid(message: impl Into<String>) -> WorkflowRuntimeError {
    WorkflowRuntimeError::InvalidState(message.into())
}

pub(super) fn hydrate_restart_execution(
    checkpoint: &ActiveRestartProjection,
) -> Result<DomainWorkflowExecution, WorkflowRuntimeError> {
    match checkpoint.projected_execution.status {
        crate::domain::workflow::ExecutionStatus::Running => {}
        other => {
            return Err(invalid(format!(
                "restart reconciliation cannot hydrate workflow status {}",
                other.as_str()
            )));
        }
    }
    let has_active_node = checkpoint
        .aggregate
        .node_executions
        .iter()
        .any(|node| node.status.is_active());
    if !has_active_node {
        return Err(invalid("restart reconciliation has no active node attempt"));
    }
    Ok(checkpoint.aggregate.clone())
}

#[cfg(test)]
mod restart_recovery_tests {
    use super::*;
    use crate::adaptor::gateway::workflow::workflow_host::resume_projection;
    use crate::domain::workflow::{
        ChildEntry, ExecutionOrigin, NodeDefinition, NodeExecutionFailureKind, NodeKind,
        NodeKindName, SequenceSpec, WorkflowDefinition, WorkflowEvent,
    };
    use crate::domain::workflow::{ExecutionParentRef, TokenUsage};

    const EXECUTION_ID: &str = "00000000-0000-4000-8000-000000000001";

    fn nested_definition() -> WorkflowDefinition {
        WorkflowDefinition {
            name: "review".to_string(),
            entry: "main".to_string(),
            nodes: vec![
                NodeDefinition {
                    name: "main".to_string(),
                    kind: NodeKind::Sequence(SequenceSpec {
                        entry: None,
                        output: None,
                        children: vec![
                            ChildEntry::reference("inner-a"),
                            ChildEntry::reference("inner-b"),
                        ],
                    }),
                    ..Default::default()
                },
                NodeDefinition {
                    name: "inner-a".to_string(),
                    ..Default::default()
                },
                NodeDefinition {
                    name: "inner-b".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    fn started() -> WorkflowEvent {
        WorkflowEvent::ExecutionStarted {
            execution_id: EXECUTION_ID.to_string(),
            workflow_name: "review".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::Cli,
            request: "please review".to_string(),
            definition: nested_definition(),
            timestamp: 1.0,
        }
    }

    fn node_started(
        id: &str,
        name: &str,
        kind: NodeKindName,
        parent: Option<ExecutionParentRef>,
        timestamp: f64,
    ) -> WorkflowEvent {
        WorkflowEvent::NodeStarted {
            execution_id: EXECUTION_ID.to_string(),
            node_execution_id: id.to_string(),
            node_name: name.to_string(),
            kind,
            attempt: 1,
            parent,
            timestamp,
        }
    }

    fn nested_running_events() -> Vec<WorkflowEvent> {
        vec![
            started(),
            node_started("seq-1", "main", NodeKindName::Sequence, None, 2.0),
            node_started(
                "leaf-a",
                "inner-a",
                NodeKindName::Session,
                Some(ExecutionParentRef::sequence_child("seq-1")),
                2.0,
            ),
            // session leaf は二信号（Submit + Stop）が揃って初めて完了できる。
            WorkflowEvent::NodeSubmitReceived {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "leaf-a".to_string(),
                timestamp: 2.5,
            },
            WorkflowEvent::NodeStopReceived {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "leaf-a".to_string(),
                timestamp: 2.6,
            },
            WorkflowEvent::NodeCompleted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "leaf-a".to_string(),
                node_name: "inner-a".to_string(),
                attempt: 1,
                result_summary: None,
                token_usage: None,
                timestamp: 3.0,
            },
            node_started(
                "leaf-b",
                "inner-b",
                NodeKindName::Session,
                Some(ExecutionParentRef::sequence_child("seq-1")),
                4.0,
            ),
        ]
    }

    #[test]
    fn restart_checkpoint_hydrates_the_nested_position_from_the_event_log() {
        let checkpoint =
            resume_projection::project_restart_checkpoint(EXECUTION_ID, &nested_running_events())
                .unwrap();

        let hydrated = hydrate_restart_execution(&checkpoint).unwrap();

        // 事実ログだけからスコープ木とネスト位置（inner-b 実行中）が戻る。
        assert!(hydrated.scope("seq-1").is_some());
        assert_eq!(hydrated.display_current_node(), Some("inner-b".to_string()));
        let leaf = hydrated
            .leaf_start_for("leaf-b")
            .expect("the interrupted leaf must be restartable in place");
        assert_eq!(leaf.node_name, "inner-b");
    }

    #[test]
    fn restart_checkpoint_rejects_an_execution_without_an_active_node() {
        let mut events = nested_running_events();
        events.push(WorkflowEvent::NodeFailed {
            execution_id: EXECUTION_ID.to_string(),
            node_execution_id: "leaf-b".to_string(),
            node_name: "inner-b".to_string(),
            attempt: 1,
            reason: "exit 1".to_string(),
            failure_kind: NodeExecutionFailureKind::ValidationFailure,
            retry_count: None,
            timestamp: 5.0,
        });
        // main sequence インスタンスも畳まれた失敗停止状態を作る。
        events.push(WorkflowEvent::NodeFailed {
            execution_id: EXECUTION_ID.to_string(),
            node_execution_id: "seq-1".to_string(),
            node_name: "main".to_string(),
            attempt: 1,
            reason: "child failed".to_string(),
            failure_kind: NodeExecutionFailureKind::ValidationFailure,
            retry_count: None,
            timestamp: 5.0,
        });

        let checkpoint =
            resume_projection::project_restart_checkpoint(EXECUTION_ID, &events).unwrap();

        assert!(hydrate_restart_execution(&checkpoint).is_err());
    }

    #[test]
    fn restart_checkpoint_rejects_a_completed_execution() {
        let mut events = nested_running_events();
        for (id, name) in [("leaf-b", "inner-b"), ("seq-1", "main")] {
            events.push(WorkflowEvent::NodeCompleted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: id.to_string(),
                node_name: name.to_string(),
                attempt: 1,
                result_summary: None,
                token_usage: None,
                timestamp: 6.0,
            });
        }
        events.push(WorkflowEvent::ExecutionCompleted {
            execution_id: EXECUTION_ID.to_string(),
            total_token_usage: TokenUsage::default(),
            timestamp: 7.0,
        });

        let checkpoint =
            resume_projection::project_restart_checkpoint(EXECUTION_ID, &events).unwrap();

        assert!(hydrate_restart_execution(&checkpoint).is_err());
    }
}
