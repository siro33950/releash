use super::*;

#[test]
fn test_workflow境界_停止分類を保持する() {
    use crate::common::operation_context::OperationStopped;
    // Given
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        // When
        let error =
            runtime_error_to_workflow_error(WorkflowRuntimeError::Technical(stopped.into()));
        // Then

        assert!(matches!(error, WorkflowError::Technical(value) if value == stopped.into()));
    }
}
