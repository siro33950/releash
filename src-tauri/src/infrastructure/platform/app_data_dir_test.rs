use super::*;

#[test]
fn test_データディレクトリ_検証用保存先はperformanceビルドだけで使う() {
    // Given
    const PROBE: &str = "RELEASH_DATA_DIR_TEST_CHILD";
    if std::env::var_os(PROBE).is_some() {
        // When / Then
        let expected = if cfg!(feature = "performance") {
            PathBuf::from("/test-performance-data")
        } else {
            default_data_dir_for_profile(BuildProfile::application()).unwrap()
        };
        assert_eq!(resolve_data_dir().unwrap(), expected);
        return;
    }
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("infrastructure::platform::app_data_dir::app_data_dir_tests::test_データディレクトリ_検証用保存先はperformanceビルドだけで使う")
        .arg("--exact")
        .env(PROBE, "1")
        .env("RELEASH_PERFORMANCE_DATA_DIR", "/test-performance-data")
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
}

#[test]
fn test_データディレクトリ_全ビルドで既存desktopの保存先を使う() {
    // Given
    let base = dirs::data_dir().unwrap();
    // When / Then
    for (profile, identifier) in [
        (BuildProfile::Production, "com.releash.app"),
        (BuildProfile::Development, "com.releash.app.dev"),
        (BuildProfile::Performance, "com.releash.app.performance"),
    ] {
        assert_eq!(
            resolve_for_profile(profile, None).unwrap(),
            base.join(identifier)
        );
        assert_eq!(
            resolve_for_profile(profile, Some(String::new())).unwrap(),
            base.join(identifier)
        );
        assert_eq!(
            resolve_for_profile(profile, Some("/explicit".into())).unwrap(),
            PathBuf::from("/explicit")
        );
    }
}
