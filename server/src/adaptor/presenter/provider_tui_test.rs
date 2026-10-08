use super::*;
use crate::domain::failure::{StorageFailure, TechnicalFailure, TechnicalFailureNature};

#[test]
fn test_store失敗_内部の詳細を表示せず文字列と転送コードを保持する() {
    // Given
    for (nature, code) in [
        (
            TechnicalFailureNature::Transient,
            connectrpc::ErrorCode::Unavailable,
        ),
        (
            TechnicalFailureNature::TimedOut,
            connectrpc::ErrorCode::DeadlineExceeded,
        ),
        (
            TechnicalFailureNature::Cancelled,
            connectrpc::ErrorCode::Canceled,
        ),
        (
            TechnicalFailureNature::Other,
            connectrpc::ErrorCode::Internal,
        ),
    ] {
        let failure = StorageFailure::from(TechnicalFailure {
            nature,
            message: "private source details".into(),
        })
        .with_message("private context details");
        // When
        let errors = [
            (
                launch_error(
                    AgentSessionLaunchUsecaseError::Store(failure.clone()),
                    AgentSessionLaunchOperation::Start,
                ),
                "Releash could not access saved AgentSession data. Try again.",
            ),
            (
                lifecycle_error(AgentSessionLifecycleUsecaseError::Store(failure)),
                "Releash could not access saved AgentSession data. Try again.",
            ),
        ];
        // Then
        for (error, message) in errors {
            assert_eq!(error.connect_code(), code);
            assert_eq!(
                serde_json::to_value(&error).unwrap(),
                serde_json::json!(message)
            );
        }
    }
}

#[test]
fn test_providerとterminal失敗_元のメッセージと表示文言をそれぞれ保持する() {
    use crate::domain::agent_session::{
        ProviderAgentLaunchGatewayError, ProviderAgentTerminalGatewayError,
    };
    for (nature, code) in [
        (
            TechnicalFailureNature::Transient,
            connectrpc::ErrorCode::Unavailable,
        ),
        (
            TechnicalFailureNature::TimedOut,
            connectrpc::ErrorCode::DeadlineExceeded,
        ),
        (
            TechnicalFailureNature::Cancelled,
            connectrpc::ErrorCode::Canceled,
        ),
        (
            TechnicalFailureNature::Other,
            connectrpc::ErrorCode::Internal,
        ),
    ] {
        // Given
        let failure = TechnicalFailure {
            nature,
            message: "original provider failure".into(),
        };
        let cases = [
            (launch_error(AgentSessionLaunchUsecaseError::Launch(ProviderAgentLaunchGatewayError::Technical(failure.clone())), AgentSessionLaunchOperation::Start), "Releash could not complete the Provider operation for this AgentSession. Try again."),
            (lifecycle_error(AgentSessionLifecycleUsecaseError::Launch(ProviderAgentLaunchGatewayError::Technical(failure.clone()))), "Releash could not complete the Provider operation for this AgentSession. Try again."),
            (launch_error(AgentSessionLaunchUsecaseError::Terminal(ProviderAgentTerminalGatewayError::Technical(failure.clone())), AgentSessionLaunchOperation::Start), "Releash could not complete the Terminal operation for this AgentSession. Try again."),
            (lifecycle_error(AgentSessionLifecycleUsecaseError::Terminal(ProviderAgentTerminalGatewayError::Technical(failure.clone()))), "Releash could not complete the Terminal operation for this AgentSession. Try again."),
            (launch_error(AgentSessionLaunchUsecaseError::Terminal(ProviderAgentTerminalGatewayError::Technical(failure.clone())), AgentSessionLaunchOperation::Start), "Releash could not complete the Terminal operation for this AgentSession. Try again."),
        ];
        for (error, display) in cases {
            // When
            let wire: crate::adaptor::presenter::client::CommandFailure = error.clone().into();
            let connect = crate::adaptor::presenter::connect::command_error(wire);
            // Then
            assert_eq!(connect.code, code);
            assert_eq!(
                connect.message.as_deref(),
                Some("original provider failure")
            );
            assert_eq!(serde_json::to_value(error).unwrap()["message"], display);
            assert_eq!(connect.details.len(), 1);
        }
    }
}

#[test]
fn test_agent_session_resume_provider利用不可は既存の文言とstatusを返す() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    // Given / When
    let error = super::lifecycle_error(
        crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::ProviderUnavailable,
    );
    // Then
    assert_eq!(
        error.to_string(),
        "Releash could not complete the Provider operation for this AgentSession. Try again."
    );
    assert_eq!(
        error.connect_code(),
        connectrpc::ErrorCode::FailedPrecondition
    );
    assert_eq!(error.cause(), None);
}
