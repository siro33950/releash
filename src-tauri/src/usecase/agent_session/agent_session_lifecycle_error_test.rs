use super::*;

use crate::domain::workflow::WorkflowError;

#[test]
fn test_session操作を経由してもworkflowの失敗分類を保持する() {
    // Given / When / Then
    for error in [
        WorkflowError::Store(
            crate::domain::failure::StorageFailure::from(
                crate::domain::local_event::CommitBatchError::QueueBusy,
            )
            .with_message("busy"),
        ),
        WorkflowError::Store(
            crate::domain::failure::StorageFailure::from(
                crate::domain::local_event::CommitBatchError::PayloadConflict,
            )
            .with_message("repair"),
        ),
        WorkflowError::External("internal".into()),
        WorkflowError::IncompatibleStoredEvent("version".into()),
        WorkflowError::CorruptStoredState("corrupt".into()),
        WorkflowError::Store(
            (crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                message: "failure".into(),
            })
            .into(),
        ),
        WorkflowError::Conflict("revision".into()),
    ] {
        let expected = match &error {
            WorkflowError::Store(failure) => {
                AgentSessionLifecycleUsecaseError::Store(failure.clone())
            }
            WorkflowError::CorruptStoredState(_) => AgentSessionLifecycleUsecaseError::Corrupt,
            WorkflowError::Conflict(_) => {
                AgentSessionLifecycleUsecaseError::Conflict(error.clone().into())
            }
            _ => AgentSessionLifecycleUsecaseError::Workflow(error.clone()),
        };
        assert_eq!(map_workflow_error(error), expected);
    }
}

#[test]
fn test_session所有済みと保存競合を区別して伝播する() {
    use super::AgentSessionUsecaseError;

    // Given / When / Then
    for source in [
        AgentSessionUsecaseError::Conflict,
        AgentSessionUsecaseError::ProviderSessionAlreadyOwned {
            agent_session_id: "owner".into(),
        },
    ] {
        let expected = source.clone().into();
        assert_eq!(
            super::map_session_error(source),
            super::AgentSessionLifecycleUsecaseError::Conflict(expected)
        );
    }
}

#[test]
fn test_workflowの結果不明を一時的なstore失敗に変えない() {
    let error = WorkflowError::Store(
        crate::domain::local_event::CommitBatchError::AppendOutcomeUnknown.into(),
    );
    // When / Then
    assert_eq!(
        map_workflow_error(error),
        AgentSessionLifecycleUsecaseError::Store(
            crate::domain::local_event::CommitBatchError::AppendOutcomeUnknown.into()
        )
    );
}
