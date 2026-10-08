#[test]
fn test_provider起動ファイル_不正pathを入力失敗として返す() {
    // Given / When / Then
    assert_eq!(
        super::map_files_error(
            crate::infrastructure::provider_lifecycle::ProviderLaunchFilesError::InvalidPath
        ),
        crate::domain::agent_session::ProviderAgentLaunchGatewayError::InvalidInput
    );
}
