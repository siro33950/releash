use super::*;

#[test]
fn test_失敗分類_全変種と委譲した理由を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (AgentSessionQueryError::InvalidRequest, F::InvalidInput),
        (AgentSessionQueryError::Unavailable, F::Temporary),
        (AgentSessionQueryError::Corrupt, F::Corrupt),
        (AgentSessionQueryError::Store(F::Temporary), F::Temporary),
        (
            AgentSessionQueryError::Store(F::RestartRequired),
            F::RestartRequired,
        ),
        (
            AgentSessionQueryError::Store(F::StateRequired),
            F::StateRequired,
        ),
        (
            AgentSessionQueryError::Store(F::InvalidInput),
            F::InvalidInput,
        ),
        (AgentSessionQueryError::Store(F::Expired), F::Expired),
        (AgentSessionQueryError::Store(F::Missing), F::Missing),
        (
            AgentSessionQueryError::Store(F::AlreadyPresent),
            F::AlreadyPresent,
        ),
        (AgentSessionQueryError::Store(F::Permission), F::Permission),
        (AgentSessionQueryError::Store(F::Capacity), F::Capacity),
        (
            AgentSessionQueryError::Store(F::Unsupported),
            F::Unsupported,
        ),
        (AgentSessionQueryError::Store(F::Internal), F::Internal),
        (AgentSessionQueryError::Store(F::Corrupt), F::Corrupt),
        (AgentSessionQueryError::Store(F::Cancelled), F::Cancelled),
        (AgentSessionQueryError::Store(F::Unknown), F::Unknown),
        (
            AgentSessionQueryError::Store(F::OutsideRange),
            F::OutsideRange,
        ),
        (
            AgentSessionQueryError::Store(F::AuthenticationRequired),
            F::AuthenticationRequired,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
