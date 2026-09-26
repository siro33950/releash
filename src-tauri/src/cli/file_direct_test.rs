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

#[test]
fn test_store失敗_cliの既存文面と文脈付き文面を保つ() {
    use crate::domain::local_event::LocalEventQueryError;
    for (source, expected) in [
        (LocalEventQueryError::QueryBusy, "Store failure: Temporary"),
        (
            LocalEventQueryError::Corrupt {
                correlation_id: "id".into(),
            },
            "Store failure: Corrupt",
        ),
        (
            LocalEventQueryError::IncompatibleStoredEvent {
                correlation_id: "id".into(),
            },
            "Store failure: StateRequired",
        ),
    ] {
        let error = workflow_error_to_cli_error(WorkflowError::Store(source.into()));
        assert!(matches!(error, CliError::Other(message) if message == expected));
    }
    let error = workflow_error_to_cli_error(WorkflowError::storage(
        LocalEventQueryError::QueryBusy,
        "read failed",
    ));
    assert!(matches!(error, CliError::Other(message) if message == "read failed"));
}
