//! Pure workflow approval transition decisions.

use crate::domain::workflow::value_objects::WorkflowDefinition;
use crate::domain::workflow::WorkflowError;

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
    if !node.is_approval_session() {
        return Err(WorkflowError::UnauthorizedApprovalTarget(
            "current node is not an approval-gated session".to_string(),
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
