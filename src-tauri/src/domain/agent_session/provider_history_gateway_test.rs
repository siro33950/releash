use super::*;

#[test]
fn test_失敗分類_agent_session_history_gateway_error_理由に対応する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (
            AgentSessionHistoryGatewayError::InvalidRequest,
            F::InvalidInput,
        ),
        (AgentSessionHistoryGatewayError::Unavailable, F::Temporary),
        (AgentSessionHistoryGatewayError::Corrupt, F::Corrupt),
        (
            AgentSessionHistoryGatewayError::Store(F::Expired),
            F::Expired,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
