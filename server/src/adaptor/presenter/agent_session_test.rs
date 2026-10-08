use super::*;
use crate::domain::provider_lifecycle::ProviderLifecycleUnavailableReason;
use crate::usecase::agent_session::{
    ProviderAvailabilityItemDto, ProviderAvailabilitySnapshotDto, ProviderUnavailableReasonDto,
};
use crate::usecase::provider_dto::AgentSessionProviderDto;
use crate::usecase::provider_lifecycle::ProviderHookHealthWarning;

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
                    configuration_revision: 0,
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
            ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded,
            "session_start_deadline_exceeded",
        ),
        (
            ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed,
            "codex_hook_delivery_unconfirmed",
        ),
        (
            ProviderLifecycleUnavailableReason::ProviderHookConfigurationRejected,
            "provider_hook_configuration_rejected",
        ),
        (
            ProviderLifecycleUnavailableReason::LocalApiUnavailable,
            "local_api_unavailable",
        ),
    ];

    // When
    let responses: Vec<_> = cases
        .iter()
        .map(|(reason, _)| {
            ProviderHookHealthWarningResponse::from(ProviderHookHealthWarning {
                provider: ProviderKind::Codex,
                launch_id: "launch".into(),
                reason: *reason,
            })
            .reason
        })
        .collect();
    // Then
    assert_eq!(responses, cases.map(|(_, expected)| expected));
}

#[test]
fn test_provider警告_購読用出力にproviderと理由を写す() {
    // Given
    let warning = ProviderHookHealthWarning {
        provider: ProviderKind::Codex,
        launch_id: "launch".into(),
        reason: ProviderLifecycleUnavailableReason::LocalApiUnavailable,
    };
    // When
    let output =
        crate::adaptor::presenter::agent_session::ProviderHookHealthWarningResponse::from(warning);
    // Then
    assert_eq!(
        output.provider,
        crate::adaptor::presenter::agent_session::ProviderHookHealthProviderResponse::Codex
    );
    assert_eq!(output.launch_id, "launch");
    assert_eq!(output.reason, "local_api_unavailable");
}
