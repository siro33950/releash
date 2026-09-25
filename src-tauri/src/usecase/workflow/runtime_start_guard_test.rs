use super::*;

#[test]
fn test_workflow検証変換_停止分類を保持する() {
    use crate::domain::failure::ClassifiedFailure;
    use crate::domain::operation_context::OperationStopped;
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
        let error =
            domain_validation_to_runtime_error(domain::WorkflowError::Stopped(stopped), &workflow);
        // Then
        assert_eq!(error.failure_kind(), stopped.failure_kind());
    }
}
