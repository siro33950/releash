use super::*;

#[test]
fn test_agent_session_terminal_spawn_error_記録用kindとpayloadを表示する() {
    let cases = [
        (
            ProviderAgentTerminalGatewayError::OwnerConflict,
            "kind=owner_conflict",
        ),
        (
            ProviderAgentTerminalGatewayError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "openpty failed".to_string(),
                },
            ),
            "openpty failed",
        ),
        (
            ProviderAgentTerminalGatewayError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "checkpoint failed".to_string(),
                },
            ),
            "checkpoint failed",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}
