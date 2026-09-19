#![cfg(all(debug_assertions, feature = "desktop", feature = "performance"))]

#[test]
fn test_cli設置_probeは実設置境界を捕捉して管理者認証と書込みを止める() {
    if std::env::var_os("RELEASH_TEST_CLI_INSTALL_ATTEMPT").is_some() {
        assert_eq!(
            releash_lib::client_api_acceptance::probe_cli_installation().unwrap_err(),
            "CLI installation intercepted by acceptance probe"
        );
        return;
    }
    // Given
    let directory = tempfile::tempdir().unwrap();
    let probe = directory.path().join("attempt");
    // When
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("test_cli設置_probeは実設置境界を捕捉して管理者認証と書込みを止める")
        .env("RELEASH_TEST_CLI_INSTALL_ATTEMPT", &probe)
        .status()
        .unwrap();
    // Then
    assert!(status.success());
    assert_eq!(
        std::fs::read_to_string(probe).unwrap(),
        "CLI installation attempted"
    );
}
