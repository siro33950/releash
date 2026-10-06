use super::shared_test_helpers::FailingSearchPathSource;
use super::*;
use crate::domain::agent_session::ProviderExecutableProbeGateway;
use crate::infrastructure::process::search_path::LoginShellPathError;
use std::sync::Arc;

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn test_provider_availability_refresh_shell時間切れの性質を保持する() {
    // Given
    let gateway = LocalProviderExecutableProbeGateway::with_search_path_source(
        None,
        Arc::new(FailingSearchPathSource(LoginShellPathError::Timeout)),
    );
    // When
    let error = gateway.refresh_search_path().unwrap_err();
    // Then
    assert_eq!(
        error,
        crate::domain::agent_session::ProviderExecutableProbeGatewayError::Technical(
            crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                message: "Timeout".into(),
            }
        )
    );
}
