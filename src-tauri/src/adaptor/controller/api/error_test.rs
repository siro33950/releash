use super::*;
use crate::domain::workflow::WorkflowError;

#[test]
fn test_workflow停止_local_apiの既存エラー形式を保つ() {
    use crate::common::operation_context::OperationStopped;
    // Given
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        // When
        let error = ApiError::from(WorkflowError::Technical(stopped.into()));
        // Then
        assert_eq!(
            error.status.as_u16(),
            if stopped == OperationStopped::Expired {
                504
            } else {
                499
            }
        );
        assert_eq!(error.body.code, "workflow_error");
        assert_eq!(error.body.message, stopped.to_string());
    }
}
