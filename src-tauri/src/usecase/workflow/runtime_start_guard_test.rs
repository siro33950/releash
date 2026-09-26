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
