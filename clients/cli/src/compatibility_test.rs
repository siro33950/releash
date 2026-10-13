use super::Compatibility;

#[test]
fn test_互換判定_protocolの大小で古い側を返す() {
    assert_eq!(Compatibility::assess(1, 1), Compatibility::Compatible);
    assert_eq!(Compatibility::assess(2, 1), Compatibility::ServerOlder);
    assert_eq!(Compatibility::assess(1, 2), Compatibility::ClientOlder);
}

#[test]
fn test_版比較_semverの順序と不正な版を扱う() {
    // Given / When / Then
    assert_eq!(
        Compatibility::assess_release("1.10.0", "1.9.0").unwrap(),
        Compatibility::ServerOlder
    );
    assert_eq!(
        Compatibility::assess_release("1.0.0", "1.0.1").unwrap(),
        Compatibility::ClientOlder
    );
    assert_eq!(
        Compatibility::assess_release("1.0.0", "1.0.0").unwrap(),
        Compatibility::Compatible
    );
    assert_eq!(
        Compatibility::assess_release("1.0.0", "1.0.0-rc.1").unwrap(),
        Compatibility::ServerOlder
    );
    assert!(Compatibility::assess_release("invalid", "1.0.0").is_err());
}
