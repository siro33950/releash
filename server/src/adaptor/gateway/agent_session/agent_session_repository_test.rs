use super::*;
#[test]
fn test_agent_session_repository_commit_errorの理由別分類を保持する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use connectrpc::ErrorCode;
    // Given / When / Then
    for (error, expected) in [
        (
            CommitBatchError::CapacityExceeded,
            ErrorCode::ResourceExhausted,
        ),
        (
            CommitBatchError::SequenceExhausted,
            ErrorCode::ResourceExhausted,
        ),
        (
            CommitBatchError::PayloadConflict,
            ErrorCode::FailedPrecondition,
        ),
        (
            CommitBatchError::Corrupt {
                correlation_id: "corrupt-commit".into(),
            },
            ErrorCode::DataLoss,
        ),
    ] {
        assert_eq!(map_commit_batch_error(error).connect_code(), expected);
    }
}

#[test]
fn test_所有照会_版の競合と所有済みを業務の失敗として保持する() {
    use crate::domain::agent_session::AgentSessionHistoryGatewayError;

    // Given
    for (error, expected) in [
        (
            AgentSessionRepositoryError::Conflict,
            AgentSessionHistoryGatewayError::Conflict,
        ),
        (
            AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
                agent_session_id: "owner".into(),
            },
            AgentSessionHistoryGatewayError::ProviderSessionAlreadyOwned {
                agent_session_id: "owner".into(),
            },
        ),
    ] {
        // When / Then
        assert_eq!(map_ownership_error(error), expected);
    }
}

#[test]
fn test_所有照会_所有済みと競合と一時的失敗の分類を保持する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use connectrpc::ErrorCode;
    // Given
    for (error, expected) in [
        (
            AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
                agent_session_id: "owner".into(),
            },
            ErrorCode::FailedPrecondition,
        ),
        (AgentSessionRepositoryError::Conflict, ErrorCode::Aborted),
        (
            AgentSessionRepositoryError::Unavailable,
            ErrorCode::Unavailable,
        ),
        (AgentSessionRepositoryError::Corrupt, ErrorCode::DataLoss),
        (
            AgentSessionRepositoryError::InvalidRequest,
            ErrorCode::InvalidArgument,
        ),
        (
            AgentSessionRepositoryError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    message: "failure".into(),
                })
                .into(),
            ),
            ErrorCode::DeadlineExceeded,
        ),
    ] {
        // When / Then
        assert_eq!(map_ownership_error(error).connect_code(), expected);
    }
}
