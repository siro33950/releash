use super::*;

#[test]
fn test_workflow停止_local_apiの既存エラー形式を保つ() {
    use crate::domain::operation_context::OperationStopped;
    // Given
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        // When
        let error = ApiError::from(WorkflowError::Stopped(stopped));
        // Then
        assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.body.code, "workflow_error");
        assert_eq!(error.body.message, stopped.to_string());
    }
}
