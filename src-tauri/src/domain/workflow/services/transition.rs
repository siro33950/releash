//! Pure workflow approval transition decisions.

use crate::domain::workflow::value_objects::{NodeDefinition, WorkflowDefinition};
use crate::domain::workflow::WorkflowError;

/// 既定の完了条件（session 二信号 / command exit code / fanout 全子完了）を
/// 満たした node の処遇。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionDisposition {
    Complete,
    RequestApproval,
}

pub fn decide_completion_disposition(node: &NodeDefinition) -> CompletionDisposition {
    if node.requires_approval_completion() {
        CompletionDisposition::RequestApproval
    } else {
        CompletionDisposition::Complete
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalApplication {
    pub effective_result: String,
    pub artifact: Option<serde_json::Value>,
    pub contract: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalCompletion {
    pub result: String,
    pub artifact: Option<serde_json::Value>,
    pub contract: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalApplicationPlan {
    pub completion: ApprovalCompletion,
}

pub fn decide_approve_action(
    workflow: &WorkflowDefinition,
    current_index: usize,
    node_is_waiting_approval: bool,
) -> Result<(), WorkflowError> {
    if !node_is_waiting_approval {
        return Err(WorkflowError::invalid_state(
            "Node is not waiting for approval",
        ));
    }
    let node = workflow.nodes.get(current_index).ok_or_else(|| {
        WorkflowError::validation(format!("node index out of range: {current_index}"))
    })?;
    if !node.requires_approval_completion() {
        return Err(WorkflowError::UnauthorizedApprovalTarget(
            "current node does not declare completion: approval".to_string(),
        ));
    }
    Ok(())
}

pub fn plan_approval_application(
    workflow: &WorkflowDefinition,
    current_index: usize,
    node_is_waiting_approval: bool,
    application: ApprovalApplication,
) -> Result<ApprovalApplicationPlan, WorkflowError> {
    decide_approve_action(workflow, current_index, node_is_waiting_approval)?;
    Ok(ApprovalApplicationPlan {
        completion: ApprovalCompletion {
            result: application.effective_result,
            artifact: application.artifact,
            contract: application.contract,
        },
    })
}

#[cfg(test)]
mod transition_tests {
    use super::*;
    use crate::domain::workflow::value_objects::{
        CommandSpec, FanoutSpec, NodeCompletion, NodeDefinition, NodeKind,
    };

    fn node(name: &str, kind: NodeKind, completion: NodeCompletion) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind,
            completion,
            ..Default::default()
        }
    }

    fn workflow(nodes: Vec<NodeDefinition>) -> WorkflowDefinition {
        let entry = nodes[0].name.clone();
        WorkflowDefinition {
            name: "wf".to_string(),
            nodes,
            entry,
            ..Default::default()
        }
    }

    #[test]
    fn test_approve_completion_approvalは全kindで承認対象になる() {
        let command_kind = NodeKind::Command(CommandSpec {
            command: "true".to_string(),
        });
        let fanout_kind = NodeKind::Fanout(FanoutSpec {
            children: vec![crate::domain::workflow::ChildEntry::reference("worker")],
            items: None,
        });
        for kind in [NodeKind::default(), command_kind, fanout_kind] {
            let wf = workflow(vec![node("main", kind, NodeCompletion::Approval)]);
            assert!(decide_approve_action(&wf, 0, true).is_ok());
        }
    }

    #[test]
    fn test_approve_completion_autoのnodeは承認対象にならない() {
        let wf = workflow(vec![node(
            "main",
            NodeKind::Command(CommandSpec {
                command: "true".to_string(),
            }),
            NodeCompletion::Auto,
        )]);

        assert!(matches!(
            decide_approve_action(&wf, 0, true),
            Err(WorkflowError::UnauthorizedApprovalTarget(_))
        ));
    }

    #[test]
    fn test_approve_waiting_approvalでないnodeは承認できない() {
        let wf = workflow(vec![node(
            "main",
            NodeKind::default(),
            NodeCompletion::Approval,
        )]);

        assert!(decide_approve_action(&wf, 0, false).is_err());
    }
}
