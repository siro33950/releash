use crate::adaptor::gateway::workflow::domain_mapping::workflow_definition_to_domain;
use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::schema::Workflow;
use crate::domain::workflow as domain;

pub(crate) fn validate_workflow_shape(workflow: &Workflow) -> Result<(), WorkflowEngineError> {
    let definition = workflow_definition_to_domain(workflow);
    domain::validation::validate_workflow_shape(&definition)
        .map_err(|err| domain_validation_to_engine_error(err, &definition))
}

pub(crate) fn validate_start(
    workflow: &Workflow,
    existing_active_workflow_name: Option<&str>,
) -> Result<(), WorkflowEngineError> {
    validate_workflow_shape(workflow)?;
    if let Some(workflow_name) = existing_active_workflow_name {
        return Err(WorkflowEngineError::AlreadyActive(
            workflow_name.to_string(),
        ));
    }
    Ok(())
}

fn domain_validation_to_engine_error(
    err: domain::WorkflowError,
    workflow: &domain::WorkflowDefinition,
) -> WorkflowEngineError {
    match err {
        domain::WorkflowError::Validation(message) if message == "workflow has no nodes" => {
            WorkflowEngineError::InvalidWorkflow("Workflow has no steps".to_string())
        }
        domain::WorkflowError::Validation(message)
            if message.starts_with("bash node ") && message.contains("not executable") =>
        {
            let node_name = workflow
                .nodes
                .iter()
                .find(|node| matches!(node.node_type, domain::NodeType::Bash))
                .map(|node| node.name.as_str())
                .unwrap_or("unknown");
            WorkflowEngineError::InvalidWorkflow(format!(
                "Bash node '{node_name}' is not executable in this milestone (planned for [13])"
            ))
        }
        domain::WorkflowError::Validation(message) => WorkflowEngineError::InvalidWorkflow(message),
        domain::WorkflowError::InvalidState(message) => WorkflowEngineError::InvalidState(message),
        domain::WorkflowError::UnauthorizedApprovalTarget(message) => {
            WorkflowEngineError::UnauthorizedApprovalTarget(message)
        }
        domain::WorkflowError::NotFound(message) | domain::WorkflowError::External(message) => {
            WorkflowEngineError::InvalidWorkflow(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::{NodeDefinition, NodeType};

    fn workflow(nodes: Vec<NodeDefinition>) -> Workflow {
        Workflow {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            variables: Default::default(),
            nodes,
        }
    }

    #[test]
    fn validate_workflow_shape_delegates_to_domain_and_preserves_empty_message() {
        let err = validate_workflow_shape(&workflow(Vec::new())).unwrap_err();

        assert_eq!(err.to_string(), "Workflow has no steps");
    }

    #[test]
    fn validate_workflow_shape_delegates_to_domain_and_preserves_bash_message() {
        let err = validate_workflow_shape(&workflow(vec![NodeDefinition {
            name: "build".to_string(),
            node_type: NodeType::Bash,
            ..Default::default()
        }]))
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "Bash node 'build' is not executable in this milestone (planned for [13])"
        );
    }

    #[test]
    fn validate_start_rejects_active_workflow_after_shape_validation() {
        let err = validate_start(
            &workflow(vec![NodeDefinition {
                name: "plan".to_string(),
                node_type: NodeType::Agent,
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
