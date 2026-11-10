use super::*;

#[test]
fn test_data_dir解決_明示指定と環境変数と既定の優先順() {
    // Given
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
        .map(|path| DataDir {
            path,
            default: true,
        })
        .ok_or_else(|| "OS data directory is unavailable".to_string());

    // When / Then
    std::env::set_var("RELEASH_DATA_DIR", &environment);
    assert_eq!(
        resolve_data_dir(Some(explicit.clone())),
        Ok(DataDir {
            path: explicit,
            default: false
        })
    );
    assert_eq!(
        resolve_data_dir(None),
        Ok(DataDir {
            path: environment,
            default: false
        })
    );
    std::env::set_var("RELEASH_DATA_DIR", "");
    assert_eq!(resolve_data_dir(None), default);
    std::env::remove_var("RELEASH_DATA_DIR");
    assert_eq!(resolve_data_dir(None), default);
}

#[test]
fn test_明示保存先_既定と同じパスでも既定扱いにしない() {
    // Given
    let paths = [
        PathBuf::from("custom"),
        default_data_dir_for_profile(BuildProfile::current()).unwrap(),
    ];
    for path in paths {
        // When
        let actual = DataDir::explicit(path.clone());
        // Then
        assert_eq!(actual.path, path);
        assert!(!actual.default);
    }
}
