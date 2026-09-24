use super::*;
use crate::domain::failure::{ClassifiedFailure, FailureKind};
use crate::domain::workflow::WorkflowError;

#[test]
fn test_session操作を経由してもworkflowの失敗分類を保持する() {
    // Given / When / Then
    for (error, expected) in [
        (
            WorkflowError::StorageUnavailable {
                message: "busy".into(),
                kind: crate::domain::failure::FailureKind::Temporary,
            },
            FailureKind::Temporary,
        ),
        (
            WorkflowError::StorageUnavailable {
                message: "repair".into(),
                kind: crate::domain::failure::FailureKind::StateRequired,
            },
            FailureKind::StateRequired,
        ),
        (
            WorkflowError::External("internal".into()),
            FailureKind::Internal,
        ),
        (
            WorkflowError::IncompatibleStoredEvent("version".into()),
            FailureKind::StateRequired,
        ),
        (
            WorkflowError::CorruptStoredState("corrupt".into()),
            FailureKind::Corrupt,
        ),
        (
            WorkflowError::Store(FailureKind::Expired),
            FailureKind::Expired,
        ),
        (
            WorkflowError::Conflict("revision".into()),
            FailureKind::RestartRequired,
        ),
    ] {
        assert_eq!(map_workflow_error(error).failure_kind(), expected);
    }
}

#[test]
fn test_失敗分類_全変種と委譲した理由を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    use crate::usecase::agent_session::AgentSessionLifecycleUsecaseError;
    // Given
    let cases = [
        (AgentSessionLifecycleUsecaseError::NotFound, F::Missing),
        (
            AgentSessionLifecycleUsecaseError::InvalidOperation,
            F::StateRequired,
        ),
        (
            AgentSessionLifecycleUsecaseError::Conflict(
                crate::domain::failure::FailureKind::RestartRequired,
            ),
            F::RestartRequired,
        ),
        (
            AgentSessionLifecycleUsecaseError::StorageUnavailable,
            F::Temporary,
        ),
        (
            AgentSessionLifecycleUsecaseError::LaunchUnavailable,
            F::StateRequired,
        ),
        (
            AgentSessionLifecycleUsecaseError::TerminalUnavailable,
            F::StateRequired,
        ),
        (AgentSessionLifecycleUsecaseError::Corrupt, F::Corrupt),
        (
            AgentSessionLifecycleUsecaseError::Store(F::Temporary),
            F::Temporary,
        ),
        (
            AgentSessionLifecycleUsecaseError::Store(F::RestartRequired),
            F::RestartRequired,
        ),
        (
            AgentSessionLifecycleUsecaseError::Store(F::StateRequired),
            F::StateRequired,
        ),
        (
            AgentSessionLifecycleUsecaseError::Store(F::InvalidInput),
            F::InvalidInput,
        ),
        (
            AgentSessionLifecycleUsecaseError::Store(F::Expired),
            F::Expired,
        ),
        (
            AgentSessionLifecycleUsecaseError::Store(F::Missing),
            F::Missing,
        ),
        (
            AgentSessionLifecycleUsecaseError::Store(F::AlreadyPresent),
            F::AlreadyPresent,
        ),
        (
            AgentSessionLifecycleUsecaseError::Store(F::Permission),
            F::Permission,
        ),
        (
            AgentSessionLifecycleUsecaseError::Store(F::Capacity),
            F::Capacity,
        ),
        (
            AgentSessionLifecycleUsecaseError::Store(F::Unsupported),
            F::Unsupported,
        ),
        (
            AgentSessionLifecycleUsecaseError::Store(F::Internal),
            F::Internal,
        ),
        (
            AgentSessionLifecycleUsecaseError::Store(F::Corrupt),
            F::Corrupt,
        ),
        (
            AgentSessionLifecycleUsecaseError::Store(F::Cancelled),
            F::Cancelled,
        ),
        (
            AgentSessionLifecycleUsecaseError::Store(F::Unknown),
            F::Unknown,
        ),
        (
            AgentSessionLifecycleUsecaseError::Store(F::OutsideRange),
            F::OutsideRange,
        ),
        (
            AgentSessionLifecycleUsecaseError::Store(F::AuthenticationRequired),
            F::AuthenticationRequired,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}

#[test]
fn test_session所有済みと保存競合を区別して伝播する() {
    use super::AgentSessionUsecaseError;
    use crate::domain::failure::{ClassifiedFailure, FailureKind};
    // Given / When / Then
    for (source, expected) in [
        (
            AgentSessionUsecaseError::Conflict,
            FailureKind::RestartRequired,
        ),
        (
            AgentSessionUsecaseError::ProviderSessionAlreadyOwned {
                agent_session_id: "owner".into(),
            },
            FailureKind::StateRequired,
        ),
    ] {
        assert_eq!(super::map_session_error(source).failure_kind(), expected);
    }
}

#[test]
fn test_workflowの結果不明を一時的なstore失敗に変えない() {
    use crate::domain::local_event::{SafeOperationFailure, SessionOperationFailureKind};
    // Given
    let source = SafeOperationFailure::new(
        SessionOperationFailureKind::OutcomeUnknown,
        crate::domain::failure::FailureKind::RestartRequired,
        "unknown",
        "id",
    );
    let error = WorkflowError::StorageUnavailable {
        message: source.to_string(),
        kind: source.failure_kind(),
    };
    // When / Then
    assert_eq!(
        map_workflow_error(error).failure_kind(),
        FailureKind::RestartRequired
    );
}
