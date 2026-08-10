//! Resume orchestration for durable runtime state.

use super::*;

fn runtime_node_kind_name(kind: crate::domain::workflow::NodeKindName) -> NodeKindName {
    match kind {
        crate::domain::workflow::NodeKindName::Session => NodeKindName::Session,
        crate::domain::workflow::NodeKindName::Fanout => NodeKindName::Fanout,
        crate::domain::workflow::NodeKindName::Command => NodeKindName::Command,
    }
}

pub(super) fn runtime_token_usage(usage: &crate::domain::workflow::TokenUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    }
}

pub(super) fn runtime_node_execution(
    node: &crate::domain::workflow::NodeExecution,
) -> NodeExecution {
    NodeExecution {
        id: node.id.clone(),
        execution_id: node.execution_id.clone(),
        node_name: node.node_name.clone(),
        kind: runtime_node_kind_name(node.kind),
        attempt: node.attempt,
        status: match node.status {
            crate::domain::workflow::NodeExecutionStatus::Running => NodeExecutionStatus::Running,
            crate::domain::workflow::NodeExecutionStatus::Paused => NodeExecutionStatus::Paused,
            crate::domain::workflow::NodeExecutionStatus::WaitingApproval => {
                NodeExecutionStatus::WaitingApproval
            }
            crate::domain::workflow::NodeExecutionStatus::Succeeded => {
                NodeExecutionStatus::Succeeded
            }
            crate::domain::workflow::NodeExecutionStatus::Failed => NodeExecutionStatus::Failed,
            crate::domain::workflow::NodeExecutionStatus::Aborted => NodeExecutionStatus::Aborted,
        },
        session_id: node.session_id.clone(),
        display_command: node.display_command.clone(),
        artifact: node
            .artifact
            .as_ref()
            .map(|artifact| artifact.value.clone()),
        token_usage: node.token_usage.as_ref().map(runtime_token_usage),
        failure: node.failure.as_ref().map(|failure| {
            crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionFailure {
                reason: failure.reason.clone(),
                kind: failure.kind,
            }
        }),
        fanout_parent: node.fanout_parent.as_ref().map(|parent| {
            crate::domain::workflow::FanoutParentRef {
                parent_node: parent.parent_node.clone(),
                parent_attempt: parent.parent_attempt,
                item_index: parent.item_index,
                child_index: parent.child_index,
            }
        }),
        completion_signals: node.completion_signals,
        started_at: node.started_at,
        completed_at: node.completed_at,
    }
}
