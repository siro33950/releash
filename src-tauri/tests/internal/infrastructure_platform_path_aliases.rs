pub(crate) mod tests {

    use releash_lib::test_support::integration::platform::alias_name_for_profile;
    use releash_lib::test_support::integration::platform::child_env_overrides_from;
    use releash_lib::test_support::integration::platform::ensure_alias_wrapper;
    use releash_lib::test_support::integration::platform::prepare_child_env;
    use releash_lib::test_support::integration::platform::BuildProfile;
    use releash_lib::test_support::integration::platform::PathAlias;
    use releash_lib::test_support::integration::platform::PathAliases;
    use std::path::PathBuf;

    #[test]
    pub fn from_runtime_uses_build_profile_for_alias_name() {
        let aliases = PathAliases::from_runtime(PathBuf::from("/tmp/data")).unwrap();
        let releash = aliases.releash();
        assert_eq!(
            releash.name,
            alias_name_for_profile(BuildProfile::current())
        );
        assert_eq!(releash.data_dir, PathBuf::from("/tmp/data"));
    }

    #[cfg(unix)]
    #[test]
    pub fn child_env_overrides_from_sets_releash_data_dir_when_parent_unset() {
        let tmp = tempfile::TempDir::new().unwrap();
        let exe = tmp.path().join("releash-bin");
        std::fs::write(&exe, "").unwrap();
        let aliases = PathAliases::test_from_releash(PathAlias {
            name: "releash-test".to_string(),
            exe_path: exe,
            data_dir: tmp.path().join("data"),
        });
        let overrides = child_env_overrides_from(&aliases, Some("/bin"), None).unwrap();
        let data_dir_value = overrides
            .iter()
            .find_map(|(k, v)| (k == "RELEASH_DATA_DIR").then(|| v.clone()))
            .expect("RELEASH_DATA_DIR override missing");
        assert_eq!(
            data_dir_value,
            tmp.path().join("data").display().to_string()
        );
    }

    #[cfg(unix)]
    #[test]
    pub fn child_env_overrides_from_omits_releash_data_dir_when_parent_set() {
        // spec issues-1054 解決順序「明示指定 > alias 内包値」: 親プロセスに
        // RELEASH_DATA_DIR が明示されているときは alias 内包値で上書きしない。
        let tmp = tempfile::TempDir::new().unwrap();
        let exe = tmp.path().join("releash-bin");
        std::fs::write(&exe, "").unwrap();
        let aliases = PathAliases::test_from_releash(PathAlias {
            name: "releash-test".to_string(),
            exe_path: exe,
            data_dir: tmp.path().join("data"),
        });
        let overrides =
            child_env_overrides_from(&aliases, Some("/bin"), Some("/explicit/path")).unwrap();
        assert!(
            overrides.iter().all(|(k, _)| k != "RELEASH_DATA_DIR"),
            "RELEASH_DATA_DIR must not be in overrides when parent set it: {overrides:?}"
        );
        // PATH は依然として alias bin を先頭に積む。
        let path_value = overrides
            .iter()
            .find_map(|(k, v)| (k == "PATH").then(|| v.clone()))
            .expect("PATH override missing");
        assert!(path_value.starts_with(tmp.path().join("data").join("bin").to_str().unwrap()));
        assert!(path_value.ends_with(":/bin"));
    }

    #[cfg(unix)]
    #[test]
    pub fn child_env_overrides_from_treats_empty_parent_data_dir_as_unset() {
        let tmp = tempfile::TempDir::new().unwrap();
        let exe = tmp.path().join("releash-bin");
        std::fs::write(&exe, "").unwrap();
        let aliases = PathAliases::test_from_releash(PathAlias {
            name: "releash-test".to_string(),
            exe_path: exe,
            data_dir: tmp.path().join("data"),
        });
        let overrides = child_env_overrides_from(&aliases, None, Some("")).unwrap();
        assert!(
            overrides.iter().any(|(k, _)| k == "RELEASH_DATA_DIR"),
            "empty parent RELEASH_DATA_DIR should be treated as unset: {overrides:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    pub fn prepare_child_env_returns_path_and_data_dir_when_data_dir_provided() {
        // spec issues-1054「agent 子プロセスへの実行環境の伝搬」:
        // PTY / agent bridge が共有する env builder は alias bin の PATH と
        // RELEASH_DATA_DIR の両方を出力する。
        let tmp = tempfile::TempDir::new().unwrap();
        let env = prepare_child_env(Some(tmp.path().join("data"))).unwrap();
        assert!(env.iter().any(|(k, _)| k == "PATH"));
        // 親プロセスに RELEASH_DATA_DIR が無いテスト前提でのみ data_dir が積まれる。
        if std::env::var("RELEASH_DATA_DIR").is_err() {
            assert!(env.iter().any(|(k, _)| k == "RELEASH_DATA_DIR"));
        }
    }

    #[cfg(unix)]
    #[test]
    pub fn prepare_child_env_propagates_wrapper_failure() {
        // wrapper 作成不能（既存ファイルが bin dir 位置を占有）→ Err を返し、
        // 呼び出し側（PTY / bridge）で spawn を中止する。
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        // bin に通常ファイルを置くと create_dir_all が失敗する。
        std::fs::write(data_dir.join("bin"), "").unwrap();
        let err = prepare_child_env(Some(data_dir)).unwrap_err();
        assert!(
            err.to_string().contains("alias bin dir"),
            "expected wrapper bin dir error, got: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    pub fn ensure_alias_wrapper_exports_data_dir_only_when_unset() {
        let tmp = tempfile::TempDir::new().unwrap();
        let exe = tmp.path().join("releash-bin");
        std::fs::write(&exe, "").unwrap();
        let alias = PathAlias {
            name: "releash-test".to_string(),
            exe_path: exe,
            data_dir: tmp.path().join("data"),
        };
        let bin_dir = ensure_alias_wrapper(&alias).unwrap();
        let wrapper = bin_dir.join("releash-test");
        let script = std::fs::read_to_string(&wrapper).unwrap();
        // wrapper は `RELEASH_DATA_DIR` 未設定時のみ alias 内包値を export する
        // （spec 解決順序: 明示指定 > alias 内包値）。
        assert!(
            script.contains(r#"if [ -z "$RELEASH_DATA_DIR" ]; then"#),
            "wrapper must guard RELEASH_DATA_DIR export: {script}"
        );
        assert!(
            script.contains("export RELEASH_DATA_DIR="),
            "wrapper must export alias data_dir when unset: {script}"
        );
    }

    #[cfg(unix)]
    #[test]
    pub fn ensure_alias_wrapper_creates_executable() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::TempDir::new().unwrap();
        let exe = tmp.path().join("releash-bin");
        std::fs::write(&exe, "").unwrap();
        let alias = PathAlias {
            name: "releash-test".to_string(),
            exe_path: exe,
            data_dir: tmp.path().join("data"),
        };
        let bin_dir = ensure_alias_wrapper(&alias).unwrap();
        let wrapper = bin_dir.join("releash-test");
        let mode = std::fs::metadata(&wrapper).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "wrapper should be executable");
    }
}
