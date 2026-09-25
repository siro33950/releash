use super::*;

#[test]
fn test_workflow境界_停止分類を保持する() {
    use crate::domain::failure::ClassifiedFailure;
    use crate::domain::operation_context::OperationStopped;
    // Given
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        // When
        let error = runtime_error_to_workflow_error(WorkflowRuntimeError::Stopped(stopped));
        // Then
        assert_eq!(error.failure_kind(), stopped.failure_kind());
        assert!(matches!(error, WorkflowError::Stopped(value) if value == stopped));
    }
}
