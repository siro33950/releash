use super::*;

#[test]
fn test_子プロセス保存先_performance由来の環境変数を起動元daemonに揃える() {
    // Given
    let inherited = default_data_dir_for_profile(BuildProfile::Performance).unwrap();
    let known = known_alias_data_dirs().unwrap();
    for profile in [
        BuildProfile::Production,
        BuildProfile::Development,
        BuildProfile::Performance,
    ] {
        let data_dir = default_data_dir_for_profile(profile).unwrap();
        // When
        let resolved = resolve_session_data_dir_env(inherited.to_str(), &data_dir, &known);
        // Then
        assert_eq!(resolved, ResolvedDataDirEnv::Set(data_dir));
    }
}
