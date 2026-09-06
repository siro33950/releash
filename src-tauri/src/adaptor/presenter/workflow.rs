use crate::adaptor::protocol::workflow as workflow_wire;
use crate::domain::workflow;

use std::collections::HashSet;

pub fn workflow_execution_to_view(
    execution: workflow::WorkflowExecution,
) -> workflow_wire::WorkflowExecutionView {
    let retryable_node_ids = execution.retryable_node_execution_ids();
    workflow_wire::WorkflowExecutionView {
        id: execution.id,
        workflow_name: execution.workflow_name,
        status: execution_status_to_view(execution.status),
        current_node: execution.current_node,
        worktree_path: execution.worktree_path,
        created_from: execution_origin_to_view(execution.created_from),
        started_at: execution.started_at,
        updated_at: execution.updated_at,
        completed_at: execution.completed_at,
        error_reason: execution.error_reason,
        interruption_reason: execution
            .interruption_reason
            .map(execution_interruption_reason_to_view),
        resume_from_node: execution.resume_from_node,
        total_token_usage: token_usage_to_view(execution.total_token_usage),
        node_executions: execution
            .node_executions
            .into_iter()
            .map(|node| {
                let can_retry = retryable_node_ids.contains(&node.id);
                node_execution_to_view_with_retry(node, can_retry)
            })
            .collect(),
        artifacts: execution
            .artifacts
            .into_iter()
            .map(artifact_to_view)
            .collect(),
        fanouts: execution
            .fanouts
            .into_iter()
            .map(|fanout| fanout_to_view(fanout, &retryable_node_ids))
            .collect(),
        approval_target: execution.approval_target.map(approval_target_to_view),
    }
}

fn execution_status_to_view(
    status: workflow::ExecutionStatus,
) -> workflow_wire::ExecutionStatusView {
    match status {
        workflow::ExecutionStatus::Running => workflow_wire::ExecutionStatusView::Running,
        #[cfg(test)]
        workflow::ExecutionStatus::WaitingApproval => {
            workflow_wire::ExecutionStatusView::WaitingApproval
        }
        workflow::ExecutionStatus::Completed => workflow_wire::ExecutionStatusView::Completed,
        workflow::ExecutionStatus::Aborted => workflow_wire::ExecutionStatusView::Aborted,
        #[cfg(test)]
        workflow::ExecutionStatus::Interrupted => workflow_wire::ExecutionStatusView::Interrupted,
    }
}

fn execution_origin_to_view(
    origin: workflow::ExecutionOrigin,
) -> workflow_wire::ExecutionOriginView {
    match origin {
        workflow::ExecutionOrigin::DesktopUi => workflow_wire::ExecutionOriginView::DesktopUi,
        workflow::ExecutionOrigin::Cli => workflow_wire::ExecutionOriginView::Cli,
        workflow::ExecutionOrigin::Agent => workflow_wire::ExecutionOriginView::Agent,
        workflow::ExecutionOrigin::Api => workflow_wire::ExecutionOriginView::Api,
    }
}

fn execution_interruption_reason_to_view(
    reason: workflow::ExecutionInterruptionReason,
) -> workflow_wire::ExecutionInterruptionReasonView {
    match reason {
        workflow::ExecutionInterruptionReason::Crash => {
            workflow_wire::ExecutionInterruptionReasonView::Crash
        }
        workflow::ExecutionInterruptionReason::Stale => {
            workflow_wire::ExecutionInterruptionReasonView::Stale
        }
        workflow::ExecutionInterruptionReason::Stop => {
            workflow_wire::ExecutionInterruptionReasonView::Stop
        }
        workflow::ExecutionInterruptionReason::Orphan => {
            workflow_wire::ExecutionInterruptionReasonView::Orphan
        }
    }
}

fn token_usage_to_view(usage: workflow::TokenUsage) -> workflow_wire::TokenUsageView {
    workflow_wire::TokenUsageView {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    }
}

fn artifact_to_view(artifact: workflow::Artifact) -> workflow_wire::ArtifactView {
    workflow_wire::ArtifactView {
        node_name: artifact.node_name,
        contract: artifact.contract,
        value: artifact.value,
        produced_at: artifact.produced_at,
    }
}

pub fn node_execution_to_view(node: workflow::NodeExecution) -> workflow_wire::NodeExecutionView {
    let can_retry = node.can_retry();
    node_execution_to_view_with_retry(node, can_retry)
}

