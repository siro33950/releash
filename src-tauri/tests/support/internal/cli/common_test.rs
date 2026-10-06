pub(crate) mod tests {
    use super::super::*;
    use tempfile::TempDir;

    /// [05] 観測経路境界 (5-1 修正): data_dir が存在しない場合は `NotFound` として
    /// 扱い、「executions 0 件」と「向き先がそもそも無い」を区別する。
    #[test]
    pub fn ensure_existing_data_dir_returns_not_found_for_missing_path() {
        let missing = std::path::PathBuf::from("/non/existent/releash-data-dir-test-path");
        let err = ensure_existing_data_dir(&missing).expect_err("missing data_dir must error");
        let CliError::NotFound(msg) = &err else {
            panic!("expected CliError::NotFound for missing data_dir, got: {err:?}");
        };
        assert!(
            msg.contains(&missing.display().to_string()),
            "error message must contain the path, got: {msg}"
        );
    }

    /// [05] 観測経路境界 (5-1 修正): data_dir が存在する場合は Ok を返す。
    #[test]
    pub fn ensure_existing_data_dir_returns_ok_for_existing_path() {
        let tmp = TempDir::new().unwrap();
        ensure_existing_data_dir(tmp.path()).expect("existing data_dir must succeed");
    }

    /// spec [01] 解決順序「明示指定 > alias 内包値」: 明示指定が無い場合は
    /// `PathAliases` から導いた alias 内包の data_dir を返す（既定値は bundle
    /// identifier suffix を持つ）。
    #[test]
    pub fn resolve_data_dir_falls_back_to_alias_data_dir_when_env_unset() {
        if dirs::data_dir().is_none() {
            return;
        }
        let resolved = resolve_data_dir_from_env(None).unwrap();
        let expected_suffix =
            crate::infrastructure::platform::path_aliases::default_data_dir_name_for_profile(
                crate::infrastructure::platform::path_aliases::BuildProfile::current(),
            );
        assert!(
            resolved.ends_with(expected_suffix),
            "expected suffix {expected_suffix}, got {}",
            resolved.display()
        );
    }

    /// spec [01]: 明示指定が空文字列のときは未設定扱いとし alias 内包値に
    /// フォールバックする（空文字列を data_dir として採用すると以降の
    /// 観測経路で「executions 0 件」と紛れるため）。
    #[test]
    pub fn resolve_data_dir_treats_empty_env_as_unset() {
        if dirs::data_dir().is_none() {
            return;
        }
        let resolved = resolve_data_dir_from_env(Some(String::new())).unwrap();
        let expected_suffix =
            crate::infrastructure::platform::path_aliases::default_data_dir_name_for_profile(
                crate::infrastructure::platform::path_aliases::BuildProfile::current(),
            );
        assert!(
            resolved.ends_with(expected_suffix),
            "empty env should fall through to alias data_dir, got {}",
            resolved.display()
        );
    }
}
