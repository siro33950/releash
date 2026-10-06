#[test]
fn test_workflow起動_停止分類をgateway境界で保持する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use crate::common::operation_context::OperationStopped;
    // Given
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        // When
        let error = super::workflow_runtime_error_to_workflow_error(
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::Technical(
                stopped.into(),
            ),
        );
        // Then
        assert_eq!(
            error.connect_code(),
            crate::domain::failure::TechnicalFailure::from(stopped).connect_code()
        );
    }
}
pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn workflow_name_resolution_diagnostics_remain_validation_errors() {
        let error =
            workflow_runtime_error_to_workflow_error(WorkflowRuntimeError::InvalidWorkflow(
                "workflow_diagnostics: WFS006: duplicate workflow name".to_string(),
            ));

        assert!(matches!(
            error,
            WorkflowError::Validation(message)
                if message.contains("WFS006") && message.contains("duplicate workflow name")
        ));
    }

    #[test]
    fn runtime_command_error_mapping_preserves_domain_variants() {
        assert!(matches!(
            workflow_runtime_error_to_workflow_error(WorkflowRuntimeError::ExecutionNotFound(
                "missing".to_string()
            )),
            WorkflowError::NotFound(message)
                if message == "No workflow execution found for session 'missing'"
        ));
        assert!(matches!(
            workflow_runtime_error_to_workflow_error(WorkflowRuntimeError::InvalidState(
                "terminal".to_string()
            )),
            WorkflowError::InvalidState(message) if message == "terminal"
        ));
        assert!(matches!(
            workflow_runtime_error_to_workflow_error(
                WorkflowRuntimeError::UnauthorizedApprovalTarget("wrong target".to_string())
            ),
            WorkflowError::UnauthorizedApprovalTarget(message) if message == "wrong target"
        ));
        assert!(matches!(
            workflow_runtime_error_to_workflow_error(WorkflowRuntimeError::ValidationError(
                "bad output".to_string()
            )),
            WorkflowError::Validation(message) if message == "bad output"
        ));
        assert!(matches!(
            workflow_runtime_error_to_workflow_error(WorkflowRuntimeError::SessionStore(
                "io".to_string()
            )),
            WorkflowError::External(message) if message == "io"
        ));
    }
    #[test]
    fn test_node事実追記_runtimeからconnectまで分類を保持する() {
        use crate::adaptor::presenter::connect::ConnectFailure;
        // Given
        for (failure, code) in [
            (
                crate::domain::local_event::CommitBatchError::QueueBusy.into(),
                connectrpc::ErrorCode::Unavailable,
            ),
            (
                crate::domain::local_event::CommitBatchError::Corrupt {
                    correlation_id: "id".into(),
                }
                .into(),
                connectrpc::ErrorCode::DataLoss,
            ),
            (
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    message: "timeout".into(),
                }
                .into(),
                connectrpc::ErrorCode::DeadlineExceeded,
            ),
            (
                crate::domain::local_event::CommitBatchError::AppendOutcomeUnknown.into(),
                connectrpc::ErrorCode::Aborted,
            ),
        ] {
            let failure: crate::domain::failure::StorageFailure = failure;
            let error = workflow_runtime_error_to_workflow_error(WorkflowRuntimeError::Store(
                failure.with_message("node append failed"),
            ));
            // Then
            assert_eq!(error.connect_code(), code);
            assert_eq!(
                crate::adaptor::presenter::connect::classified_error(error).code,
                code
            );
        }
    }
}
