use super::*;

#[test]
fn test_workflow境界_停止分類を保持する() {
    use crate::common::operation_context::OperationStopped;
    use crate::domain::failure::ClassifiedFailure;
    // Given
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        // When
        let error =
            runtime_error_to_workflow_error(WorkflowRuntimeError::Technical(stopped.into()));
        // Then
        assert_eq!(error.failure_kind(), stopped.failure_kind());
        assert!(matches!(error, WorkflowError::Technical(value) if value == stopped.into()));
    }
}
