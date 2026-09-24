use super::*;

#[test]
fn test_失敗分類_agent_session_repository_error_理由に対応する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (AgentSessionRepositoryError::Conflict, F::RestartRequired),
        (
            AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
                agent_session_id: "session".into(),
            },
            F::StateRequired,
        ),
        (AgentSessionRepositoryError::InvalidRequest, F::InvalidInput),
        (AgentSessionRepositoryError::Corrupt, F::Corrupt),
        (AgentSessionRepositoryError::Unavailable, F::Temporary),
        (AgentSessionRepositoryError::Store(F::Expired), F::Expired),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
