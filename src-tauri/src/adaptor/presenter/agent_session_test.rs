use super::*;
use crate::usecase::agent_session::{
    ProviderAvailabilityItemDto, ProviderAvailabilitySnapshotDto, ProviderUnavailableReasonDto,
};
use crate::usecase::provider_dto::AgentSessionProviderDto;
use crate::usecase::provider_lifecycle::{
    ProviderHookHealthReasonDto, ProviderHookHealthWarningDto,
};

#[test]
fn test_provider利用可否_四種類の理由を転送文字列へ写す() {
    // Given
    let cases = [
        (ProviderUnavailableReasonDto::NotFound, "not_found"),
        (
            ProviderUnavailableReasonDto::NotExecutable,
            "not_executable",
        ),
        (
            ProviderUnavailableReasonDto::SearchPathUnavailable,
            "search_path_unavailable",
        ),
        (ProviderUnavailableReasonDto::ProbeFailed, "probe_failed"),
    ];

    // When
    for (reason, expected) in cases {
        let response =
            ProviderAvailabilitySnapshotResponse::from(ProviderAvailabilitySnapshotDto {
                providers: vec![ProviderAvailabilityItemDto {
                    provider: AgentSessionProviderDto::Claude,
                    display_name: "Claude".into(),
                    default_executable: "claude".into(),
                    configured_executable: None,
                    effective_executable: "claude".into(),
                    available: false,
                    resolved_executable: None,
                    unavailable_reason: Some(reason),
                }],
            });
        // Then
        assert_eq!(
            response.providers[0].unavailable_reason.as_deref(),
            Some(expected)
        );
    }
}

#[test]
fn test_provider警告_四種類の理由を転送文字列へ写す() {
    // Given
    let cases = [
        (
            ProviderHookHealthReasonDto::SessionStartDeadlineExceeded,
            "session_start_deadline_exceeded",
        ),
        (
            ProviderHookHealthReasonDto::CodexHookDeliveryUnconfirmed,
            "codex_hook_delivery_unconfirmed",
        ),
        (
            ProviderHookHealthReasonDto::ProviderHookConfigurationRejected,
            "provider_hook_configuration_rejected",
        ),
        (
            ProviderHookHealthReasonDto::LocalApiUnavailable,
            "local_api_unavailable",
        ),
    ];

    // When
    for (reason, expected) in cases {
        let response = ProviderHookHealthWarningResponse::from(ProviderHookHealthWarningDto {
            provider: AgentSessionProviderDto::Codex,
            launch_id: "launch".into(),
            reason,
        });
        // Then
        assert_eq!(response.reason, expected);
    }
}