fn node_execution_to_view_with_retry(
    node: workflow::NodeExecution,
    can_retry: bool,
) -> workflow_wire::NodeExecutionView {
    let (submit_received, stop_received, waiting_for) =
        completion_signal_view(node.completion_signals);
    let can_approve = node.status == workflow::NodeExecutionStatus::WaitingApproval;
    let has_artifact = node.artifact.is_some();
    workflow_wire::NodeExecutionView {
        recovery_reason: node.recovery_reason.clone(),
        id: node.id,
        execution_id: node.execution_id,
        node_name: node.node_name,
        kind: node_kind_to_view(node.kind),
        attempt: node.attempt,
        status: node_status_to_view(node.status),
        submit_received,
        stop_received,
        waiting_for,
        can_approve,
        can_retry,
        has_artifact,
        session_id: node.session_id,
        display_command: node.display_command,
        result_summary: node.result_summary,
        artifact: node.artifact.map(artifact_to_view),
        token_usage: node.token_usage.map(token_usage_to_view),
        failure: node
            .failure
            .map(|failure| workflow_wire::NodeExecutionFailureView {
                reason: failure.reason,
                kind: failure_kind_to_view(failure.kind),
            }),
        parent: node
            .parent
            .map(|parent| workflow_wire::ExecutionParentRefView {
                parent_id: parent.parent_id,
                item_index: parent.fanout_slot.and_then(|slot| slot.item_index),
                child_index: parent.fanout_slot.map(|slot| slot.child_index),
            }),
        started_at: node.started_at,
        completed_at: node.completed_at,
    }
}

fn fanout_to_view(
    fanout: workflow::Fanout,
    retryable_node_ids: &HashSet<String>,
) -> workflow_wire::FanoutView {
    workflow_wire::FanoutView {
        parent: {
            let can_retry = retryable_node_ids.contains(&fanout.parent.id);
            node_execution_to_view_with_retry(fanout.parent, can_retry)
        },
        children: fanout
            .children
            .into_iter()
            .map(|child| {
                let can_retry = retryable_node_ids.contains(&child.id);
                node_execution_to_view_with_retry(child, can_retry)
            })
            .collect(),
        artifact: fanout.artifact.map(artifact_to_view),
    }
}

fn completion_signal_view(
    state: workflow::NodeCompletionSignalState,
) -> (bool, bool, Option<workflow_wire::NodeCompletionSignalView>) {
    match state {
        workflow::NodeCompletionSignalState::Pending => (false, false, None),
        workflow::NodeCompletionSignalState::SubmitReceived => (
            true,
            false,
            Some(workflow_wire::NodeCompletionSignalView::Stop),
        ),
        workflow::NodeCompletionSignalState::StopReceived => (
            false,
            true,
            Some(workflow_wire::NodeCompletionSignalView::Submit),
        ),
        workflow::NodeCompletionSignalState::Ready => (true, true, None),
    }
}

fn approval_target_to_view(target: workflow::ApprovalTarget) -> workflow_wire::ApprovalTargetView {
    workflow_wire::ApprovalTargetView {
        node_execution_id: target.node_execution_id,
        node_name: target.node_name,
        session_id: target.session_id,
    }
}

fn node_kind_to_view(kind: workflow::NodeKindName) -> workflow_wire::NodeKindView {
    match kind {
        workflow::NodeKindName::Command => workflow_wire::NodeKindView::Command,
        workflow::NodeKindName::Session => workflow_wire::NodeKindView::Session,
        workflow::NodeKindName::Fanout => workflow_wire::NodeKindView::Fanout,
        workflow::NodeKindName::Sequence => workflow_wire::NodeKindView::Sequence,
    }
}

fn node_status_to_view(
    status: workflow::NodeExecutionStatus,
) -> workflow_wire::NodeExecutionStatusView {
    match status {
        workflow::NodeExecutionStatus::Unresolved => {
            workflow_wire::NodeExecutionStatusView::Unresolved
        }
        workflow::NodeExecutionStatus::Running => workflow_wire::NodeExecutionStatusView::Running,
        workflow::NodeExecutionStatus::Paused => workflow_wire::NodeExecutionStatusView::Paused,
        workflow::NodeExecutionStatus::WaitingApproval => {
            workflow_wire::NodeExecutionStatusView::WaitingApproval
        }
        workflow::NodeExecutionStatus::Succeeded => {
            workflow_wire::NodeExecutionStatusView::Succeeded
        }
        workflow::NodeExecutionStatus::Failed => workflow_wire::NodeExecutionStatusView::Failed,
        workflow::NodeExecutionStatus::Aborted => workflow_wire::NodeExecutionStatusView::Aborted,
    }
}

