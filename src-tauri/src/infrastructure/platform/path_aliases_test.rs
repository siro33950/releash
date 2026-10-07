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
