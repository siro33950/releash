use super::*;

#[test]
fn test_data_dir解決_明示指定と環境変数と既定の優先順() {
    // Given
    let _lock = crate::test_support::TEST_ENV_LOCK.lock();
    struct RestoreEnv(Option<std::ffi::OsString>);
    impl Drop for RestoreEnv {
        fn drop(&mut self) {
            match &self.0 {
                Some(value) => std::env::set_var("RELEASH_DATA_DIR", value),
                None => std::env::remove_var("RELEASH_DATA_DIR"),
            }
        }
    }
    let _restore = RestoreEnv(std::env::var_os("RELEASH_DATA_DIR"));
    let explicit = PathBuf::from("explicit");
    let environment = PathBuf::from("environment");
    let default = default_data_dir_for_profile(BuildProfile::current())
        .ok_or_else(|| "OS data directory is unavailable".to_string());

    // When / Then
    std::env::set_var("RELEASH_DATA_DIR", &environment);
    assert_eq!(resolve_data_dir(Some(explicit.clone())), Ok(explicit));
    assert_eq!(resolve_data_dir(None), Ok(environment));
    std::env::set_var("RELEASH_DATA_DIR", "");
    assert_eq!(resolve_data_dir(None), default);
    std::env::remove_var("RELEASH_DATA_DIR");
    assert_eq!(resolve_data_dir(None), default);
}

#[test]
fn default_data_dir_name_distinguishes_dev_and_production() {
    assert_eq!(
        default_data_dir_name_for_profile(BuildProfile::Production),
        "com.releash.app"
    );
    assert_eq!(
        default_data_dir_name_for_profile(BuildProfile::Development),
        "com.releash.app.dev"
    );
}