fn failure_kind_to_view(
    kind: workflow::NodeExecutionFailureKind,
) -> workflow_wire::NodeExecutionFailureKindView {
    match kind {
        workflow::NodeExecutionFailureKind::StartupTimeout => {
            workflow_wire::NodeExecutionFailureKindView::StartupTimeout
        }
        workflow::NodeExecutionFailureKind::StaleRuntimeTimeout => {
            workflow_wire::NodeExecutionFailureKindView::StaleRuntimeTimeout
        }
        workflow::NodeExecutionFailureKind::ModelRefusal => {
            workflow_wire::NodeExecutionFailureKindView::ModelRefusal
        }
        workflow::NodeExecutionFailureKind::StructuredOutputMismatch => {
            workflow_wire::NodeExecutionFailureKindView::StructuredOutputMismatch
        }
        workflow::NodeExecutionFailureKind::ValidationFailure => {
            workflow_wire::NodeExecutionFailureKindView::ValidationFailure
        }
        workflow::NodeExecutionFailureKind::UserAbort => {
            workflow_wire::NodeExecutionFailureKindView::UserAbort
        }
        workflow::NodeExecutionFailureKind::InfrastructureCrash => {
            workflow_wire::NodeExecutionFailureKindView::InfrastructureCrash
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(node_name: &str) -> workflow::Artifact {
        workflow::Artifact {
            node_name: node_name.to_string(),
            contract: Some("result".to_string()),
            value: serde_json::json!({"ok": true}),
            produced_at: 2.0,
        }
    }

    fn node() -> workflow::NodeExecution {
        workflow::NodeExecution {
            recovery_reason: None,
            id: "node-1".to_string(),
            execution_id: "execution-1".to_string(),
            node_name: "review".to_string(),
            kind: workflow::NodeKindName::Session,
            attempt: 1,
            status: workflow::NodeExecutionStatus::WaitingApproval,
            session_id: Some("session-1".to_string()),
            display_command: None,
            result_summary: None,
            artifact: Some(artifact("review")),
            token_usage: Some(workflow::TokenUsage {
                input_tokens: 3,
                output_tokens: 2,
            }),
            failure: None,
            parent: None,
            completion_signals: workflow::NodeCompletionSignalState::StopReceived,
            started_at: 1.5,
            completed_at: None,
        }
    }

    #[test]
    fn maps_complete_public_read_model_without_legacy_wrapper() {
        let node = node();
        let execution = workflow::WorkflowExecution {
            id: "execution-1".to_string(),
            workflow_name: "review".to_string(),
            status: workflow::ExecutionStatus::Interrupted,
            current_node: Some("review".to_string()),
            created_from: workflow::ExecutionOrigin::Cli,
            worktree_path: "/repo".to_string(),
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
            interruption_reason: Some(workflow::ExecutionInterruptionReason::Stop),
            resume_from_node: Some("review".to_string()),
            total_token_usage: workflow::TokenUsage {
                input_tokens: 3,
                output_tokens: 2,
            },
            node_executions: vec![node.clone()],
            artifacts: vec![artifact("request")],
            fanouts: vec![workflow::Fanout {
                parent: node,
                children: Vec::new(),
                artifact: Some(artifact("review")),
            }],
            approval_target: Some(workflow::ApprovalTarget {
                node_execution_id: "node-1".to_string(),
                node_name: "review".to_string(),
                session_id: Some("session-1".to_string()),
            }),
        };

        let value = serde_json::to_value(workflow_execution_to_view(execution)).unwrap();
        assert_eq!(value["id"], "execution-1");
        assert_eq!(value["currentNode"], "review");
        assert_eq!(value["nodeExecutions"][0]["artifact"]["nodeName"], "review");
        assert_eq!(value["fanouts"][0]["parent"]["id"], "node-1");
        assert_eq!(value["approvalTarget"]["sessionId"], "session-1");
        assert_eq!(value["nodeExecutions"][0]["submitReceived"], false);
        assert_eq!(value["nodeExecutions"][0]["stopReceived"], true);
        assert_eq!(value["nodeExecutions"][0]["waitingFor"], "submit");
        assert_eq!(value["nodeExecutions"][0]["canApprove"], true);
        assert_eq!(value["nodeExecutions"][0]["canRetry"], false);
        assert_eq!(value["nodeExecutions"][0]["hasArtifact"], true);
        assert_eq!(value["interruptionReason"], "stop");
        assert_eq!(value["resumeFromNode"], "review");
    }

    #[test]
    fn maps_masked_command_display_to_camel_case_wire_field() {
        let mut command = node();
        command.kind = workflow::NodeKindName::Command;
        command.session_id = None;
        command.display_command = Some("printf '[REDACTED]'".to_string());

        let value = serde_json::to_value(node_execution_to_view(command)).unwrap();

        assert_eq!(value["displayCommand"], "printf '[REDACTED]'");
        assert!(value.get("display_command").is_none());
    }
}
