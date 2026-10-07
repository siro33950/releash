use super::*;

#[test]
fn alias_name_for_profile_returns_releash_for_production() {
    assert_eq!(alias_name_for_profile(BuildProfile::Production), "releash");
}

#[test]
fn alias_name_for_profile_returns_releash_dev_for_development() {
    assert_eq!(
        alias_name_for_profile(BuildProfile::Development),
        "releash-dev"
    );
}

#[test]
fn compose_path_with_alias_bin_prepends_to_head_when_path_set() {
    // alias bin dir は既存 PATH の**先頭**にあること。末尾だと既存 PATH 前方に
    // `releash` / `releash-dev` を含む別ディレクトリがあると wrapper が解決されず
    // alias と実行 binary の一意対応 (spec [01]) が崩れる。bin dir に置く実体は
    // wrapper のみで、システムコマンドの shadow 経路は生じない。
    let bin_dir = PathBuf::from("/tmp/my-data/bin");
    let composed = compose_path_with_alias_bin(Some("/system-bin:/other-bin"), bin_dir.as_path());
    assert_eq!(composed, "/tmp/my-data/bin:/system-bin:/other-bin");
}

#[test]
fn compose_path_with_alias_bin_falls_back_to_bin_only_when_path_unset() {
    let bin_dir = PathBuf::from("/tmp/my-data/bin");
    assert_eq!(
        compose_path_with_alias_bin(None, bin_dir.as_path()),
        "/tmp/my-data/bin"
    );
    assert_eq!(
        compose_path_with_alias_bin(Some(""), bin_dir.as_path()),
        "/tmp/my-data/bin"
    );
}

#[cfg(unix)]
#[test]
fn prepare_child_env_returns_empty_when_data_dir_none() {
    // app_data_dir() 解決失敗時の経路: 既存挙動 (silent skip) を温存。
    let env = prepare_child_env(None).unwrap();
    assert!(
        env.is_empty(),
        "no overrides expected when data_dir is None"
    );
}

/// 親 env が None なら自分の data_dir を設置する。
#[test]
fn resolve_session_data_dir_env_sets_self_when_parent_env_absent() {
    let self_dir = PathBuf::from("/home/u/.local/share/com.releash.app.dev");
    let known = vec![
        PathBuf::from("/home/u/.local/share/com.releash.app"),
        self_dir.clone(),
    ];
    let result = resolve_session_data_dir_env(None, &self_dir, &known);
    assert_eq!(result, ResolvedDataDirEnv::Set(self_dir));
}

/// 親 env が空文字なら自分の data_dir を設置する。
#[test]
fn resolve_session_data_dir_env_sets_self_when_parent_env_empty() {
    let self_dir = PathBuf::from("/home/u/.local/share/com.releash.app.dev");
    let known = vec![
        PathBuf::from("/home/u/.local/share/com.releash.app"),
        self_dir.clone(),
    ];
    let result = resolve_session_data_dir_env(Some(""), &self_dir, &known);
    assert_eq!(result, ResolvedDataDirEnv::Set(self_dir));
}

/// 親 env が「別 alias の data_dir」を指している場合は、別 Releash binary 由来の
/// inherit と判定して自分の alias data_dir で上書きする (バグ修正の主シナリオ)。
#[test]
fn resolve_session_data_dir_env_overrides_when_parent_matches_other_alias() {
    let self_dir = PathBuf::from("/home/u/.local/share/com.releash.app.dev");
    let other_alias = PathBuf::from("/home/u/.local/share/com.releash.app");
    let known = vec![other_alias.clone(), self_dir.clone()];
    let result =
        resolve_session_data_dir_env(Some(other_alias.to_str().unwrap()), &self_dir, &known);
    assert_eq!(result, ResolvedDataDirEnv::Set(self_dir));
}

/// 親 env が「自分と同じ alias の data_dir」を指している場合も Set(self) を返す
/// (同種 inherit 経路で値が一致しているケースの整合性確認、結果は no-op 同等)。
#[test]
fn resolve_session_data_dir_env_overrides_when_parent_matches_own_alias() {
    let self_dir = PathBuf::from("/home/u/.local/share/com.releash.app.dev");
    let known = vec![
        PathBuf::from("/home/u/.local/share/com.releash.app"),
        self_dir.clone(),
    ];
    let result = resolve_session_data_dir_env(Some(self_dir.to_str().unwrap()), &self_dir, &known);
    assert_eq!(result, ResolvedDataDirEnv::Set(self_dir));
}

/// 親 env が「既知 alias data_dir のいずれにも一致しない任意パス」を指している場合は
/// ユーザーの真の明示指定として尊重 (Keep) する。
#[test]
fn resolve_session_data_dir_env_keeps_parent_when_arbitrary_path() {
    let self_dir = PathBuf::from("/home/u/.local/share/com.releash.app.dev");
    let known = vec![
        PathBuf::from("/home/u/.local/share/com.releash.app"),
        self_dir.clone(),
    ];
    let result = resolve_session_data_dir_env(Some("/tmp/custom-releash"), &self_dir, &known);
    assert_eq!(result, ResolvedDataDirEnv::Keep);
}
