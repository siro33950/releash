use super::*;

#[test]
fn test_workflow検証変換_停止分類を保持する() {
    use crate::common::operation_context::OperationStopped;
    use crate::domain::failure::{StorageFailure, StorageFailureSource, TechnicalFailure};
    // Given
    let workflow = domain::WorkflowDefinition {
        name: "test".into(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes: Vec::new(),
        entry: "main".into(),
    };
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        // When
        let error = domain_validation_to_runtime_error(
            domain::WorkflowError::Technical(stopped.into()),
            &workflow,
        );
        // Then
        let technical = TechnicalFailure::from(stopped);
        let expected = StorageFailure {
            nature: technical.nature,
            source: StorageFailureSource::Workflow(Box::new(domain::WorkflowError::Technical(
                technical,
            ))),
            context: None,
        };
        assert!(matches!(error, WorkflowRuntimeError::Store(actual) if actual == expected));
    }
}
pub(crate) mod tests {
    use super::super::*;
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
}
