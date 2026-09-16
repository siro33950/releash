use super::*;

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
