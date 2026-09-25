use super::*;

#[test]
fn test_workflow停止_cliの既存エラー形式を保つ() {
    use crate::common::operation_context::OperationStopped;
    // Given
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        // When
        let error = workflow_error_to_cli_error(WorkflowError::Technical(stopped.into()));
        // Then
        assert!(matches!(error, CliError::Other(message) if message == stopped.to_string()));
    }
}
