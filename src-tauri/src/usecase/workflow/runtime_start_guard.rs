//! Start command admission and domain validation mapping.

use crate::domain::workflow as domain;
use crate::domain::workflow::WorkflowDefinition;
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;

pub(crate) fn validate_workflow_shape(
    workflow: &WorkflowDefinition,
) -> Result<(), WorkflowRuntimeError> {
    domain::validation::validate_workflow_shape(workflow)
        .map_err(|err| domain_validation_to_runtime_error(err, workflow))
}

pub(crate) fn validate_start(
    workflow: &WorkflowDefinition,
    existing_active_workflow_name: Option<&str>,
) -> Result<(), WorkflowRuntimeError> {
    validate_workflow_shape(workflow)?;
    if let Some(workflow_name) = existing_active_workflow_name {
        return Err(WorkflowRuntimeError::AlreadyActive(
            workflow_name.to_string(),
        ));
    }
    Ok(())
}

fn domain_validation_to_runtime_error(
    err: domain::WorkflowError,
    _workflow: &domain::WorkflowDefinition,
) -> WorkflowRuntimeError {
    match err {
        domain::WorkflowError::Validation(message) if message == "workflow has no nodes" => {
            WorkflowRuntimeError::InvalidWorkflow("Workflow has no nodes".to_string())
        }
        domain::WorkflowError::Validation(message) => {
            WorkflowRuntimeError::InvalidWorkflow(message)
        }
        domain::WorkflowError::InvalidState(message) => WorkflowRuntimeError::InvalidState(message),
        domain::WorkflowError::Conflict(message) => WorkflowRuntimeError::Conflict(message),
        domain::WorkflowError::UnauthorizedApprovalTarget(message) => {
            WorkflowRuntimeError::UnauthorizedApprovalTarget(message)
        }
        domain::WorkflowError::NotFound(message)
        | domain::WorkflowError::External(message)
        | domain::WorkflowError::StorageUnavailable { message, .. }
        | domain::WorkflowError::CorruptStoredState(message)
        | domain::WorkflowError::IncompatibleStoredEvent(message) => {
            WorkflowRuntimeError::InvalidWorkflow(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::NodeDefinition;

    fn workflow(nodes: Vec<NodeDefinition>) -> WorkflowDefinition {
        let entry = nodes
            .first()
            .map(|node| node.name.clone())
            .unwrap_or_else(|| "main".to_string());
        WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes,
            entry,
        }
    }

    #[test]
    fn validate_workflow_shape_delegates_to_domain_and_preserves_empty_message() {
        let err = validate_workflow_shape(&workflow(Vec::new())).unwrap_err();

        assert_eq!(err.to_string(), "Workflow has no nodes");
    }

    #[test]
    fn validate_start_rejects_active_workflow_after_shape_validation() {
        let err = validate_start(
            &workflow(vec![NodeDefinition {
                name: "plan".to_string(),
                ..Default::default()
            }]),
            Some("wf"),
        )
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "Workflow 'wf' is already running for this session"
        );
    }
}
