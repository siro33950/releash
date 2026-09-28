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
